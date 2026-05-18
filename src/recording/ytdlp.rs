use anyhow::Result;
use std::path::PathBuf;
use tokio::process::{Child, Command};

use crate::config::ResolvedFormat;

pub struct YtDlpProcess {
    child: Child,
    pub output_path: PathBuf,
}

impl YtDlpProcess {
    pub fn new(
        url: &str,
        output_path: PathBuf,
        cookies_path: Option<&std::path::Path>,
    ) -> Result<Self> {
        Self::with_options(url, output_path, cookies_path, None, true)
    }

    /// Spawn yt-dlp with an explicit format selector and optional `--live-from-start`.
    /// `format` of `None` means use built-in default `"best"`.
    pub fn with_options(
        url: &str,
        output_path: PathBuf,
        cookies_path: Option<&std::path::Path>,
        format: Option<&ResolvedFormat>,
        live_from_start: bool,
    ) -> Result<Self> {
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cmd = Command::new("yt-dlp");
        if live_from_start {
            cmd.arg("--live-from-start");
        }
        cmd.arg("--continue");
        cmd.args(["--no-part"]);

        let format_str = format.map(|f| f.format.as_str()).unwrap_or("best");
        cmd.args(["-f", format_str]);

        // Bitrate hint for format selection sort.
        if let Some(kbps) = format.and_then(|f| f.bitrate_kbps) {
            cmd.args(["-S", &format!("vbr~{kbps}")]);
        }

        cmd.arg("-o");
        cmd.arg(&output_path);

        if let Some(cookies) = cookies_path {
            cmd.args(["--cookies", &cookies.to_string_lossy()]);
        }

        cmd.arg(url);

        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd.spawn()?;

        Ok(Self { child, output_path })
    }

    /// Gracefully stop by sending SIGINT, then wait
    pub async fn stop(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGINT);
                }
                match tokio::time::timeout(std::time::Duration::from_secs(15), self.child.wait())
                    .await
                {
                    Ok(Ok(_)) => return Ok(()),
                    Ok(Err(e)) => {
                        tracing::warn!("yt-dlp wait error: {e}");
                    }
                    Err(_) => {
                        tracing::warn!("yt-dlp didn't stop in 15s, killing");
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

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.output_path)
            .map(|m| m.len())
            .unwrap_or(0)
    }
}

impl Drop for YtDlpProcess {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
    }
}
