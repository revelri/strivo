use std::path::Path;
use std::process::Stdio;

use anyhow::Result;

/// Download a single video using yt-dlp with archive tracking.
pub async fn download_video(
    video_url: &str,
    output_dir: &Path,
    archive_txt: &Path,
    format: &str,
    concurrent_fragments: u32,
    cookies_path: Option<&Path>,
    playlist_name: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;

    let output_template = if let Some(playlist) = playlist_name {
        format!(
            "{}/Playlists/{}/%(upload_date>%m-%d-%Y)s - %(title)s.%(ext)s",
            output_dir.display(),
            playlist
        )
    } else {
        format!(
            "{}/%(upload_date>%Y-%m)s/%(upload_date>%m-%d-%Y)s - %(title)s.%(ext)s",
            output_dir.display()
        )
    };

    let mut cmd = tokio::process::Command::new("yt-dlp");
    cmd.args([
        "--download-archive",
        archive_txt.to_str().unwrap_or(""),
        "--no-overwrites",
        "-f",
        format,
        "--concurrent-fragments",
        &concurrent_fragments.to_string(),
        "-o",
        &output_template,
    ]);

    if let Some(cookies) = cookies_path {
        cmd.args(["--cookies", cookies.to_str().unwrap_or("")]);
    }

    cmd.arg(video_url);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "already been recorded" is not an error
        if stderr.contains("has already been recorded") {
            return Ok(());
        }
        anyhow::bail!("yt-dlp download failed: {}", stderr.chars().take(300).collect::<String>());
    }

    Ok(())
}

/// Build a YouTube/Twitch video URL from a video ID and platform.
pub fn video_url(video_id: &str, platform: &str) -> String {
    match platform {
        "twitch" => format!("https://www.twitch.tv/videos/{video_id}"),
        _ => format!("https://www.youtube.com/watch?v={video_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_url_youtube() {
        assert_eq!(video_url("abc123", "youtube"), "https://www.youtube.com/watch?v=abc123");
    }

    #[test]
    fn video_url_twitch() {
        assert_eq!(video_url("12345", "twitch"), "https://www.twitch.tv/videos/12345");
    }
}
