use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::platform::{ChannelEntry, PlatformKind};
use crate::recording::job::{RecordingJob, RecordingState};
use crate::recording::RecordingCommand;

#[allow(dead_code)]
pub enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Tick,
    ChannelsUpdated(Vec<ChannelEntry>),
    ChannelWentLive(ChannelEntry),
    ChannelWentOffline(ChannelEntry),
    StreamUrlResolved {
        channel_id: String,
        url: String,
    },
    RecordingStarted {
        job: RecordingJob,
    },
    RecordingProgress {
        job_id: Uuid,
        bytes_written: u64,
        duration_secs: f64,
    },
    RecordingFinished {
        job_id: Uuid,
    },
    ThumbnailReady {
        channel_id: String,
        path: PathBuf,
    },
    ThumbnailDecoded {
        channel_id: String,
        protocol: StatefulProtocol,
    },
    Notification {
        title: String,
        body: String,
    },
    WatchResolved {
        channel_name: String,
        stream_url: String,
    },
    WatchFailed {
        error: String,
    },
    AllRecordingsStopped,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Sidebar,
    Detail,
    RecordingList,
    Settings,
    Log,
    Wizard,
}

pub struct AppState {
    pub config: AppConfig,
    pub channels: Vec<ChannelEntry>,
    pub selected_channel: usize,
    pub active_pane: ActivePane,
    pub should_quit: bool,
    pub quit_confirm: bool,
    pub status_message: String,
    pub show_help: bool,
    // Recording state
    pub recordings: HashMap<Uuid, RecordingJob>,
    pub active_recording_channels: HashSet<String>, // channel_ids with active recordings
    pub selected_recording: usize,
    pub transcode_mode: bool,

    // Playback
    pub watching_channel: Option<String>,

    // Platform connection status
    pub twitch_connected: bool,
    pub youtube_connected: bool,

    // Thumbnail cache (channel_id -> protocol state for rendering)
    pub thumbnail_cache: HashMap<String, PathBuf>,
    pub thumbnail_protocols: HashMap<String, StatefulProtocol>,
    pub picker: Option<Picker>,

    // Sender for recording commands
    pub recording_tx: Option<tokio::sync::mpsc::UnboundedSender<RecordingCommand>>,

    // Settings edit state
    pub settings_selected: usize,

    // Log viewer state
    pub log_lines: Vec<String>,
    pub log_scroll: usize,
    pub log_auto_scroll: bool,
    pub log_path: PathBuf,

    // Sidebar display order: indices into app.channels in visual sort order
    pub sidebar_order: Vec<usize>,

    // Sidebar scroll offsets for autoscroll (channel index → text scroll offset)
    pub scroll_offsets: HashMap<usize, usize>,
    pub tick_counter: u64,
    // Track previously selected channel for resetting scroll offset
    prev_selected_channel: usize,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let first_run = config.twitch.is_none() && config.youtube.is_none();
        Self {
            config,
            channels: Vec::new(),
            selected_channel: 0,
            active_pane: if first_run {
                ActivePane::Wizard
            } else {
                ActivePane::Sidebar
            },
            should_quit: false,
            quit_confirm: false,
            status_message: String::new(),
            show_help: false,
            recordings: HashMap::new(),
            active_recording_channels: HashSet::new(),
            selected_recording: 0,
            transcode_mode: false,
            watching_channel: None,
            twitch_connected: false,
            youtube_connected: false,
            thumbnail_cache: HashMap::new(),
            thumbnail_protocols: HashMap::new(),
            picker: Picker::from_query_stdio().ok(),
            recording_tx: None,
            settings_selected: 0,
            log_lines: Vec::new(),
            log_scroll: 0,
            log_auto_scroll: true,
            log_path: AppConfig::state_dir().join("streavo.log"),
            sidebar_order: Vec::new(),
            scroll_offsets: HashMap::new(),
            tick_counter: 0,
            prev_selected_channel: 0,
        }
    }

    pub fn active_recording_count(&self) -> usize {
        self.recordings
            .values()
            .filter(|r| {
                r.state == RecordingState::Recording || r.state == RecordingState::ResolvingUrl
            })
            .count()
    }

    pub fn sorted_recordings(&self) -> Vec<&RecordingJob> {
        let mut recs: Vec<&RecordingJob> = self.recordings.values().collect();
        recs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        recs
    }

    /// Groups finished+active recordings by day label ("Today", "Yesterday", "March 12, 2026"), newest first
    pub fn recordings_by_day(&self) -> Vec<(String, Vec<&RecordingJob>)> {
        use chrono::Local;

        let mut recs = self.sorted_recordings();
        // sorted_recordings already sorts newest first
        if recs.is_empty() {
            return Vec::new();
        }

        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let mut groups: Vec<(String, Vec<&RecordingJob>)> = Vec::new();

        for rec in recs.drain(..) {
            let rec_date = rec.started_at.with_timezone(&Local).date_naive();
            let label = if rec_date == today {
                "Today".to_string()
            } else if rec_date == yesterday {
                "Yesterday".to_string()
            } else {
                rec_date.format("%B %-d, %Y").to_string()
            };

            if let Some(last) = groups.last_mut() {
                if last.0 == label {
                    last.1.push(rec);
                    continue;
                }
            }
            groups.push((label, vec![rec]));
        }

        groups
    }

    /// Counts finished recordings where `watched == false` for a given channel
    pub fn unwatched_count_for_channel(&self, channel_id: &str) -> usize {
        self.recordings
            .values()
            .filter(|r| {
                r.channel_id == channel_id
                    && r.state == RecordingState::Finished
                    && !r.watched
            })
            .count()
    }

    pub fn handle_event(&mut self, event: AppEvent) -> Option<AppAction> {
        match event {
            AppEvent::Key(key) => return self.handle_key(key),
            AppEvent::Resize(_, _) => {}
            AppEvent::Tick => {
                self.tick_counter = self.tick_counter.wrapping_add(1);
                // Reset scroll offset when selection changes
                if self.selected_channel != self.prev_selected_channel {
                    self.scroll_offsets.remove(&self.prev_selected_channel);
                    self.prev_selected_channel = self.selected_channel;
                }
                // Autoscroll stream title for selected live channel in sidebar
                if self.active_pane == ActivePane::Sidebar {
                    if let Some(ch) = self.channels.get(self.selected_channel) {
                        if ch.is_live && ch.stream_title.is_some() {
                            if self.tick_counter % 6 == 0 {
                                let offset = self.scroll_offsets.entry(self.selected_channel).or_insert(0);
                                *offset += 1;
                            }
                        }
                    }
                }
            }
            AppEvent::ChannelsUpdated(channels) => {
                // Remember currently selected channel by ID
                let prev_selected_id = self.channels.get(self.selected_channel).map(|ch| ch.id.clone());

                // Preserve auto_record settings from config
                let mut updated = channels;
                for ch in &mut updated {
                    ch.auto_record = self.config.auto_record_channels.iter().any(|a| {
                        a.channel_id == ch.id && a.platform == ch.platform.to_string()
                    });
                }
                self.channels = updated;

                // Restore selection by ID, fall back to clamping
                if let Some(ref prev_id) = prev_selected_id {
                    if let Some(new_idx) = self.channels.iter().position(|ch| &ch.id == prev_id) {
                        self.selected_channel = new_idx;
                    } else if self.selected_channel >= self.channels.len() {
                        self.selected_channel = self.channels.len().saturating_sub(1);
                    }
                } else if self.selected_channel >= self.channels.len() {
                    self.selected_channel = self.channels.len().saturating_sub(1);
                }
            }
            AppEvent::ChannelWentLive(ref channel) => {
                self.status_message = format!("{} went live!", channel.display_name);
                return Some(AppAction::Notify {
                    title: format!("{} is live!", channel.display_name),
                    body: channel
                        .stream_title
                        .clone()
                        .unwrap_or_else(|| "Stream started".to_string()),
                });
            }
            AppEvent::ChannelWentOffline(channel) => {
                self.status_message = format!("{} went offline", channel.display_name);
            }
            AppEvent::StreamUrlResolved { channel_id, .. } => {
                self.status_message = format!("Stream URL resolved for {channel_id}");
            }
            AppEvent::RecordingStarted { job } => {
                self.status_message = format!("Recording started: {}", job.channel_name);
                // Manager is single source of truth — always register its job
                self.recordings.insert(job.id, job);
                self.rebuild_active_channels();
            }
            AppEvent::RecordingProgress {
                job_id,
                bytes_written,
                duration_secs,
            } => {
                if let Some(job) = self.recordings.get_mut(&job_id) {
                    job.bytes_written = bytes_written;
                    job.duration_secs = duration_secs;
                    job.state = RecordingState::Recording;
                }
            }
            AppEvent::RecordingFinished { job_id } => {
                if let Some(job) = self.recordings.get_mut(&job_id) {
                    if job.state != RecordingState::Failed {
                        job.state = RecordingState::Finished;
                    }
                }
                self.rebuild_active_channels();
                let active = self.active_recording_count();
                self.status_message = format!("Recording finished ({active} active)");
            }
            AppEvent::ThumbnailReady { channel_id, path } => {
                self.thumbnail_cache.insert(channel_id.clone(), path);
            }
            AppEvent::ThumbnailDecoded { channel_id, protocol } => {
                self.thumbnail_protocols.insert(channel_id, protocol);
            }
            AppEvent::Notification { title, body } => {
                return Some(AppAction::Notify { title, body });
            }
            AppEvent::WatchResolved { channel_name, stream_url } => {
                self.watching_channel = Some(channel_name.clone());
                return Some(AppAction::LaunchMpv {
                    channel_name,
                    url: stream_url,
                });
            }
            AppEvent::WatchFailed { error } => {
                self.status_message = format!("Watch failed: {error}");
            }
            AppEvent::AllRecordingsStopped => {
                // All recordings have been stopped — safe to quit now
                self.should_quit = true;
            }
            AppEvent::Error(msg) => {
                self.status_message = format!("Error: {msg}");
            }
        }
        None
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Option<AppAction> {
        use crossterm::event::{KeyCode, KeyEventKind};

        if key.kind != KeyEventKind::Press {
            return None;
        }

        // Quit confirmation handling
        if self.quit_confirm {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    // Stop all recordings — quit will happen when AllRecordingsStopped arrives
                    if let Some(ref tx) = self.recording_tx {
                        let _ = tx.send(RecordingCommand::StopAll);
                    }
                    self.status_message = "Stopping recordings...".to_string();
                    self.quit_confirm = false;
                    return None;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.quit_confirm = false;
                    self.status_message.clear();
                    return None;
                }
                _ => return None,
            }
        }

        // Global keys
        match key.code {
            KeyCode::Char('q') if self.active_pane != ActivePane::Wizard => {
                if self.active_recording_count() > 0 {
                    self.quit_confirm = true;
                    self.status_message =
                        "Recordings active! Quit? (y/n)".to_string();
                } else {
                    self.should_quit = true;
                }
                return None;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                return None;
            }
            KeyCode::Esc if self.show_help => {
                self.show_help = false;
                return None;
            }
            KeyCode::Esc if self.active_pane == ActivePane::Wizard => {
                self.active_pane = ActivePane::Sidebar;
                return None;
            }
            KeyCode::Char('F') if self.active_pane != ActivePane::Wizard && self.active_pane != ActivePane::Log => {
                self.refresh_log();
                self.active_pane = ActivePane::Log;
                return None;
            }
            _ => {}
        }

        match self.active_pane {
            ActivePane::Sidebar => self.handle_sidebar_key(key),
            ActivePane::Detail => self.handle_detail_key(key),
            ActivePane::RecordingList => self.handle_recording_list_key(key),
            ActivePane::Settings => self.handle_settings_key(key),
            ActivePane::Log => self.handle_log_key(key),
            ActivePane::Wizard => None,
        }
    }

    fn handle_sidebar_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Option<AppAction> {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.sidebar_order.is_empty() {
                    // Navigate in sidebar display order
                    let cur_pos = self.sidebar_order.iter().position(|&i| i == self.selected_channel).unwrap_or(0);
                    let next_pos = (cur_pos + 1) % self.sidebar_order.len();
                    self.selected_channel = self.sidebar_order[next_pos];
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.sidebar_order.is_empty() {
                    let cur_pos = self.sidebar_order.iter().position(|&i| i == self.selected_channel).unwrap_or(0);
                    let next_pos = if cur_pos == 0 {
                        self.sidebar_order.len() - 1
                    } else {
                        cur_pos - 1
                    };
                    self.selected_channel = self.sidebar_order[next_pos];
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if !self.channels.is_empty() {
                    self.active_pane = ActivePane::Detail;
                }
            }
            KeyCode::Char('L') => {
                self.active_pane = ActivePane::RecordingList;
            }
            KeyCode::Char('s') | KeyCode::Char('C') => {
                self.active_pane = ActivePane::Settings;
            }
            _ => {}
        }
        None
    }

    fn handle_detail_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Option<AppAction> {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.active_pane = ActivePane::Sidebar;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.channels.is_empty() {
                    self.selected_channel = (self.selected_channel + 1) % self.channels.len();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.channels.is_empty() {
                    self.selected_channel = if self.selected_channel == 0 {
                        self.channels.len() - 1
                    } else {
                        self.selected_channel - 1
                    };
                }
            }
            KeyCode::Char('r') => {
                // Start recording
                if let Some(ch) = self.selected_channel() {
                    if !ch.is_live {
                        self.status_message = "Channel is not live".to_string();
                        return None;
                    }
                    // Check if already recording this channel
                    if self.is_channel_recording(&ch.id) {
                        self.status_message = format!("Already recording {}", ch.display_name);
                        return None;
                    }
                    let ch = ch.clone();
                    return Some(AppAction::StartRecording {
                        channel_id: ch.id,
                        channel_name: ch.name,
                        platform: ch.platform,
                        transcode: self.transcode_mode,
                    });
                }
            }
            KeyCode::Char('w') => {
                // Watch in mpv
                if let Some(ch) = self.selected_channel() {
                    if !ch.is_live {
                        self.status_message = "Channel is not live".to_string();
                        return None;
                    }
                    let ch = ch.clone();
                    self.watching_channel = Some(ch.name.clone());
                    return Some(AppAction::Watch {
                        channel_name: ch.name,
                        platform: ch.platform,
                    });
                }
            }
            KeyCode::Char('a') => {
                // Toggle auto-record
                if let Some(idx) = self.channels.get(self.selected_channel).map(|_| self.selected_channel) {
                    let ch = &mut self.channels[idx];
                    ch.auto_record = !ch.auto_record;

                    if ch.auto_record {
                        self.config.auto_record_channels.push(
                            crate::config::AutoRecordEntry {
                                platform: ch.platform.to_string(),
                                channel_id: ch.id.clone(),
                                channel_name: ch.name.clone(),
                            },
                        );
                        self.status_message =
                            format!("Auto-record ON for {}", ch.display_name);
                    } else {
                        self.config.auto_record_channels.retain(|a| {
                            !(a.channel_id == ch.id
                                && a.platform == ch.platform.to_string())
                        });
                        self.status_message =
                            format!("Auto-record OFF for {}", ch.display_name);
                    }

                    // Persist config
                    if let Err(e) = self.config.save(None) {
                        self.status_message = format!("Failed to save config: {e}");
                    }
                }
            }
            KeyCode::Char('t') => {
                self.transcode_mode = !self.transcode_mode;
                self.status_message = if self.transcode_mode {
                    "Transcode mode: ON (NVENC)".to_string()
                } else {
                    "Transcode mode: OFF (passthrough)".to_string()
                };
            }
            _ => {}
        }
        None
    }

    fn handle_recording_list_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Option<AppAction> {
        use crossterm::event::KeyCode;

        let recordings = self.sorted_recordings();
        let count = recordings.len();

        // Clamp selection to valid range
        if count > 0 {
            self.selected_recording = self.selected_recording.min(count - 1);
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.active_pane = ActivePane::Sidebar;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if count > 0 {
                    self.selected_recording = (self.selected_recording + 1) % count;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if count > 0 {
                    self.selected_recording = if self.selected_recording == 0 {
                        count - 1
                    } else {
                        self.selected_recording - 1
                    };
                }
            }
            KeyCode::Char('s') => {
                // Stop selected recording
                let recs = self.sorted_recordings();
                if let Some(rec) = recs.get(self.selected_recording) {
                    if rec.state == RecordingState::Recording
                        || rec.state == RecordingState::ResolvingUrl
                    {
                        let job_id = rec.id;
                        if let Some(ref tx) = self.recording_tx {
                            let _ = tx.send(RecordingCommand::Stop { job_id });
                        }
                        self.status_message = format!("Stopping recording: {}", rec.channel_name);
                    }
                }
            }
            KeyCode::Char('p') => {
                // Play finished recording
                let recs = self.sorted_recordings();
                if let Some(rec) = recs.get(self.selected_recording) {
                    if rec.state == RecordingState::Finished {
                        let job_id = rec.id;
                        let path = rec.output_path.clone();
                        // Mark as watched
                        if let Some(job) = self.recordings.get_mut(&job_id) {
                            job.watched = true;
                        }
                        return Some(AppAction::PlayFile { path });
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn handle_settings_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Option<AppAction> {
        use crossterm::event::KeyCode;

        const SETTINGS_COUNT: usize = 5;
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.active_pane = ActivePane::Sidebar;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.settings_selected = (self.settings_selected + 1) % SETTINGS_COUNT;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.settings_selected = if self.settings_selected == 0 {
                    SETTINGS_COUNT - 1
                } else {
                    self.settings_selected - 1
                };
            }
            _ => {}
        }
        None
    }

    fn handle_log_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Option<AppAction> {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.active_pane = ActivePane::Sidebar;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.log_scroll + 1 < self.log_lines.len() {
                    self.log_scroll += 1;
                    self.log_auto_scroll = false;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
                self.log_auto_scroll = false;
            }
            KeyCode::Char('G') => {
                // Jump to bottom, re-enable auto-scroll
                self.log_scroll = self.log_lines.len().saturating_sub(1);
                self.log_auto_scroll = true;
            }
            KeyCode::Char('g') => {
                // Jump to top
                self.log_scroll = 0;
                self.log_auto_scroll = false;
            }
            KeyCode::PageDown => {
                self.log_scroll = (self.log_scroll + 30).min(
                    self.log_lines.len().saturating_sub(1),
                );
                self.log_auto_scroll = false;
            }
            KeyCode::PageUp => {
                self.log_scroll = self.log_scroll.saturating_sub(30);
                self.log_auto_scroll = false;
            }
            KeyCode::Char('c') => {
                // Clear log
                std::fs::write(&self.log_path, "").ok();
                self.log_lines.clear();
                self.log_scroll = 0;
                self.status_message = "Log cleared".to_string();
            }
            _ => {}
        }
        None
    }

    pub fn refresh_log(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.log_path) {
            self.log_lines = content.lines().map(|l| l.to_string()).collect();
            if self.log_auto_scroll {
                self.log_scroll = self.log_lines.len().saturating_sub(1);
            }
        }
    }

    pub fn selected_channel(&self) -> Option<&ChannelEntry> {
        self.channels.get(self.selected_channel)
    }

    /// Rebuild the set of channel_ids with active recordings
    fn rebuild_active_channels(&mut self) {
        self.active_recording_channels = self
            .recordings
            .values()
            .filter(|r| matches!(r.state, RecordingState::Recording | RecordingState::ResolvingUrl))
            .map(|r| r.channel_id.clone())
            .collect();
    }

    /// Check if a channel has an active recording
    pub fn is_channel_recording(&self, channel_id: &str) -> bool {
        self.active_recording_channels.contains(channel_id)
    }
}

/// Actions the TUI loop should execute in response to key events
pub enum AppAction {
    StartRecording {
        channel_id: String,
        channel_name: String,
        platform: PlatformKind,
        transcode: bool,
    },
    Watch {
        channel_name: String,
        platform: PlatformKind,
    },
    LaunchMpv {
        channel_name: String,
        url: String,
    },
    PlayFile {
        path: PathBuf,
    },
    Notify {
        title: String,
        body: String,
    },
}
