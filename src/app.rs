use crate::config::AppConfig;
use crate::platform::ChannelEntry;

#[derive(Debug)]
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
        job_id: uuid::Uuid,
    },
    RecordingProgress {
        job_id: uuid::Uuid,
        bytes_written: u64,
        duration_secs: f64,
    },
    RecordingFinished {
        job_id: uuid::Uuid,
    },
    ThumbnailReady {
        channel_id: String,
        path: std::path::PathBuf,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Sidebar,
    Detail,
    RecordingList,
    Settings,
    Help,
    Wizard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
}

pub struct AppState {
    pub config: AppConfig,
    pub channels: Vec<ChannelEntry>,
    pub selected_channel: usize,
    pub active_pane: ActivePane,
    pub input_mode: InputMode,
    pub should_quit: bool,
    pub status_message: String,
    pub show_help: bool,
    pub first_run: bool,
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
            input_mode: InputMode::Normal,
            should_quit: false,
            status_message: String::new(),
            show_help: false,
            first_run,
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize(_, _) => {}
            AppEvent::Tick => {}
            AppEvent::ChannelsUpdated(channels) => {
                self.channels = channels;
                if self.selected_channel >= self.channels.len() {
                    self.selected_channel = self.channels.len().saturating_sub(1);
                }
            }
            AppEvent::ChannelWentLive(channel) => {
                self.status_message = format!("{} went live!", channel.display_name);
            }
            AppEvent::ChannelWentOffline(channel) => {
                self.status_message = format!("{} went offline", channel.display_name);
            }
            AppEvent::Error(msg) => {
                self.status_message = format!("Error: {msg}");
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyEventKind};

        if key.kind != KeyEventKind::Press {
            return;
        }

        // Global keys
        match key.code {
            KeyCode::Char('q') if self.active_pane != ActivePane::Wizard => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                return;
            }
            KeyCode::Esc if self.show_help => {
                self.show_help = false;
                return;
            }
            KeyCode::Esc if self.active_pane == ActivePane::Wizard => {
                self.active_pane = ActivePane::Sidebar;
                return;
            }
            _ => {}
        }

        match self.active_pane {
            ActivePane::Sidebar => self.handle_sidebar_key(key),
            ActivePane::Detail => self.handle_detail_key(key),
            ActivePane::Wizard => {} // handled by wizard widget
            _ => {}
        }
    }

    fn handle_sidebar_key(&mut self, key: crossterm::event::KeyEvent) {
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
            KeyCode::Enter => {
                if !self.channels.is_empty() {
                    self.active_pane = ActivePane::Detail;
                }
            }
            KeyCode::Char('l') => {
                self.active_pane = ActivePane::RecordingList;
            }
            KeyCode::Char('s') => {
                self.active_pane = ActivePane::Settings;
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.active_pane = ActivePane::Sidebar;
            }
            _ => {}
        }
    }

    pub fn selected_channel(&self) -> Option<&ChannelEntry> {
        self.channels.get(self.selected_channel)
    }
}
