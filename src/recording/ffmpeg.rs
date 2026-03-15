use anyhow::Result;
use std::path::PathBuf;
use tokio::process::{Child, Command};

pub struct FfmpegProcess {
    child: Child,
    pub output_path: PathBuf,
}

pub struct FfmpegBuilder {
    input_url: String,
    output_path: PathBuf,
    transcode: bool,
}

impl FfmpegBuilder {
    pub fn new(input_url: String, output_path: PathBuf) -> Self {
        Self {
            input_url,
            output_path,
            transcode: false,
        }
    }

    pub fn transcode(mut self, enabled: bool) -> Self {
        self.transcode = enabled;
        self
    }

    pub fn build(self) -> Result<FfmpegProcess> {
        // Ensure output directory exists
        if let Some(parent) = self.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-y", "-hide_banner", "-loglevel", "warning"]);

        // Input
        cmd.args(["-i", &self.input_url]);

        if self.transcode {
            // NVENC hardware transcode
            cmd.args([
                "-c:v", "h264_nvenc",
                "-preset", "p4",
                "-cq", "23",
                "-c:a", "aac",
                "-b:a", "192k",
            ]);
        } else {
            // Passthrough (no re-encoding)
            cmd.args(["-c", "copy"]);
        }

        // MKV container for crash resilience
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
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    self.child.wait(),
                )
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
