use anyhow::{Result, bail, Context};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};

const SOCKET_PATH: &str = "/tmp/streavo-mpv.sock";

pub struct MpvController {
    child: Option<Child>,
    socket_path: String,
}

impl MpvController {
    pub fn new() -> Self {
        Self {
            child: None,
            socket_path: SOCKET_PATH.to_string(),
        }
    }

    /// Launch mpv with IPC server, playing the given URL
    pub async fn play(&mut self, url: &str) -> Result<()> {
        // Kill existing instance if any
        self.quit().await.ok();

        // Clean up stale socket
        let _ = std::fs::remove_file(&self.socket_path);

        let child = Command::new("mpv")
            .args([
                &format!("--input-ipc-server={}", self.socket_path),
                "--no-terminal",
                "--force-window=yes",
                "--keep-open=no",
                url,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to launch mpv - is it installed?")?;

        self.child = Some(child);

        // Wait briefly for socket to appear
        for _ in 0..20 {
            if Path::new(&self.socket_path).exists() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Ok(())
    }

    /// Play a local file
    pub async fn play_file(&mut self, path: &Path) -> Result<()> {
        self.play(&path.to_string_lossy()).await
    }

    /// Send a JSON IPC command to mpv
    async fn send_command(&self, command: &[&str]) -> Result<String> {
        let socket_path = Path::new(&self.socket_path);
        if !socket_path.exists() {
            bail!("mpv IPC socket not found");
        }

        let stream = UnixStream::connect(socket_path)
            .await
            .context("Failed to connect to mpv IPC socket")?;

        let (reader, mut writer) = tokio::io::split(stream);

        // Build JSON command
        let cmd_json = serde_json::json!({
            "command": command
        });
        let mut msg = serde_json::to_string(&cmd_json)?;
        msg.push('\n');

        writer.write_all(msg.as_bytes()).await?;
        writer.flush().await?;

        // Read response
        let mut buf_reader = BufReader::new(reader);
        let mut response = String::new();
        buf_reader.read_line(&mut response).await?;

        Ok(response)
    }

    /// Toggle play/pause
    pub async fn toggle_pause(&self) -> Result<()> {
        self.send_command(&["cycle", "pause"]).await?;
        Ok(())
    }

    /// Seek relative (seconds, can be negative)
    pub async fn seek(&self, seconds: f64) -> Result<()> {
        self.send_command(&["seek", &seconds.to_string(), "relative"]).await?;
        Ok(())
    }

    /// Get current playback position
    pub async fn get_position(&self) -> Result<f64> {
        let resp = self
            .send_command(&["get_property", "time-pos"])
            .await?;
        let parsed: serde_json::Value = serde_json::from_str(&resp)?;
        parsed["data"]
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Invalid position response"))
    }

    /// Set volume (0-100)
    pub async fn set_volume(&self, volume: u32) -> Result<()> {
        self.send_command(&["set_property", "volume", &volume.to_string()])
            .await?;
        Ok(())
    }

    /// Quit mpv
    pub async fn quit(&mut self) -> Result<()> {
        // Try IPC quit first (before borrowing self.child mutably)
        if Path::new(&self.socket_path).exists() {
            self.send_command(&["quit"]).await.ok();
        }

        if let Some(ref mut child) = self.child {
            // Wait briefly for clean exit
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                child.wait(),
            )
            .await
            {
                Ok(_) => {}
                Err(_) => {
                    child.kill().await.ok();
                }
            }

            self.child = None;
        }

        let _ = std::fs::remove_file(&self.socket_path);
        Ok(())
    }

    /// Check if mpv is still running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

impl Drop for MpvController {
    fn drop(&mut self) {
        // Best-effort cleanup
        if let Some(ref mut child) = self.child {
            // Can't do async in drop, just kill
            let _ = child.start_kill();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}
