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

use std::collections::HashSet;

use crate::app::{AppState, DaemonEvent};
use crate::config::ArchiverConfig;
use crate::platform::ChannelEntry;
use crate::recording::job::RecordingState;

use super::{
    DaemonEventKind, PaneId, Plugin, PluginAction, PluginCommand, PluginContext,
};
use types::{
    ArchiveJob, ArchiveState, ArchiverEvent, ArchiverView, ConfigModalState,
    PickerState, RecordingFilter,
};

pub const PANE_ID: PaneId = "archiver";

/// Number of static config fields in the Archiver config modal (before channel checklist).
/// Fields: enabled, archive_dir, format, concurrent_fragments, rate_limit = 5 fields.
/// Channel checkboxes start at index ARCHIVER_STATIC_FIELDS (i.e., after index 4).
const ARCHIVER_STATIC_FIELDS: usize = 5;

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

    // --- New: config modal, picker, tandem ---
    /// Whether the plugin is enabled for tandem auto-processing.
    pub enabled: bool,
    /// Whether the first-run config has been completed.
    pub configured: bool,
    /// Tandem channels.
    pub tandem_channels: Vec<String>,
    /// Tandem playlists.
    pub tandem_playlists: Vec<String>,
    /// Config modal state.
    pub config_modal: ConfigModalState,
    /// Draft config being edited in the modal.
    pub config_draft: Option<ArchiverConfig>,
    /// Recording picker state.
    pub picker: PickerState,
    /// Cached channel list for config modal tandem checkboxes.
    pub cached_channels: Vec<(String, String)>,
}

impl Default for ArchiverPlugin {
    fn default() -> Self {
        Self::new()
    }
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
            enabled: false,
            configured: false,
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
            config_modal: ConfigModalState::Hidden,
            config_draft: None,
            picker: PickerState::default(),
            cached_channels: Vec::new(),
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

    /// Open the config modal, cloning current config into draft.
    fn open_config_modal(&mut self, app: &AppState) {
        self.cached_channels = app.channels.iter().map(|ch| {
            let key = format!("{}:{}", ch.platform, ch.id);
            let display = format!("[{}] {}", ch.platform, ch.display_name);
            (key, display)
        }).collect();

        let draft = self.config.clone().unwrap_or_default();
        self.config_draft = Some(draft);

        self.config_modal = ConfigModalState::Active {
            selected_field: 0,
            editing: false,
            static_field_count: ARCHIVER_STATIC_FIELDS,
        };
    }

    /// Handle keys while config modal is active.
    fn handle_config_modal_key(&mut self, key: KeyEvent, _app: &AppState) -> Vec<PluginAction> {
        let ConfigModalState::Active { ref mut selected_field, ref mut editing, static_field_count } = self.config_modal else {
            return Vec::new();
        };
        let total_fields = static_field_count + self.cached_channels.len();
        let save_idx = total_fields;

        // Indices: 0=enabled, 1=archive_dir, 2=format, 3=concurrent_fragments, 4..4+N=tandem channels, last=[Save]
        // Actually: 0=enabled, 1=archive_dir, 2=format, 3=concurrent_fragments, 4=rate_limit, then channels, then save
        // static_field_count = 5 means indices 0..4 are static fields

        if *editing {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => { *editing = false; }
                KeyCode::Backspace => {
                    if let Some(ref mut draft) = self.config_draft {
                        match *selected_field {
                            1 => { let s = draft.archive_dir.to_string_lossy().to_string(); if !s.is_empty() { draft.archive_dir = PathBuf::from(&s[..s.len().saturating_sub(1)]); } }
                            2 => { draft.format.pop(); }
                            3 => { /* number field - no backspace */ }
                            4 => { draft.rate_limit.pop(); }
                            _ => {}
                        }
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut draft) = self.config_draft {
                        match *selected_field {
                            1 => { let mut s = draft.archive_dir.to_string_lossy().to_string(); s.push(c); draft.archive_dir = PathBuf::from(s); }
                            2 => draft.format.push(c),
                            3 => {
                                if let Some(d) = c.to_digit(10) {
                                    draft.concurrent_fragments = draft.concurrent_fragments * 10 + d;
                                }
                            }
                            4 => draft.rate_limit.push(c),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return Vec::new();
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                *selected_field = (*selected_field + 1).min(save_idx);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                *selected_field = selected_field.saturating_sub(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if *selected_field == save_idx {
                    // Save
                    if let Some(mut draft) = self.config_draft.take() {
                        draft.configured = true;
                        self.enabled = draft.enabled;
                        self.configured = true;
                        self.tandem_channels = draft.tandem_channels.clone();
                        self.tandem_playlists = draft.tandem_playlists.clone();
                        self.config = Some(draft.clone());
                        self.config_modal = ConfigModalState::Hidden;
                        return vec![PluginAction::UpdateConfig {
                            plugin_name: "archiver",
                            config_update: Box::new(draft),
                        }];
                    }
                    self.config_modal = ConfigModalState::Hidden;
                } else if *selected_field == 0 {
                    // Toggle enabled
                    if let Some(ref mut draft) = self.config_draft {
                        draft.enabled = !draft.enabled;
                    }
                } else if *selected_field >= ARCHIVER_STATIC_FIELDS && *selected_field < save_idx {
                    // Tandem channel toggle
                    let ch_idx = *selected_field - ARCHIVER_STATIC_FIELDS;
                    if let Some(ref mut draft) = self.config_draft {
                        if let Some((key, _)) = self.cached_channels.get(ch_idx) {
                            if draft.tandem_channels.contains(key) {
                                draft.tandem_channels.retain(|k| k != key);
                            } else {
                                draft.tandem_channels.push(key.clone());
                                draft.enabled = true;
                            }
                        }
                    }
                } else if matches!(*selected_field, 1 | 2 | 3 | 4) {
                    *editing = true;
                    // Reset concurrent_fragments for re-entry when editing
                    if *selected_field == 3 {
                        if let Some(ref mut draft) = self.config_draft {
                            draft.concurrent_fragments = 0;
                        }
                    }
                }
            }
            KeyCode::Esc => {
                self.config_modal = ConfigModalState::Hidden;
                self.config_draft = None;
            }
            _ => {}
        }
        Vec::new()
    }

    /// Handle keys in the RecordingPicker view.
    fn handle_picker_key(&mut self, key: KeyEvent, app: &AppState) -> Vec<PluginAction> {
        self.refresh_picker_list(app);

        match key.code {
            KeyCode::Tab => { self.view = ArchiverView::ChannelList; }
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.picker.visible_ids.is_empty() {
                    self.picker.selected = (self.picker.selected + 1) % self.picker.visible_ids.len();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.picker.visible_ids.is_empty() {
                    self.picker.selected = if self.picker.selected == 0 {
                        self.picker.visible_ids.len() - 1
                    } else {
                        self.picker.selected - 1
                    };
                }
            }
            KeyCode::Char(' ') => {
                if let Some(&id) = self.picker.visible_ids.get(self.picker.selected) {
                    if !self.picker.selections.remove(&id) {
                        self.picker.selections.insert(id);
                    }
                }
            }
            KeyCode::Char('a') => {
                for &id in &self.picker.visible_ids {
                    self.picker.selections.insert(id);
                }
            }
            KeyCode::Char('f') => {
                self.cycle_picker_filter(app);
                self.refresh_picker_list(app);
            }
            KeyCode::Enter => {
                return self.process_selected_recordings(app);
            }
            KeyCode::Esc => {
                self.picker.selections.clear();
                self.view = ArchiverView::ChannelList;
            }
            _ => {}
        }
        Vec::new()
    }

    fn refresh_picker_list(&mut self, app: &AppState) {
        let finished: Vec<_> = app.recordings.values()
            .filter(|r| r.state == RecordingState::Finished)
            .filter(|r| match &self.picker.filter {
                RecordingFilter::All => true,
                RecordingFilter::ByChannel(ch) => {
                    let key = format!("{}:{}", r.platform, r.channel_id);
                    key == *ch
                }
                RecordingFilter::ByPlaylist(pl) => {
                    r.playlist.as_deref() == Some(pl.as_str())
                }
            })
            .collect();

        self.picker.visible_ids = finished.iter().map(|r| r.id).collect();
        if self.picker.selected >= self.picker.visible_ids.len() {
            self.picker.selected = self.picker.visible_ids.len().saturating_sub(1);
        }
    }

    fn cycle_picker_filter(&mut self, app: &AppState) {
        let channels: Vec<String> = {
            let mut seen = HashSet::new();
            app.recordings.values()
                .filter(|r| r.state == RecordingState::Finished)
                .filter_map(|r| {
                    let key = format!("{}:{}", r.platform, r.channel_id);
                    if seen.insert(key.clone()) { Some(key) } else { None }
                })
                .collect()
        };

        self.picker.filter = match &self.picker.filter {
            RecordingFilter::All => {
                if let Some(ch) = channels.first() {
                    RecordingFilter::ByChannel(ch.clone())
                } else {
                    RecordingFilter::All
                }
            }
            RecordingFilter::ByChannel(current) => {
                let idx = channels.iter().position(|c| c == current).unwrap_or(0);
                if idx + 1 < channels.len() {
                    RecordingFilter::ByChannel(channels[idx + 1].clone())
                } else {
                    RecordingFilter::All
                }
            }
            RecordingFilter::ByPlaylist(_) => RecordingFilter::All,
        };
        self.picker.selections.clear();
    }

    fn process_selected_recordings(&mut self, app: &AppState) -> Vec<PluginAction> {
        let ids: Vec<uuid::Uuid> = if self.picker.selections.is_empty() {
            self.picker.visible_ids.get(self.picker.selected).copied().into_iter().collect()
        } else {
            self.picker.selections.drain().collect()
        };

        // Clone matching channels to avoid borrow conflict with self.start_archive()
        let channel_matches: Vec<_> = ids.iter()
            .filter_map(|id| app.recordings.get(id))
            .filter_map(|rec| {
                self.channels.iter().find(|c| c.id == rec.channel_id).cloned()
            })
            .collect();

        let mut actions = Vec::new();
        for channel in &channel_matches {
            let mut batch = self.start_archive(channel);
            actions.append(&mut batch);
        }
        if !actions.is_empty() {
            self.view = ArchiverView::ArchiveQueue;
        }
        actions
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

        // Load tandem / enabled state from config
        self.enabled = ctx.config.archiver.enabled;
        self.configured = ctx.config.archiver.configured;
        self.tandem_channels = ctx.config.archiver.tandem_channels.clone();
        self.tandem_playlists = ctx.config.archiver.tandem_playlists.clone();

        // Ensure archive directory exists
        if let Some(ref config) = self.config {
            let _ = std::fs::create_dir_all(&config.archive_dir);
        }

        tracing::info!("Archiver plugin initialized (enabled: {}, configured: {}, tandem_channels: {}, db: {})",
            self.enabled, self.configured, self.tandem_channels.len(), db_path.display());
        Ok(())
    }

    fn shutdown(&mut self) {
        self.db.take();
        tracing::info!("Archiver plugin shutting down");
    }

    fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
        Some(vec![
            DaemonEventKind::ChannelsUpdated,
            DaemonEventKind::RecordingFinished,
        ])
    }

    fn on_event(&mut self, event: &DaemonEvent, app: &AppState) -> Vec<PluginAction> {
        match event {
            DaemonEvent::ChannelsUpdated(channels) => {
                self.channels = channels.clone();
            }
            DaemonEvent::RecordingFinished { job_id, final_state, .. } => {
                if *final_state != RecordingState::Finished || !self.enabled {
                    return Vec::new();
                }
                if let Some(rec) = app.recordings.get(job_id) {
                    let channel_key = format!("{}:{}", rec.platform, rec.channel_id);
                    let is_tandem = self.tandem_channels.contains(&channel_key)
                        || rec.playlist.as_ref().is_some_and(|p| self.tandem_playlists.contains(p));

                    if is_tandem {
                        let channel = self.channels.iter().find(|c| c.id == rec.channel_id).cloned();
                        if let Some(channel) = channel {
                            return self.start_archive(&channel);
                        }
                    }
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, app: &AppState) -> Vec<PluginAction> {
        // First-run: auto-open config modal if not configured
        if !self.configured && self.config_modal == ConfigModalState::Hidden {
            self.open_config_modal(app);
        }

        // Config modal intercepts all keys when active
        if self.config_modal != ConfigModalState::Hidden {
            return self.handle_config_modal_key(key, app);
        }

        // RecordingPicker view
        if self.view == ArchiverView::RecordingPicker {
            return self.handle_picker_key(key, app);
        }

        // Channel list / queue views
        match key.code {
            KeyCode::Tab => {
                self.view = match self.view {
                    ArchiverView::ChannelList => ArchiverView::ArchiveQueue,
                    ArchiverView::ArchiveQueue => ArchiverView::RecordingPicker,
                    ArchiverView::RecordingPicker => ArchiverView::ChannelList,
                };
            }
            KeyCode::Char('c') => {
                self.open_config_modal(app);
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
                    _ => {}
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
                    _ => {}
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
        match self.view {
            ArchiverView::RecordingPicker => render::render_recording_picker(self, frame, area, app),
            _ => render::render(self, frame, area, app),
        }

        // Overlay config modal if active
        if self.config_modal != ConfigModalState::Hidden {
            render::render_config_modal(self, frame, area);
        }
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
