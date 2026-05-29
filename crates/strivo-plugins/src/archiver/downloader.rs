use std::path::Path;
use std::process::Stdio;

use anyhow::Result;

/// Options passed to the yt-dlp invocation. Centralizes the rate-limit
/// + per-job knobs so the call site doesn't grow another positional
/// argument per option.
#[derive(Debug, Clone, Default)]
pub struct DownloadOpts<'a> {
    /// yt-dlp `--limit-rate` value (e.g., "5M"). When empty, no limit.
    pub rate_limit: &'a str,
    /// Max retries for this video. yt-dlp's `--retries`.
    pub retries: u32,
}

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
    download_video_with_opts(
        video_url,
        output_dir,
        archive_txt,
        format,
        concurrent_fragments,
        cookies_path,
        playlist_name,
        DownloadOpts::default(),
    )
    .await
}

/// Same as [`download_video`] but with extra knobs threaded through to
/// yt-dlp. The legacy signature wraps this with `DownloadOpts::default()`
/// to keep existing call sites unchanged. (R2.)
#[allow(clippy::too_many_arguments)]
pub async fn download_video_with_opts(
    video_url: &str,
    output_dir: &Path,
    archive_txt: &Path,
    format: &str,
    concurrent_fragments: u32,
    cookies_path: Option<&Path>,
    playlist_name: Option<&str>,
    opts: DownloadOpts<'_>,
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

    if !opts.rate_limit.is_empty() {
        cmd.args(["--limit-rate", opts.rate_limit]);
    }
    if opts.retries > 0 {
        let r = opts.retries.to_string();
        cmd.args(["--retries", &r]);
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
        anyhow::bail!(
            "yt-dlp download failed: {}",
            stderr.chars().take(300).collect::<String>()
        );
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
        assert_eq!(
            video_url("abc123", "youtube"),
            "https://www.youtube.com/watch?v=abc123"
        );
    }

    #[test]
    fn video_url_twitch() {
        assert_eq!(
            video_url("12345", "twitch"),
            "https://www.twitch.tv/videos/12345"
        );
    }
}
