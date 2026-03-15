use std::collections::HashMap;
use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::platform::{ChannelEntry, PlatformKind};
use crate::recording::job::{RecordingJob, RecordingState};
use crate::recording::RecordingCommand;

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
        job_id: Uuid,
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
    Notification {
        title: String,
        body: String,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Sidebar,
    Detail,
    RecordingList,
    Settings,
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
    pub first_run: bool,

    // Recording state
    pub recordings: HashMap<Uuid, RecordingJob>,
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
            first_run,
            recordings: HashMap::new(),
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

    pub fn handle_event(&mut self, event: AppEvent) -> Option<AppAction> {
        match event {
            AppEvent::Key(key) => return self.handle_key(key),
            AppEvent::Resize(_, _) => {}
            AppEvent::Tick => {}
            AppEvent::ChannelsUpdated(channels) => {
                // Preserve auto_record settings from config
                let mut updated = channels;
                for ch in &mut updated {
                    ch.auto_record = self.config.auto_record_channels.iter().any(|a| {
                        a.channel_id == ch.id && a.platform == ch.platform.to_string()
                    });
                }
                self.channels = updated;
                if self.selected_channel >= self.channels.len() {
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
            AppEvent::RecordingStarted { .. } => {
                self.status_message = "Recording started".to_string();
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
                let active = self.active_recording_count();
                self.status_message = format!("Recording finished ({active} active)");
            }
            AppEvent::ThumbnailReady { channel_id, path } => {
                self.thumbnail_cache.insert(channel_id.clone(), path.clone());
                // Load into protocol for rendering
                if let Some(ref mut picker) = self.picker {
                    if let Ok(img) = image::ImageReader::open(&path)
                        .and_then(|r| r.decode().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
                    {
                        let proto = picker.new_resize_protocol(img);
                        self.thumbnail_protocols.insert(channel_id, proto);
                    }
                }
            }
            AppEvent::Notification { title, body } => {
                return Some(AppAction::Notify { title, body });
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
                    // Stop all recordings then quit
                    if let Some(ref tx) = self.recording_tx {
                        let _ = tx.send(RecordingCommand::StopAll);
                    }
                    self.should_quit = true;
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
            _ => {}
        }

        match self.active_pane {
            ActivePane::Sidebar => self.handle_sidebar_key(key),
            ActivePane::Detail => self.handle_detail_key(key),
            ActivePane::RecordingList => self.handle_recording_list_key(key),
            ActivePane::Settings => self.handle_settings_key(key),
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
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if !self.channels.is_empty() {
                    self.active_pane = ActivePane::Detail;
                }
            }
            KeyCode::Char('L') => {
                self.active_pane = ActivePane::RecordingList;
            }
            KeyCode::Char('s') => {
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
                        return Some(AppAction::PlayFile {
                            path: rec.output_path.clone(),
                        });
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

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.active_pane = ActivePane::Sidebar;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.settings_selected = (self.settings_selected + 1) % 5;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.settings_selected = if self.settings_selected == 0 {
                    4
                } else {
                    self.settings_selected - 1
                };
            }
            _ => {}
        }
        None
    }

    pub fn selected_channel(&self) -> Option<&ChannelEntry> {
        self.channels.get(self.selected_channel)
    }

    pub fn register_recording(&mut self, job: RecordingJob) {
        self.recordings.insert(job.id, job);
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
    PlayFile {
        path: PathBuf,
    },
    Notify {
        title: String,
        body: String,
    },
}
