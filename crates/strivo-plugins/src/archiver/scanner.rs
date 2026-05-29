use std::path::Path;
use std::process::Stdio;

use anyhow::Result;

use super::types::VideoEntry;

/// Filter applied to a scanned video catalog before submitting batches.
/// All fields are optional; "no filter" matches everything. (R1.)
#[allow(dead_code)] // consumed by the Catalog view (R1 phase 2 render)
#[derive(Debug, Clone, Default)]
pub struct CatalogFilter {
    /// Title regex (case-insensitive). Compiled lazily — bad regexes
    /// are reported as a no-op match by [`Self::matches`].
    pub title_regex: Option<String>,
    /// Only keep videos uploaded ≥ this YYYYMMDD string when set.
    pub since_yyyymmdd: Option<String>,
    /// Only keep videos uploaded < this YYYYMMDD string when set.
    pub until_yyyymmdd: Option<String>,
    /// Minimum duration in seconds.
    pub min_duration_secs: Option<f64>,
    /// Maximum duration in seconds.
    pub max_duration_secs: Option<f64>,
    /// Only keep videos belonging to this playlist when set.
    pub playlist: Option<String>,
}

impl CatalogFilter {
    #[allow(dead_code)] // consumed by the Catalog view (R1 phase 2 render)
    pub fn matches(&self, entry: &VideoEntry) -> bool {
        if let Some(rx) = &self.title_regex {
            // Use case-insensitive substring match instead of pulling in
            // a regex crate; cheap to upgrade later when the UI demands
            // anchored patterns.
            if !entry
                .title
                .to_lowercase()
                .contains(&rx.to_lowercase())
            {
                return false;
            }
        }
        if let Some(since) = &self.since_yyyymmdd {
            if entry.upload_date.as_str() < since.as_str() {
                return false;
            }
        }
        if let Some(until) = &self.until_yyyymmdd {
            if entry.upload_date.as_str() >= until.as_str() {
                return false;
            }
        }
        if let Some(min) = self.min_duration_secs {
            if entry.duration_secs.unwrap_or(0.0) < min {
                return false;
            }
        }
        if let Some(max) = self.max_duration_secs {
            if entry.duration_secs.unwrap_or(f64::MAX) > max {
                return false;
            }
        }
        if let Some(pl) = &self.playlist {
            let matches_pl = entry
                .playlist
                .as_ref()
                .map(|p| p.eq_ignore_ascii_case(pl))
                .unwrap_or(false);
            if !matches_pl {
                return false;
            }
        }
        true
    }
}

/// Apply a [`CatalogFilter`] over a scanned catalog and optionally cap
/// the result at `max`. Newest-first by upload date when both
/// `upload_date` fields are present.
#[allow(dead_code)] // consumed by the Catalog view (R1 phase 2 render)
pub fn filter_and_cap(
    entries: &[VideoEntry],
    filter: &CatalogFilter,
    max: Option<usize>,
) -> Vec<VideoEntry> {
    let mut matched: Vec<VideoEntry> = entries
        .iter()
        .filter(|e| filter.matches(e))
        .cloned()
        .collect();
    matched.sort_by(|a, b| b.upload_date.cmp(&a.upload_date));
    if let Some(n) = max {
        matched.truncate(n);
    }
    matched
}

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
        anyhow::bail!(
            "yt-dlp scan failed: {}",
            stderr.chars().take(300).collect::<String>()
        );
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)?;

    let entries = parsed["entries"].as_array().cloned().unwrap_or_default();

    let videos: Vec<VideoEntry> = entries
        .iter()
        .filter_map(|entry| {
            let video_id = entry["id"].as_str()?.to_string();
            let title = entry["title"].as_str().unwrap_or("Untitled").to_string();
            let upload_date = entry["upload_date"].as_str().unwrap_or("").to_string();
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

    fn make_entry(id: &str, title: &str, date: &str, dur: f64, pl: Option<&str>) -> VideoEntry {
        VideoEntry {
            video_id: id.into(),
            title: title.into(),
            upload_date: date.into(),
            duration_secs: Some(dur),
            playlist: pl.map(String::from),
            downloaded: false,
        }
    }

    #[test]
    fn filter_title_substring() {
        let f = CatalogFilter {
            title_regex: Some("recap".into()),
            ..Default::default()
        };
        assert!(f.matches(&make_entry("a", "Weekly RECAP show", "20260101", 100.0, None)));
        assert!(!f.matches(&make_entry("b", "Other stream", "20260101", 100.0, None)));
    }

    #[test]
    fn filter_date_window() {
        let f = CatalogFilter {
            since_yyyymmdd: Some("20260101".into()),
            until_yyyymmdd: Some("20260201".into()),
            ..Default::default()
        };
        assert!(f.matches(&make_entry("a", "x", "20260115", 100.0, None)));
        assert!(!f.matches(&make_entry("b", "x", "20251231", 100.0, None)));
        assert!(!f.matches(&make_entry("c", "x", "20260201", 100.0, None)));
    }

    #[test]
    fn filter_duration_range() {
        let f = CatalogFilter {
            min_duration_secs: Some(60.0),
            max_duration_secs: Some(7200.0),
            ..Default::default()
        };
        assert!(f.matches(&make_entry("a", "x", "20260101", 1800.0, None)));
        assert!(!f.matches(&make_entry("b", "x", "20260101", 30.0, None)));
        assert!(!f.matches(&make_entry("c", "x", "20260101", 99999.0, None)));
    }

    #[test]
    fn filter_and_cap_newest_first() {
        let entries = vec![
            make_entry("a", "Old", "20260101", 100.0, None),
            make_entry("b", "New", "20260201", 100.0, None),
            make_entry("c", "Mid", "20260115", 100.0, None),
        ];
        let result = filter_and_cap(&entries, &CatalogFilter::default(), Some(2));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].video_id, "b"); // newest first
        assert_eq!(result[1].video_id, "c");
    }

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
