pub mod ffmpeg;
pub mod job;

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::AppEvent;
use crate::config::AppConfig;
use crate::platform::PlatformKind;
use crate::recording::ffmpeg::{FfmpegBuilder, FfmpegProcess};
use crate::recording::job::{RecordingJob, RecordingState};
use crate::stream::resolver;

#[derive(Debug)]
pub enum RecordingCommand {
    Start {
        channel_id: String,
        channel_name: String,
        platform: PlatformKind,
        transcode: bool,
        cookies_path: Option<PathBuf>,
        stream_title: Option<String>,
    },
    Stop {
        job_id: Uuid,
    },
    StopAll,
}

struct ActiveRecording {
    job: RecordingJob,
    process: Option<FfmpegProcess>,
    retry_count: u32,
    cookies_path: Option<PathBuf>,
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
        mpsc::unbounded_channel::<(Uuid, Result<FfmpegProcess, String>)>();

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    RecordingCommand::Start { channel_id, channel_name, platform, transcode, cookies_path, stream_title } => {
                        // Check if already recording this channel
                        let already = active.values().any(|r| {
                            r.job.channel_id == channel_id
                                && !matches!(r.job.state, RecordingState::Finished | RecordingState::Failed)
                        });
                        if already {
                            let _ = event_tx.send(AppEvent::Error(
                                format!("Already recording {channel_name}")
                            ));
                            continue;
                        }

                        let output_path = build_output_path(&config, &channel_name, platform, stream_title.as_deref());
                        let job = RecordingJob::new(
                            channel_id.clone(),
                            channel_name.clone(),
                            platform,
                            output_path.clone(),
                            transcode,
                            stream_title,
                        );
                        let job_id = job.id;
                        let _ = event_tx.send(AppEvent::RecordingStarted { job: job.clone() });

                        active.insert(job_id, ActiveRecording {
                            job,
                            process: None,
                            retry_count: 0,
                            cookies_path: cookies_path.clone(),
                        });

                        // Spawn resolve + start task
                        let rtx = resolve_tx.clone();
                        let etx = event_tx.clone();
                        tokio::spawn(async move {
                            match resolver::resolve_stream_url(platform, &channel_name, cookies_path.as_deref()).await {
                                Ok(stream_info) => {
                                    let _ = etx.send(AppEvent::StreamUrlResolved {
                                        channel_id: channel_id.clone(),
                                        url: stream_info.url.clone(),
                                    });
                                    match FfmpegBuilder::new(stream_info.url, output_path)
                                        .transcode(transcode)
                                        .build()
                                    {
                                        Ok(process) => {
                                            let _ = rtx.send((job_id, Ok(process)));
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
                    RecordingCommand::Stop { job_id } => {
                        if let Some(mut rec) = active.remove(&job_id) {
                            rec.job.state = RecordingState::Stopping;
                            if let Some(ref mut proc) = rec.process {
                                if let Err(e) = proc.stop().await {
                                    tracing::error!("Failed to stop ffmpeg: {e}");
                                }
                            }
                            rec.job.state = RecordingState::Finished;
                            let _ = event_tx.send(AppEvent::RecordingFinished { job_id });
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
                                    let _ = event_tx.send(AppEvent::RecordingFinished { job_id: id });
                                }
                            }
                        }
                        let _ = event_tx.send(AppEvent::AllRecordingsStopped);
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
                            let _ = event_tx.send(AppEvent::RecordingFinished { job_id });
                            let _ = event_tx.send(AppEvent::Error(e));
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

                        let _ = event_tx.send(AppEvent::RecordingProgress {
                            job_id: *id,
                            bytes_written: rec.job.bytes_written,
                            duration_secs: rec.job.duration_secs,
                        });

                        match proc.try_wait() {
                            Ok(Some(status)) => {
                                if status.success() {
                                    rec.job.state = RecordingState::Finished;
                                    finished.push(*id);
                                } else if rec.retry_count < 3 {
                                    rec.retry_count += 1;
                                    let wait_secs = 2u64.pow(rec.retry_count);
                                    tracing::warn!(
                                        "ffmpeg exited with {status}, retry {}/3 in {wait_secs}s for {}",
                                        rec.retry_count,
                                        rec.job.channel_name
                                    );
                                    rec.job.state = RecordingState::ResolvingUrl;
                                    rec.process = None;

                                    // Generate retry-specific output path to avoid overwriting
                                    let retry_path = {
                                        let orig = &rec.job.output_path;
                                        let stem = orig.file_stem().unwrap_or_default().to_string_lossy();
                                        let ext = orig.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
                                        let parent = orig.parent().unwrap_or(orig);
                                        parent.join(format!("{stem}_retry{}.{ext}", rec.retry_count))
                                    };
                                    rec.job.output_path = retry_path;

                                    // Re-resolve and restart
                                    let rtx = resolve_tx.clone();
                                    let job = rec.job.clone();
                                    let jid = *id;
                                    let retry_cookies = rec.cookies_path.clone();
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
                                                    .build()
                                                {
                                                    Ok(p) => { let _ = rtx.send((jid, Ok(p))); }
                                                    Err(e) => { let _ = rtx.send((jid, Err(format!("{e}")))); }
                                                }
                                            }
                                            Err(e) => {
                                                let _ = rtx.send((jid, Err(format!("{e}"))));
                                            }
                                        }
                                    });
                                } else {
                                    rec.job.state = RecordingState::Failed;
                                    rec.job.error = Some(format!("ffmpeg exited: {status} after 3 retries"));
                                    finished.push(*id);
                                }
                            }
                            Ok(None) => {} // still running
                            Err(e) => {
                                tracing::error!("Failed to check ffmpeg status: {e}");
                            }
                        }
                    }
                }
                for id in finished {
                    active.remove(&id);
                    let _ = event_tx.send(AppEvent::RecordingFinished { job_id: id });
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
    };

    // Sanitize stream title for filesystem safety
    let title = stream_title.unwrap_or("stream");
    let safe_title: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .trim()
        .to_string();
    let safe_title = if safe_title.is_empty() { "stream".to_string() } else { safe_title };
    // Truncate to avoid excessively long filenames
    let safe_title: String = safe_title.chars().take(80).collect();

    let filename = config
        .recording
        .filename_template
        .replace("{channel}", channel_name)
        .replace("{date}", &date.to_string())
        .replace("{title}", &safe_title)
        .replace("{platform}", platform_str);

    config.recording_dir.join(filename)
}
