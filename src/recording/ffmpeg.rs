use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::ResolvedFormat;

/// How many trailing stderr lines to keep for diagnostics.
const STDERR_TAIL_LINES: usize = 40;

pub struct FfmpegProcess {
    child: Child,
    pub output_path: PathBuf,
    stderr_tail: Arc<Mutex<std::collections::VecDeque<String>>>,
}

pub struct FfmpegBuilder {
    input_url: String,
    output_path: PathBuf,
    transcode: bool,
    format: Option<ResolvedFormat>,
    from_start: bool,
}

impl FfmpegBuilder {
    pub fn new(input_url: String, output_path: PathBuf) -> Self {
        Self {
            input_url,
            output_path,
            transcode: false,
            format: None,
            from_start: false,
        }
    }

    pub fn transcode(mut self, enabled: bool) -> Self {
        self.transcode = enabled;
        self
    }

    pub fn format(mut self, format: ResolvedFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Start pulling from the first segment in the HLS manifest instead of
    /// the live edge. For Twitch this lands ~5 minutes back (the DVR window);
    /// the closest the protocol gets to "from beginning".
    pub fn from_start(mut self, enabled: bool) -> Self {
        self.from_start = enabled;
        self
    }

    pub fn build(self) -> Result<FfmpegProcess> {
        if let Some(parent) = self.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-y", "-hide_banner", "-loglevel", "warning"]);

        if self.from_start {
            // -99999 lands on the first segment in the current HLS
            // playlist (negative is clamped to 0 after `n_segments +
            // live_start_index`). Plain `0` would target absolute
            // segment index 0, which is never present in a live
            // playlist with rolling EXT-X-MEDIA-SEQUENCE — ffmpeg then
            // 404s every segment and exits.
            cmd.args(["-live_start_index", "-99999"]);
        }

        cmd.args(["-i", &self.input_url]);

        // Resolve codecs: explicit format overrides the legacy `transcode` toggle.
        let (vcodec, acodec, bitrate_kbps) = match (self.format.as_ref(), self.transcode) {
            (Some(f), _) => (f.video_codec.clone(), f.audio_codec.clone(), f.bitrate_kbps),
            (None, true) => ("h264_nvenc".to_string(), "aac".to_string(), None),
            (None, false) => ("copy".to_string(), "copy".to_string(), None),
        };

        if vcodec == "copy" && acodec == "copy" {
            cmd.args(["-c", "copy"]);
        } else {
            cmd.args(["-c:v", &vcodec]);
            if vcodec == "h264_nvenc" {
                cmd.args(["-preset", "p4"]);
                if let Some(kbps) = bitrate_kbps {
                    cmd.args(["-b:v", &format!("{kbps}k")]);
                } else {
                    cmd.args(["-cq", "23"]);
                }
            } else if vcodec == "libx264" {
                cmd.args(["-preset", "veryfast"]);
                if let Some(kbps) = bitrate_kbps {
                    cmd.args(["-b:v", &format!("{kbps}k")]);
                } else {
                    cmd.args(["-crf", "23"]);
                }
            }
            cmd.args(["-c:a", &acodec]);
            if acodec != "copy" {
                cmd.args(["-b:a", "192k"]);
            }
        }

        cmd.arg(&self.output_path);

        // Don't inherit stdin so we can send signals
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // Drain stderr asynchronously: a piped+un-drained stderr fills
        // the kernel pipe buffer and stalls ffmpeg. Also keep the last
        // STDERR_TAIL_LINES so failure paths can surface the real error.
        let stderr_tail = Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(
            STDERR_TAIL_LINES,
        )));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let mut t = tail.lock().unwrap();
                    if t.len() >= STDERR_TAIL_LINES {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            });
        }

        Ok(FfmpegProcess {
            child,
            output_path: self.output_path,
            stderr_tail,
        })
    }
}

impl FfmpegProcess {
    /// Gracefully stop recording by sending SIGINT (ffmpeg writes trailer)
    pub async fn stop(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id() {
                // Send SIGINT for clean shutdown
                unsafe {
                    libc::kill(pid as i32, libc::SIGINT);
                }
                // Wait for ffmpeg to finish writing
                match tokio::time::timeout(std::time::Duration::from_secs(10), self.child.wait())
                    .await
                {
                    Ok(Ok(_)) => return Ok(()),
                    Ok(Err(e)) => {
                        tracing::warn!("ffmpeg wait error: {e}");
                    }
                    Err(_) => {
                        tracing::warn!("ffmpeg didn't stop in 10s, killing");
                        self.child.kill().await.ok();
                    }
                }
            }
        }

        #[cfg(not(unix))]
        {
            self.child.kill().await.ok();
        }

        Ok(())
    }

    /// Check if process is still running
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    /// Get the output file size in bytes
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.output_path)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Snapshot of the trailing ffmpeg stderr lines, joined with newlines.
    /// Useful for surfacing the real cause of a non-zero exit.
    pub fn stderr_tail(&self) -> String {
        self.stderr_tail
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for FfmpegProcess {
    fn drop(&mut self) {
        // If process already exited, nothing to do
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                // Still running — kill to prevent zombie
                let _ = self.child.start_kill();
            }
        }
    }
}
