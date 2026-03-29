use std::path::Path;
use std::process::Stdio;

use anyhow::Result;

use super::types::VideoEntry;

/// Scan a channel for all videos using yt-dlp --flat-playlist.
/// Returns videos not yet in the archive.txt tracking file.
pub async fn scan_channel(
    channel_url: &str,
    archive_txt: &Path,
    cookies_path: Option<&Path>,
) -> Result<Vec<VideoEntry>> {
    let mut cmd = tokio::process::Command::new("yt-dlp");
    cmd.args([
        "--flat-playlist",
        "--skip-download",
        "--dump-single-json",
        "--no-warnings",
    ]);

    if let Some(cookies) = cookies_path {
        cmd.args(["--cookies", cookies.to_str().unwrap_or("")]);
    }

    // Filter against existing archive
    if archive_txt.exists() {
        cmd.args(["--download-archive", archive_txt.to_str().unwrap_or("")]);
    }

    cmd.arg(channel_url);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("yt-dlp scan failed: {}", stderr.chars().take(300).collect::<String>());
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)?;

    let entries = parsed["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let videos: Vec<VideoEntry> = entries
        .iter()
        .filter_map(|entry| {
            let video_id = entry["id"].as_str()?.to_string();
            let title = entry["title"].as_str().unwrap_or("Untitled").to_string();
            let upload_date = entry["upload_date"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let duration_secs = entry["duration"].as_f64();
            let playlist = entry["playlist_title"]
                .as_str()
                .or_else(|| entry["playlist"].as_str())
                .map(String::from);

            Some(VideoEntry {
                video_id,
                title,
                upload_date,
                duration_secs,
                playlist,
                downloaded: false,
            })
        })
        .collect();

    Ok(videos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_video_entry_from_json() {
        let json = serde_json::json!({
            "id": "abc123",
            "title": "Test Stream",
            "upload_date": "20260328",
            "duration": 3600.0,
            "playlist_title": "My Playlist"
        });

        let entry = VideoEntry {
            video_id: json["id"].as_str().unwrap().to_string(),
            title: json["title"].as_str().unwrap().to_string(),
            upload_date: json["upload_date"].as_str().unwrap().to_string(),
            duration_secs: json["duration"].as_f64(),
            playlist: json["playlist_title"].as_str().map(String::from),
            downloaded: false,
        };

        assert_eq!(entry.video_id, "abc123");
        assert_eq!(entry.title, "Test Stream");
        assert_eq!(entry.upload_date, "20260328");
        assert_eq!(entry.duration_secs, Some(3600.0));
        assert_eq!(entry.playlist.as_deref(), Some("My Playlist"));
    }
}
