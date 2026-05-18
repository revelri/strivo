pub mod catalog;
pub mod chapters;
pub mod ffmpeg;
pub mod job;
pub mod persist;
pub mod scan;
pub mod schedule;
pub mod segments;
pub mod thumbnail;
pub mod trash;
pub mod ytdlp;

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::AppEvent;
use crate::config::{AppConfig, RecordingFormat, ResolvedFormat};
use crate::platform::PlatformKind;
use crate::recording::ffmpeg::{FfmpegBuilder, FfmpegProcess};
use crate::recording::job::{RecordingJob, RecordingState};
use crate::recording::ytdlp::YtDlpProcess;
use crate::stream::resolver;

/// Resolve the format/quality settings for a recording, walking
/// per-channel override → global → built-in defaults.
pub fn resolve_format(
    config: &AppConfig,
    channel_id: &str,
    platform: PlatformKind,
) -> ResolvedFormat {
    let platform_str = platform.to_string();
    let channel_override: Option<&RecordingFormat> = config
        .auto_record_channels
        .iter()
        .find(|c| c.channel_id == channel_id && c.platform == platform_str)
        .and_then(|c| c.format.as_ref());
    RecordingFormat::resolved(channel_override, &config.recording.format)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RecordingCommand {
    Start {
        channel_id: String,
        channel_name: String,
        platform: PlatformKind,
        transcode: bool,
        cookies_path: Option<PathBuf>,
        stream_title: Option<String>,
        from_start: bool,
        /// If provided, the recording manager uses this ID instead of generating a new one.
        /// Used by the schedule manager to track job IDs for timed Stop commands.
        job_id: Option<Uuid>,
    },
    Stop {
        job_id: Uuid,
    },
    StopAll,
    DownloadVod {
        url: String,
        channel_name: String,
        platform: PlatformKind,
        output_path: PathBuf,
        cookies_path: Option<PathBuf>,
        post_title: Option<String>,
    },
}

/// Unified recorder process — either FFmpeg or yt-dlp
enum RecorderProcess {
    Ffmpeg(FfmpegProcess),
    YtDlp(YtDlpProcess),
}

impl RecorderProcess {
    async fn stop(&mut self) -> Result<()> {
        match self {
            Self::Ffmpeg(p) => p.stop().await,
            Self::YtDlp(p) => p.stop().await,
        }
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        match self {
            Self::Ffmpeg(p) => p.try_wait(),
            Self::YtDlp(p) => p.try_wait(),
        }
    }

    fn file_size(&self) -> u64 {
        match self {
            Self::Ffmpeg(p) => p.file_size(),
            Self::YtDlp(p) => p.file_size(),
        }
    }
}

struct ActiveRecording {
    job: RecordingJob,
    process: Option<RecorderProcess>,
    retry_count: u32,
    cookies_path: Option<PathBuf>,
    from_start: bool,
    /// All on-disk segments produced for this recording so far. Element 0
    /// is always the original output path; subsequent retries append
    /// `_partN.mkv` paths via `segments::segment_path`. On Finished the
    /// orchestrator merges them back into the base path via mkvmerge
    /// (M5.5).
    segments: Vec<PathBuf>,
}

pub async fn run_manager(
    config: AppConfig,
    mut cmd_rx: mpsc::UnboundedReceiver<RecordingCommand>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    cancel: CancellationToken,
) {
    let mut active: HashMap<Uuid, ActiveRecording> = HashMap::new();
    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(2));
    // Channel for spawned resolve tasks to send back results
    let (resolve_tx, mut resolve_rx) =
        mpsc::unbounded_channel::<(Uuid, Result<RecorderProcess, String>)>();

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    RecordingCommand::Start { channel_id, channel_name, platform, transcode, cookies_path, stream_title, from_start, job_id: requested_id } => {
                        // Check if already recording this channel
                        let already = active.values().any(|r| {
                            r.job.channel_id == channel_id
                                && !matches!(r.job.state, RecordingState::Finished | RecordingState::Failed)
                        });
                        if already {
                            let _ = event_tx.send(AppEvent::error(
                                format!("Already recording {channel_name}")
                            ));
                            continue;
                        }

                        let output_path = build_output_path(&config, &channel_name, platform, stream_title.as_deref());
                        let mut job = RecordingJob::new(
                            channel_id.clone(),
                            channel_name.clone(),
                            platform,
                            output_path.clone(),
                            transcode,
                            stream_title,
                        );
                        if let Some(id) = requested_id {
                            job.id = id;
                        }
                        let job_id = job.id;
                        let _ = event_tx.send(AppEvent::recording_started(job.clone()));

                        active.insert(job_id, ActiveRecording {
                            job,
                            process: None,
                            retry_count: 0,
                            cookies_path: cookies_path.clone(),
                            from_start,
                            segments: vec![output_path.clone()],
                        });

                        let resolved_format = resolve_format(&config, &channel_id, platform);

                        // YouTube + from_start: use yt-dlp directly (no URL resolution needed)
                        if platform == PlatformKind::YouTube && from_start {
                            let rtx = resolve_tx.clone();
                            let url = if channel_name.starts_with("UC") && channel_name.len() == 24 {
                                format!("https://www.youtube.com/channel/{channel_name}/live")
                            } else {
                                format!("https://www.youtube.com/@{channel_name}/live")
                            };
                            let cookies = cookies_path.clone();
                            let fmt = resolved_format.clone();
                            tokio::spawn(async move {
                                match YtDlpProcess::with_options(&url, output_path, cookies.as_deref(), Some(&fmt), true) {
                                    Ok(process) => {
                                        let _ = rtx.send((job_id, Ok(RecorderProcess::YtDlp(process))));
                                    }
                                    Err(e) => {
                                        let _ = rtx.send((job_id, Err(format!("yt-dlp failed: {e}"))));
                                    }
                                }
                            });
                        } else {
                            // Normal path: resolve URL then spawn FFmpeg
                            if from_start && platform == PlatformKind::Twitch {
                                tracing::warn!("Record from start not supported for Twitch, falling back to normal recording");
                            }

                            let rtx = resolve_tx.clone();
                            let etx = event_tx.clone();
                            let fmt = resolved_format.clone();
                            tokio::spawn(async move {
                                match resolver::resolve_stream_url(platform, &channel_name, cookies_path.as_deref()).await {
                                    Ok(stream_info) => {
                                        let _ = etx.send(AppEvent::stream_url_resolved(
                                            channel_id.clone(),
                                            stream_info.url.clone(),
                                        ));
                                        match FfmpegBuilder::new(stream_info.url, output_path)
                                            .transcode(transcode)
                                            .format(fmt)
                                            .build()
                                        {
                                            Ok(process) => {
                                                let _ = rtx.send((job_id, Ok(RecorderProcess::Ffmpeg(process))));
                                            }
                                            Err(e) => {
                                                let _ = rtx.send((job_id, Err(format!("FFmpeg failed: {e}"))));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = rtx.send((job_id, Err(format!("Resolve failed: {e}"))));
                                    }
                                }
                            });
                        }
                    }
                    RecordingCommand::Stop { job_id } => {
                        if let Some(mut rec) = active.remove(&job_id) {
                            rec.job.state = RecordingState::Stopping;
                            if let Some(ref mut proc) = rec.process {
                                if let Err(e) = proc.stop().await {
                                    tracing::error!("Failed to stop recorder: {e}");
                                }
                            }
                            rec.job.state = RecordingState::Finished;
                            let _ = event_tx.send(AppEvent::recording_finished(job_id, RecordingState::Finished, None));
                        }
                    }
                    RecordingCommand::StopAll => {
                        let ids: Vec<Uuid> = active.keys().copied().collect();
                        for id in ids {
                            if let Some(mut rec) = active.remove(&id) {
                                if matches!(rec.job.state, RecordingState::Recording | RecordingState::ResolvingUrl) {
                                    rec.job.state = RecordingState::Stopping;
                                    if let Some(ref mut proc) = rec.process {
                                        proc.stop().await.ok();
                                    }
                                    rec.job.state = RecordingState::Finished;
                                    let _ = event_tx.send(AppEvent::recording_finished(id, RecordingState::Finished, None));
                                }
                            }
                        }
                        let _ = event_tx.send(AppEvent::all_recordings_stopped());
                    }
                    RecordingCommand::DownloadVod { url, channel_name, platform, output_path, cookies_path, post_title } => {
                        let job = RecordingJob::new(
                            String::new(),
                            channel_name,
                            platform,
                            output_path.clone(),
                            false,
                            post_title,
                        );
                        let job_id = job.id;
                        let _ = event_tx.send(AppEvent::recording_started(job.clone()));

                        active.insert(job_id, ActiveRecording {
                            job,
                            process: None,
                            retry_count: 0,
                            cookies_path: cookies_path.clone(),
                            from_start: false,
                            segments: vec![output_path.clone()],
                        });

                        let rtx = resolve_tx.clone();
                        let fmt = resolve_format(&config, "", platform);
                        tokio::spawn(async move {
                            match YtDlpProcess::with_options(&url, output_path, cookies_path.as_deref(), Some(&fmt), false) {
                                Ok(process) => {
                                    let _ = rtx.send((job_id, Ok(RecorderProcess::YtDlp(process))));
                                }
                                Err(e) => {
                                    let _ = rtx.send((job_id, Err(format!("yt-dlp VOD download failed: {e}"))));
                                }
                            }
                        });
                    }
                }
            }
            Some((job_id, result)) = resolve_rx.recv() => {
                if let Some(rec) = active.get_mut(&job_id) {
                    match result {
                        Ok(process) => {
                            rec.process = Some(process);
                            rec.job.state = RecordingState::Recording;
                            rec.job.started_at = chrono::Utc::now();
                        }
                        Err(e) => {
                            rec.job.state = RecordingState::Failed;
                            rec.job.error = Some(e.clone());
                            let _ = event_tx.send(AppEvent::recording_finished(job_id, RecordingState::Failed, Some(e.clone())));
                            let _ = event_tx.send(AppEvent::error(e));
                        }
                    }
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Recording manager shutting down, stopping all recordings");
                let ids: Vec<Uuid> = active.keys().copied().collect();
                for id in ids {
                    if let Some(mut rec) = active.remove(&id) {
                        if matches!(rec.job.state, RecordingState::Recording | RecordingState::ResolvingUrl) {
                            if let Some(ref mut proc) = rec.process {
                                proc.stop().await.ok();
                            }
                        }
                    }
                }
                break;
            }
            _ = poll_interval.tick() => {
                let mut finished = Vec::new();
                for (id, rec) in active.iter_mut() {
                    if rec.job.state != RecordingState::Recording {
                        continue;
                    }
                    if let Some(ref mut proc) = rec.process {
                        rec.job.bytes_written = proc.file_size();
                        rec.job.duration_secs = (chrono::Utc::now() - rec.job.started_at)
                            .num_seconds() as f64;

                        let _ = event_tx.send(AppEvent::recording_progress(*id, rec.job.bytes_written, rec.job.duration_secs));

                        match proc.try_wait() {
                            Ok(Some(status)) => {
                                if status.success() {
                                    rec.job.state = RecordingState::Finished;
                                    finished.push((*id, RecordingState::Finished, None));
                                } else if !rec.from_start && rec.retry_count < 3 {
                                    // M5.5 gap-resume: keep the prior segment on
                                    // disk and write the next chunk to
                                    // `<base>_partN.mkv`. After Finished the
                                    // segments merge back into the base file.
                                    rec.retry_count += 1;
                                    let wait_secs = 2u64.pow(rec.retry_count);
                                    tracing::warn!(
                                        "Recorder exited with {status}, resume segment {}/3 in {wait_secs}s for {}",
                                        rec.retry_count,
                                        rec.job.channel_name
                                    );
                                    rec.job.state = RecordingState::ResolvingUrl;
                                    rec.process = None;

                                    // Segment N path derives from the original
                                    // base (segments[0]).
                                    let base = rec.segments[0].clone();
                                    let segment_path = segments::segment_path(&base, rec.retry_count + 1);
                                    rec.segments.push(segment_path.clone());
                                    rec.job.output_path = segment_path;

                                    // Re-resolve and restart
                                    let rtx = resolve_tx.clone();
                                    let job = rec.job.clone();
                                    let jid = *id;
                                    let retry_cookies = rec.cookies_path.clone();
                                    let retry_fmt = resolve_format(&config, &job.channel_id, job.platform);
                                    tokio::spawn(async move {
                                        tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                                        match resolver::resolve_stream_url(
                                            job.platform,
                                            &job.channel_name,
                                            retry_cookies.as_deref(),
                                        ).await {
                                            Ok(info) => {
                                                match FfmpegBuilder::new(info.url, job.output_path)
                                                    .transcode(job.transcode)
                                                    .format(retry_fmt)
                                                    .build()
                                                {
                                                    Ok(p) => { let _ = rtx.send((jid, Ok(RecorderProcess::Ffmpeg(p)))); }
                                                    Err(e) => { let _ = rtx.send((jid, Err(format!("{e}")))); }
                                                }
                                            }
                                            Err(e) => {
                                                let _ = rtx.send((jid, Err(format!("{e}"))));
                                            }
                                        }
                                    });
                                } else {
                                    let error_msg = format!("Recorder exited: {status} after {} retries", rec.retry_count);
                                    rec.job.state = RecordingState::Failed;
                                    rec.job.error = Some(error_msg.clone());
                                    finished.push((*id, RecordingState::Failed, Some(error_msg)));
                                }
                            }
                            Ok(None) => {} // still running
                            Err(e) => {
                                tracing::error!("Failed to check recorder status: {e}");
                            }
                        }
                    }
                }
                for (id, final_state, error) in finished {
                    let rec = active.remove(&id);
                    // M5.5: if this recording produced multiple segments,
                    // merge them back into the base path via mkvmerge before
                    // emitting RecordingFinished. Single-segment recordings
                    // (the common case) emit inline.
                    let needs_merge = matches!(
                        (final_state, rec.as_ref()),
                        (RecordingState::Finished, Some(r)) if r.segments.len() > 1
                    );
                    if let (true, Some(r)) = (needs_merge, rec) {
                        let etx = event_tx.clone();
                        let job_id = id;
                        let base = r.segments[0].clone();
                        let segs = r.segments.clone();
                        tokio::task::spawn_blocking(move || {
                            // Output to a sibling temp path; mkvmerge refuses
                            // to overwrite one of its own inputs.
                            let parent = base.parent().unwrap_or(std::path::Path::new("."));
                            let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("recording");
                            let ext = base.extension().and_then(|s| s.to_str()).unwrap_or("mkv");
                            let temp = parent.join(format!(".{stem}.merging.{ext}"));
                            let merged = match segments::merge_segments(&segs, &temp) {
                                Ok(()) => true,
                                Err(e) => {
                                    tracing::warn!(job_id = %job_id, error = %e, "merge failed; keeping segments");
                                    false
                                }
                            };
                            let final_state = if merged {
                                // Rename the merged temp over the base path,
                                // then unlink the part files.
                                if let Err(e) = std::fs::rename(&temp, &base) {
                                    tracing::warn!(error = %e, "rename merged file failed");
                                    let _ = std::fs::remove_file(&temp);
                                    return etx.send(AppEvent::recording_finished(
                                        job_id,
                                        RecordingState::Finished,
                                        Some(format!("merged segments preserved as {}", temp.display())),
                                    ));
                                }
                                for s in segs.iter().skip(1) {
                                    let _ = std::fs::remove_file(s);
                                }
                                tracing::info!(job_id = %job_id, "merged {} segments", segs.len());
                                RecordingState::Finished
                            } else {
                                let _ = std::fs::remove_file(&temp);
                                RecordingState::Finished
                            };
                            etx.send(AppEvent::recording_finished(job_id, final_state, None))
                        });
                    } else {
                        let _ = event_tx.send(AppEvent::recording_finished(id, final_state, error));
                    }
                }
            }
        }
    }
}

pub fn build_output_path(
    config: &AppConfig,
    channel_name: &str,
    platform: PlatformKind,
    stream_title: Option<&str>,
) -> PathBuf {
    let now = chrono::Local::now();
    let date = now.format("%Y-%m-%d_%H%M%S");
    let platform_str = match platform {
        PlatformKind::Twitch => "twitch",
        PlatformKind::YouTube => "youtube",
        PlatformKind::Patreon => "patreon",
    };

    // Sanitize stream title for filesystem safety
    let title = stream_title.unwrap_or("stream");
    let safe_title: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string();
    let safe_title = if safe_title.is_empty() {
        "stream".to_string()
    } else {
        safe_title
    };
    // Truncate to avoid excessively long filenames
    let safe_title: String = safe_title.chars().take(80).collect();

    let filename = config
        .recording
        .filename_template
        .replace("{channel}", channel_name)
        .replace("{date}", &date.to_string())
        .replace("{title}", &safe_title)
        .replace("{platform}", platform_str);

    disambiguate_path(config.recording_dir.join(filename))
}

/// Compute the per-episode output directory for catalog-pull and structured recordings.
///
/// Layout: `{root}/{platform}/{channel}/{YYYY-MM-DD}_{title}/`
///
/// Both `channel` and `title` are filesystem-sanitized. The result is *not*
/// disambiguated — a re-run that lands on the same date+title will reuse the
/// directory; the catalog index in §5 is what guarantees we don't re-download.
pub fn episode_dir(
    root: &std::path::Path,
    platform: PlatformKind,
    channel: &str,
    date: chrono::DateTime<chrono::Utc>,
    title: &str,
) -> PathBuf {
    let platform_str = match platform {
        PlatformKind::Twitch => "twitch",
        PlatformKind::YouTube => "youtube",
        PlatformKind::Patreon => "patreon",
    };
    let date_str = date.format("%Y-%m-%d").to_string();
    let leaf = format!("{date_str}_{}", sanitize_path_component(title));
    root.join(platform_str)
        .join(sanitize_path_component(channel))
        .join(leaf)
}

/// Strip filesystem-hostile characters and clamp length so deeply-nested paths
/// don't exceed PATH_MAX on any platform.
pub fn sanitize_path_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    let truncated: String = trimmed.chars().take(80).collect();
    if truncated.is_empty() {
        "untitled".to_string()
    } else {
        truncated
    }
}

/// Per-episode metadata sidecar. Written next to `video.mkv` after a catalog-pull
/// recording finishes so downstream tools (Crunchr, archiver, etc.) have provenance
/// without parsing filenames.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EpisodeMetadata {
    pub platform: String,
    pub channel_id: String,
    pub channel_name: String,
    pub vod_id: String,
    pub title: String,
    pub source_url: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub duration_secs: Option<f64>,
    pub format: String,
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Serialize EpisodeMetadata to `{episode_dir}/metadata.json`, creating the dir
/// if needed. Best-effort: errors are returned but never panic.
pub fn write_metadata_json(episode_dir: &std::path::Path, meta: &EpisodeMetadata) -> Result<()> {
    std::fs::create_dir_all(episode_dir)?;
    let path = episode_dir.join("metadata.json");
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RecordingFormat;

    #[test]
    fn format_resolution_precedence() {
        let global = RecordingFormat {
            format: Some("bestvideo+bestaudio".into()),
            container: Some("mp4".into()),
            ..Default::default()
        };
        let channel = RecordingFormat {
            format: Some("worst".into()),
            ..Default::default()
        };
        let r = RecordingFormat::resolved(Some(&channel), &global);
        assert_eq!(r.format, "worst", "channel wins on format");
        assert_eq!(r.container, "mp4", "global fills missing container");
        assert_eq!(r.video_codec, "copy", "built-in default copy");
        assert_eq!(r.audio_codec, "copy");
    }

    #[test]
    fn format_resolution_uses_builtin_default_when_empty() {
        let r = RecordingFormat::resolved(None, &RecordingFormat::default());
        assert_eq!(r.format, "best");
        assert_eq!(r.container, "mkv");
        assert_eq!(r.video_codec, "copy");
        assert_eq!(r.audio_codec, "copy");
    }

    #[test]
    fn episode_dir_layout() {
        let root = std::path::PathBuf::from("/tmp/strivo");
        let date = chrono::DateTime::parse_from_rfc3339("2026-04-12T15:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let dir = episode_dir(
            &root,
            PlatformKind::Patreon,
            "Some Creator",
            date,
            "Episode 1: Hello!",
        );
        assert_eq!(
            dir,
            std::path::PathBuf::from(
                "/tmp/strivo/patreon/Some Creator/2026-04-12_Episode 1_ Hello_"
            )
        );
    }

    #[test]
    fn sanitize_clamps_and_strips() {
        assert_eq!(sanitize_path_component(""), "untitled");
        assert_eq!(sanitize_path_component("...."), "untitled");
        assert_eq!(sanitize_path_component("a/b\\c:d"), "a_b_c_d");
        let long = "x".repeat(200);
        assert_eq!(sanitize_path_component(&long).len(), 80);
    }
}

/// If `path` already exists, return `stem_1.ext`, `stem_2.ext`, ... until a
/// free slot is found. Guards against two concurrent recordings that resolve
/// to the same template-rendered filename silently stomping each other.
fn disambiguate_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned());
    for n in 1u32.. {
        let candidate_name = match &ext {
            Some(e) => format!("{stem}_{n}.{e}"),
            None => format!("{stem}_{n}"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}
