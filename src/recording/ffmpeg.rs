use anyhow::Result;
use std::path::PathBuf;
use tokio::process::{Child, Command};

use crate::config::ResolvedFormat;

pub struct FfmpegProcess {
    child: Child,
    pub output_path: PathBuf,
}

pub struct FfmpegBuilder {
    input_url: String,
    output_path: PathBuf,
    transcode: bool,
    format: Option<ResolvedFormat>,
}

impl FfmpegBuilder {
    pub fn new(input_url: String, output_path: PathBuf) -> Self {
        Self {
            input_url,
            output_path,
            transcode: false,
            format: None,
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

    pub fn build(self) -> Result<FfmpegProcess> {
        if let Some(parent) = self.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-y", "-hide_banner", "-loglevel", "warning"]);

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

        let child = cmd.spawn()?;

        Ok(FfmpegProcess {
            child,
            output_path: self.output_path,
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
