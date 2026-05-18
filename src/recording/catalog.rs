//! Catalog-pull runner: enumerate a channel's back catalog, dedupe via
//! [`PersistDb`], and download each missing episode into the per-episode layout
//! `{root}/{platform}/{channel}/{date}_{title}/`.
//!
//! The runner is driver-agnostic — callers feed in a list of [`VodEntry`]s
//! (typically from `Platform::fetch_channel_vods` / `PatreonClient::fetch_channel_vods`)
//! and it handles dedup, directory layout, yt-dlp invocation, and metadata sidecar.

use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::ResolvedFormat;
use crate::platform::{PlatformKind, VodEntry};
use crate::recording::persist::PersistDb;
use crate::recording::ytdlp::YtDlpProcess;
use crate::recording::{episode_dir, write_metadata_json, EpisodeMetadata};

/// Outcome of a catalog pull, surfaced to the CLI / TUI for a final summary.
#[derive(Debug, Default, Clone)]
pub struct CatalogReport {
    pub discovered: usize,
    pub skipped_existing: usize,
    pub downloaded: usize,
    pub failed: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct CatalogPullOptions {
    pub root: PathBuf,
    pub channel_name: String,
    pub format: ResolvedFormat,
    pub cookies_path: Option<PathBuf>,
    pub force: bool,
    /// When true, emit a `crunchr_auto` marker file so the Crunchr plugin picks
    /// the episode up automatically without per-channel tandem config.
    pub crunchr_auto: bool,
}

/// Channel for live progress events. Use unbounded — events are tiny and the
/// caller may be slow (TUI updates).
pub type ProgressTx = mpsc::UnboundedSender<CatalogProgress>;

#[derive(Debug, Clone)]
pub enum CatalogProgress {
    Discovered(usize),
    Skipped {
        vod_id: String,
        title: String,
    },
    Starting {
        vod_id: String,
        title: String,
        episode_dir: PathBuf,
    },
    Finished {
        vod_id: String,
        episode_dir: PathBuf,
    },
    Failed {
        vod_id: String,
        error: String,
    },
}

/// Run a catalog pull serially: download one episode at a time, recording each
/// into its own episode dir and updating the persist db. Serial is intentional
/// — yt-dlp parallelism saturates upstream bandwidth fast and Patreon will rate-limit.
pub async fn run_pull(
    db: &PersistDb,
    vods: Vec<VodEntry>,
    opts: &CatalogPullOptions,
    progress: Option<ProgressTx>,
) -> Result<CatalogReport> {
    let mut report = CatalogReport {
        discovered: vods.len(),
        ..Default::default()
    };
    let _ = progress
        .as_ref()
        .map(|p| p.send(CatalogProgress::Discovered(report.discovered)));

    for vod in vods {
        // Always record discovery — lets future re-runs see the title even if
        // we never get to download.
        if let Err(e) = db.upsert_catalog_entry(&vod).await {
            tracing::warn!("catalog: failed to upsert {} {}: {e}", vod.platform, vod.id);
        }

        if !opts.force {
            match db
                .is_vod_recorded(vod.platform, &vod.channel_id, &vod.id)
                .await
            {
                Ok(true) => {
                    report.skipped_existing += 1;
                    let _ = progress.as_ref().map(|p| {
                        p.send(CatalogProgress::Skipped {
                            vod_id: vod.id.clone(),
                            title: vod.title.clone(),
                        })
                    });
                    continue;
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("catalog: dedupe check failed: {e}"),
            }
        }

        let date = vod.published_at.unwrap_or_else(Utc::now);
        let ep_dir = episode_dir(
            &opts.root,
            vod.platform,
            &opts.channel_name,
            date,
            &vod.title,
        );
        let video_path = ep_dir.join(video_filename(vod.platform, &opts.format));

        let _ = progress.as_ref().map(|p| {
            p.send(CatalogProgress::Starting {
                vod_id: vod.id.clone(),
                title: vod.title.clone(),
                episode_dir: ep_dir.clone(),
            })
        });

        match download_one(&vod, &video_path, opts).await {
            Ok(bytes) => {
                let meta = EpisodeMetadata {
                    platform: vod.platform.to_string(),
                    channel_id: vod.channel_id.clone(),
                    channel_name: opts.channel_name.clone(),
                    vod_id: vod.id.clone(),
                    title: vod.title.clone(),
                    source_url: vod.url.clone(),
                    published_at: vod.published_at,
                    recorded_at: Utc::now(),
                    duration_secs: vod.duration.map(|d| d.as_secs_f64()),
                    format: opts.format.format.clone(),
                    container: opts.format.container.clone(),
                    video_codec: opts.format.video_codec.clone(),
                    audio_codec: opts.format.audio_codec.clone(),
                    bytes,
                    sha256: None,
                };
                if let Err(e) = write_metadata_json(&ep_dir, &meta) {
                    tracing::warn!("catalog: metadata.json write failed: {e}");
                }
                if opts.crunchr_auto {
                    // Marker the Crunchr plugin can grep for in lieu of tandem config.
                    let _ = std::fs::write(ep_dir.join(".crunchr-auto"), b"");
                }
                if let Err(e) = db
                    .mark_vod_recorded(vod.platform, &vod.channel_id, &vod.id, &ep_dir)
                    .await
                {
                    tracing::warn!("catalog: mark_vod_recorded failed: {e}");
                }
                report.downloaded += 1;
                let _ = progress.as_ref().map(|p| {
                    p.send(CatalogProgress::Finished {
                        vod_id: vod.id.clone(),
                        episode_dir: ep_dir.clone(),
                    })
                });
            }
            Err(e) => {
                let msg = format!("{e:#}");
                report.failed.push((vod.id.clone(), msg.clone()));
                let _ = progress.as_ref().map(|p| {
                    p.send(CatalogProgress::Failed {
                        vod_id: vod.id.clone(),
                        error: msg,
                    })
                });
            }
        }
    }

    Ok(report)
}

/// Pick a sensible video filename inside the episode dir. The container honors
/// `format.container` so callers transcoding to mp4 land at `video.mp4`.
fn video_filename(_platform: PlatformKind, format: &ResolvedFormat) -> String {
    let ext = match format.container.as_str() {
        "" => "mkv",
        s => s,
    };
    format!("video.{ext}")
}

async fn download_one(vod: &VodEntry, target: &Path, opts: &CatalogPullOptions) -> Result<u64> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // VOD downloads are bounded, so yt-dlp without --live-from-start.
    let mut proc = YtDlpProcess::with_options(
        &vod.url,
        target.to_path_buf(),
        opts.cookies_path.as_deref(),
        Some(&opts.format),
        false,
    )
    .with_context(|| format!("spawn yt-dlp for {}", vod.url))?;

    // Poll until exit. yt-dlp downloads are typically minutes, so 1s polling is fine.
    loop {
        match proc.try_wait()? {
            Some(status) => {
                if !status.success() {
                    anyhow::bail!("yt-dlp exited with {status}");
                }
                break;
            }
            None => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
        }
    }

    let bytes = std::fs::metadata(target).map(|m| m.len()).unwrap_or(0);
    Ok(bytes)
}
