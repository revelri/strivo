mod db;
mod downloader;
pub mod render;
mod scanner;
pub mod types;

use std::any::Any;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use uuid::Uuid;

use crate::app::{AppState, DaemonEvent};
use crate::config::ArchiverConfig;
use crate::platform::ChannelEntry;

use super::{
    DaemonEventKind, PaneId, Plugin, PluginAction, PluginCommand, PluginContext,
};
use types::{ArchiveJob, ArchiveState, ArchiverEvent, ArchiverView};

pub const PANE_ID: PaneId = "archiver";

pub struct ArchiverPlugin {
    db: Option<rusqlite::Connection>,
    data_dir: PathBuf,
    pub config: Option<ArchiverConfig>,
    pub jobs: Vec<ArchiveJob>,
    pub channels: Vec<ChannelEntry>,
    pub selected_channel: usize,
    pub selected_job: usize,
    pub view: ArchiverView,
    pub last_error: Option<String>,
}

impl ArchiverPlugin {
    pub fn new() -> Self {
        Self {
            db: None,
            data_dir: PathBuf::new(),
            config: None,
            jobs: Vec::new(),
            channels: Vec::new(),
            selected_channel: 0,
            selected_job: 0,
            view: ArchiverView::ChannelList,
            last_error: None,
        }
    }

    fn start_archive(&mut self, channel: &ChannelEntry) -> Vec<PluginAction> {
        let config = match self.config.as_ref() {
            Some(c) => c.clone(),
            None => return vec![PluginAction::SetStatus("Archiver: no config".to_string())],
        };

        let channel_dir = config.archive_dir.join(&channel.display_name);
        let archive_txt = channel_dir.join("archive.txt");

        // Build channel URL
        let channel_url = match channel.platform {
            crate::platform::PlatformKind::Twitch => {
                format!("https://www.twitch.tv/{}/videos?filter=archives", channel.name)
            }
            crate::platform::PlatformKind::YouTube => {
                format!("https://www.youtube.com/@{}/videos", channel.name)
            }
            _ => return Vec::new(),
        };

        let job_id = Uuid::new_v4();
        let job = ArchiveJob {
            id: job_id,
            channel_name: channel.display_name.clone(),
            channel_url: channel_url.clone(),
            platform: channel.platform,
            archive_dir: channel_dir.clone(),
            state: ArchiveState::Scanning,
            total_videos: 0,
            completed_videos: 0,
            current_video: None,
            error: None,
        };
        self.jobs.push(job);

        // Get cookies path for YouTube
        let cookies = if channel.platform == crate::platform::PlatformKind::YouTube {
            self.config.as_ref()
                .and_then(|_| None::<PathBuf>) // Would read from AppConfig.youtube.cookies_path
        } else {
            None
        };

        let url = channel_url;
        let archive = archive_txt;
        vec![PluginAction::SpawnTask {
            plugin_name: "archiver",
            future: Box::pin(async move {
                match scanner::scan_channel(&url, &archive, cookies.as_deref()).await {
                    Ok(videos) => Box::new(ArchiverEvent::ScanComplete {
                        job_id,
                        videos,
                    }) as Box<dyn Any + Send>,
                    Err(e) => Box::new(ArchiverEvent::JobError {
                        job_id,
                        error: format!("Scan failed: {e}"),
                    }) as Box<dyn Any + Send>,
                }
            }),
        }]
    }

    fn start_next_download(&mut self, job_id: Uuid) -> Vec<PluginAction> {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) else {
            return Vec::new();
        };

        if job.completed_videos >= job.total_videos {
            job.state = ArchiveState::Complete;
            return vec![PluginAction::SetStatus(format!(
                "Archiver: {} complete ({} videos)",
                job.channel_name, job.total_videos
            ))];
        }

        let Some(conn) = self.db.as_ref() else { return Vec::new() };

        // Get channel ID from DB
        let channel_id = conn.query_row(
            "SELECT id FROM channels WHERE url = ?1",
            [&job.channel_url],
            |r| r.get::<_, i64>(0),
        );
        let Ok(channel_id) = channel_id else { return Vec::new() };

        // Get next pending video
        let pending = db::get_pending_videos(conn, channel_id);
        let Ok(pending) = pending else { return Vec::new() };

        let Some((video_id, title, _date, playlist)) = pending.into_iter().next() else {
            job.state = ArchiveState::Complete;
            return Vec::new();
        };

        job.current_video = Some(title.clone());

        let config = self.config.clone().unwrap_or_default();
        let url = downloader::video_url(&video_id, &job.platform.to_string().to_lowercase());
        let output_dir = job.archive_dir.clone();
        let archive_txt = job.archive_dir.join("archive.txt");
        let format = config.format.clone();
        let fragments = config.concurrent_fragments;

        vec![PluginAction::SpawnTask {
            plugin_name: "archiver",
            future: Box::pin(async move {
                match downloader::download_video(
                    &url,
                    &output_dir,
                    &archive_txt,
                    &format,
                    fragments,
                    None,
                    playlist.as_deref(),
                ).await {
                    Ok(()) => Box::new(ArchiverEvent::VideoDownloaded {
                        job_id,
                        video_id,
                    }) as Box<dyn Any + Send>,
                    Err(e) => Box::new(ArchiverEvent::JobError {
                        job_id,
                        error: format!("Download failed: {e}"),
                    }) as Box<dyn Any + Send>,
                }
            }),
        }]
    }

    fn handle_archiver_event(&mut self, event: ArchiverEvent) -> Vec<PluginAction> {
        match event {
            ArchiverEvent::ScanComplete { job_id, videos } => {
                let video_count = videos.len();

                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) {
                    job.total_videos = video_count;
                    job.state = ArchiveState::Downloading;

                    // Insert videos into DB
                    if let Some(conn) = self.db.as_ref() {
                        let config = self.config.as_ref().map(|c| c.archive_dir.display().to_string()).unwrap_or_default();
                        if let Ok(channel_id) = db::upsert_channel(
                            conn,
                            &job.channel_name,
                            &job.channel_url,
                            &job.platform.to_string(),
                            &config,
                        ) {
                            let data: Vec<_> = videos.iter().map(|v| (
                                v.video_id.clone(),
                                v.title.clone(),
                                v.upload_date.clone(),
                                v.duration_secs,
                                v.playlist.clone(),
                            )).collect();
                            let _ = db::insert_videos(conn, channel_id, &data);
                        }
                    }
                }

                if video_count == 0 {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) {
                        job.state = ArchiveState::Complete;
                    }
                    return vec![PluginAction::SetStatus("Archiver: channel fully archived".to_string())];
                }

                self.start_next_download(job_id)
            }
            ArchiverEvent::VideoDownloaded { job_id, video_id } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) {
                    job.completed_videos += 1;

                    // Mark as downloaded in DB
                    if let Some(conn) = self.db.as_ref() {
                        if let Ok(channel_id) = conn.query_row(
                            "SELECT id FROM channels WHERE url = ?1",
                            [&job.channel_url],
                            |r| r.get::<_, i64>(0),
                        ) {
                            let _ = db::mark_downloaded(conn, channel_id, &video_id);
                        }
                    }
                }

                self.start_next_download(job_id)
            }
            ArchiverEvent::JobComplete { job_id } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) {
                    job.state = ArchiveState::Complete;
                }
                Vec::new()
            }
            ArchiverEvent::JobError { job_id, error } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) {
                    job.state = ArchiveState::Failed;
                    job.error = Some(error.clone());
                }
                self.last_error = Some(error.clone());
                vec![PluginAction::SetStatus(format!("Archiver error: {error}"))]
            }
        }
    }
}

impl Plugin for ArchiverPlugin {
    fn name(&self) -> &'static str {
        "archiver"
    }

    fn display_name(&self) -> &str {
        "Archiver"
    }

    fn init(&mut self, ctx: &PluginContext) -> anyhow::Result<()> {
        self.data_dir = ctx.data_dir.clone();

        std::fs::create_dir_all(&ctx.data_dir)?;
        std::fs::create_dir_all(&ctx.cache_dir)?;

        let db_path = ctx.data_dir.join("archiver.db");
        self.db = Some(db::open_and_init(&db_path)?);

        self.config = Some(ctx.config.archiver.clone());

        // Ensure archive directory exists
        if let Some(ref config) = self.config {
            let _ = std::fs::create_dir_all(&config.archive_dir);
        }

        tracing::info!("Archiver plugin initialized (db: {})", db_path.display());
        Ok(())
    }

    fn shutdown(&mut self) {
        self.db.take();
        tracing::info!("Archiver plugin shutting down");
    }

    fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
        Some(vec![
            DaemonEventKind::ChannelsUpdated,
        ])
    }

    fn on_event(&mut self, event: &DaemonEvent, _app: &AppState) -> Vec<PluginAction> {
        if let DaemonEvent::ChannelsUpdated(channels) = event {
            self.channels = channels.clone();
        }
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, _app: &AppState) -> Vec<PluginAction> {
        match key.code {
            KeyCode::Tab => {
                self.view = match self.view {
                    ArchiverView::ChannelList => ArchiverView::ArchiveQueue,
                    ArchiverView::ArchiveQueue => ArchiverView::ChannelList,
                };
            }
            KeyCode::Char('j') | KeyCode::Down => {
                match self.view {
                    ArchiverView::ChannelList => {
                        if !self.channels.is_empty() {
                            self.selected_channel = (self.selected_channel + 1) % self.channels.len();
                        }
                    }
                    ArchiverView::ArchiveQueue => {
                        if !self.jobs.is_empty() {
                            self.selected_job = (self.selected_job + 1) % self.jobs.len();
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                match self.view {
                    ArchiverView::ChannelList => {
                        if !self.channels.is_empty() {
                            self.selected_channel = if self.selected_channel == 0 {
                                self.channels.len() - 1
                            } else {
                                self.selected_channel - 1
                            };
                        }
                    }
                    ArchiverView::ArchiveQueue => {
                        if !self.jobs.is_empty() {
                            self.selected_job = if self.selected_job == 0 {
                                self.jobs.len() - 1
                            } else {
                                self.selected_job - 1
                            };
                        }
                    }
                }
            }
            KeyCode::Enter => {
                if self.view == ArchiverView::ChannelList {
                    if let Some(channel) = self.channels.get(self.selected_channel).cloned() {
                        return self.start_archive(&channel);
                    }
                }
            }
            KeyCode::Char('d') => {
                // Cancel selected job
                if self.view == ArchiverView::ArchiveQueue {
                    if let Some(job) = self.jobs.get_mut(self.selected_job) {
                        if job.state == ArchiveState::Downloading || job.state == ArchiveState::Scanning {
                            job.state = ArchiveState::Failed;
                            job.error = Some("Cancelled by user".to_string());
                        }
                    }
                }
            }
            KeyCode::Esc => {
                return vec![PluginAction::NavigateBack];
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_plugin_event(&mut self, event: Box<dyn Any + Send>) -> Vec<PluginAction> {
        if let Ok(archiver_event) = event.downcast::<ArchiverEvent>() {
            return self.handle_archiver_event(*archiver_event);
        }
        Vec::new()
    }

    fn commands(&self) -> Vec<PluginCommand> {
        vec![PluginCommand {
            name: "Archiver",
            description: "Channel archiver",
            key: KeyCode::Char('A'),
            modifiers: KeyModifiers::SHIFT,
        }]
    }

    fn panes(&self) -> Vec<PaneId> {
        vec![PANE_ID]
    }

    fn render_pane(
        &self,
        _pane_id: PaneId,
        frame: &mut Frame,
        area: Rect,
        app: &AppState,
    ) {
        render::render(self, frame, area, app);
    }

    fn status_line(&self, _app: &AppState) -> Option<String> {
        let active = self.jobs.iter().filter(|j| {
            j.state == ArchiveState::Downloading || j.state == ArchiveState::Scanning
        }).count();

        if active > 0 {
            Some(format!("AR:{active}"))
        } else {
            None
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
