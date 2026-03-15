use anyhow::{Result, bail};
use tokio::process::Command;

use crate::platform::PlatformKind;
use crate::stream::StreamInfo;

/// Resolve the best stream URL for a channel using streamlink (Twitch) or yt-dlp (YouTube).
pub async fn resolve_stream_url(
    platform: PlatformKind,
    channel_name: &str,
    cookies_path: Option<&std::path::Path>,
) -> Result<StreamInfo> {
    match platform {
        PlatformKind::Twitch => resolve_twitch(channel_name).await,
        PlatformKind::YouTube => resolve_youtube(channel_name, cookies_path).await,
    }
}

async fn resolve_twitch(channel_name: &str) -> Result<StreamInfo> {
    let url = format!("https://twitch.tv/{channel_name}");

    // Try streamlink first
    let output = Command::new("streamlink")
        .args(["--stream-url", &url, "best"])
        .output()
        .await;

    match output {
        Ok(output) if output.status.success() => {
            let stream_url = String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_string();
            if stream_url.is_empty() {
                bail!("streamlink returned empty URL for {channel_name}");
            }
            Ok(StreamInfo {
                url: stream_url,
                quality: "best".to_string(),
                is_live: true,
            })
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("streamlink failed for {channel_name}: {stderr}");
        }
        Err(_) => {
            // Fallback to yt-dlp
            resolve_with_ytdlp(&url, None).await
        }
    }
}

async fn resolve_youtube(
    channel_name: &str,
    cookies_path: Option<&std::path::Path>,
) -> Result<StreamInfo> {
    // channel_name could be a channel ID or handle
    let url = if channel_name.starts_with("UC") && channel_name.len() == 24 {
        format!("https://www.youtube.com/channel/{channel_name}/live")
    } else {
        format!("https://www.youtube.com/@{channel_name}/live")
    };

    resolve_with_ytdlp(&url, cookies_path).await
}

async fn resolve_with_ytdlp(
    url: &str,
    cookies_path: Option<&std::path::Path>,
) -> Result<StreamInfo> {
    let mut cmd = Command::new("yt-dlp");
    cmd.args(["-g", "--no-warnings", "-f", "best"]);

    if let Some(cookies) = cookies_path {
        cmd.args(["--cookies", &cookies.to_string_lossy()]);
    }

    cmd.arg(url);

    let output = cmd.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("yt-dlp failed for {url}: {stderr}");
    }

    let stream_url = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if stream_url.is_empty() {
        bail!("yt-dlp returned empty URL for {url}");
    }

    Ok(StreamInfo {
        url: stream_url,
        quality: "best".to_string(),
        is_live: true,
    })
}
