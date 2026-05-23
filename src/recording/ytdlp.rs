use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::ResolvedFormat;

const STDERR_TAIL_LINES: usize = 40;

/// YT-2 — resolve a YouTube `/live` channel URL to the underlying
/// `/watch?v=<id>` URL of the active broadcast.
///
/// Why: `yt-dlp --live-from-start` against `/channel/UC.../live` or
/// `/@handle/live` works only when yt-dlp's extractor follows the
/// redirect cleanly. In practice we've observed the live stream
/// starting at the join-time slice when the URL form is the channel
/// alias — the extractor races the redirect and falls back to the
/// stream's live-edge cursor. Resolving to `/watch?v=<id>` first gives
/// `--live-from-start` a stable video URL it can replay against.
///
/// Implementation: shell out to `yt-dlp --print id --no-warnings
/// --no-download --no-playlist <url>` with a short timeout. Returns
/// the resolved video ID; caller composes the watch URL.
pub async fn resolve_live_video_id(
    channel_live_url: &str,
    cookies_path: Option<&std::path::Path>,
) -> Result<String> {
    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        "--print",
        "id",
        "--no-warnings",
        "--no-download",
        "--no-playlist",
        // Cap network work — if the channel isn't live, error out
        // quickly so the caller can fall back to the original URL.
        "--socket-timeout",
        "20",
    ]);
    if let Some(cookies) = cookies_path {
        cmd.args(["--cookies", &cookies.to_string_lossy()]);
    }
    cmd.arg(channel_live_url);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        async move { cmd.output().await },
    )
    .await
    .context("yt-dlp --print id timed out after 30 s")?
    .context("yt-dlp --print id failed to spawn")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "yt-dlp --print id exit {}: {}",
            output.status,
            stderr
                .lines()
                .last()
                .unwrap_or("(no stderr)")
                .chars()
                .take(200)
                .collect::<String>()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let video_id = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .ok_or_else(|| anyhow::anyhow!("yt-dlp --print id returned empty output"))?;

    // Sanity-check the shape. YouTube video IDs are 11 chars of base64
    // [A-Za-z0-9_-]. Anything else is suspicious.
    if video_id.len() != 11 || !video_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
        anyhow::bail!("yt-dlp --print id returned unexpected shape: {video_id:?}");
    }
    Ok(video_id)
}

pub struct YtDlpProcess {
    child: Child,
    pub output_path: PathBuf,
    stderr_tail: Arc<Mutex<std::collections::VecDeque<String>>>,
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
            // YT-3 — give yt-dlp a brief grace period to find the
            // video when the stream is just coming online. Default
            // is to fail immediately, which loses the first 30 s of
            // many recordings to the user's reaction time.
            cmd.args(["--wait-for-video", "30"]);
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

        let mut child = cmd.spawn()?;

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

        Ok(Self { child, output_path, stderr_tail })
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
