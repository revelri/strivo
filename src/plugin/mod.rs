pub mod archiver;
pub mod crunchr;
pub mod registry;

use std::any::Any;
use std::path::PathBuf;
use std::pin::Pin;
use std::future::Future;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::{AppState, DaemonEvent};
use crate::config::AppConfig;

/// Unique identifier for a plugin-contributed pane.
pub type PaneId = &'static str;

/// A command that a plugin registers (for global keybinding + help overlay).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PluginCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Actions a plugin can request the host to perform.
#[allow(dead_code)]
pub enum PluginAction {
    /// Update the status bar message.
    SetStatus(String),
    /// Send a desktop notification.
    Notify { title: String, body: String },
    /// Navigate to this plugin's pane.
    ActivatePane(PaneId),
    /// Navigate back to sidebar (deactivate plugin pane).
    NavigateBack,
    /// Spawn an async task; results delivered back via on_plugin_event.
    SpawnTask {
        plugin_name: &'static str,
        future: Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>>,
    },
    /// Play a file in mpv.
    PlayFile(PathBuf),
}

/// Context provided to plugins during initialization.
pub struct PluginContext<'a> {
    pub config: &'a AppConfig,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

/// Fieldless mirror of DaemonEvent for event filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonEventKind {
    ChannelsUpdated,
    ChannelWentLive,
    ChannelWentOffline,
    StreamUrlResolved,
    RecordingStarted,
    RecordingProgress,
    RecordingFinished,
    Notification,
    AllRecordingsStopped,
    DeviceCodeRequired,
    PlatformAuthenticated,
    PatreonPostFound,
    Error,
}

impl DaemonEventKind {
    pub fn from_event(event: &DaemonEvent) -> Self {
        match event {
            DaemonEvent::ChannelsUpdated(_) => Self::ChannelsUpdated,
            DaemonEvent::ChannelWentLive(_) => Self::ChannelWentLive,
            DaemonEvent::ChannelWentOffline(_) => Self::ChannelWentOffline,
            DaemonEvent::StreamUrlResolved { .. } => Self::StreamUrlResolved,
            DaemonEvent::RecordingStarted { .. } => Self::RecordingStarted,
            DaemonEvent::RecordingProgress { .. } => Self::RecordingProgress,
            DaemonEvent::RecordingFinished { .. } => Self::RecordingFinished,
            DaemonEvent::Notification { .. } => Self::Notification,
            DaemonEvent::AllRecordingsStopped => Self::AllRecordingsStopped,
            DaemonEvent::DeviceCodeRequired { .. } => Self::DeviceCodeRequired,
            DaemonEvent::PlatformAuthenticated { .. } => Self::PlatformAuthenticated,
            DaemonEvent::PatreonPostFound { .. } => Self::PatreonPostFound,
            DaemonEvent::Error(_) => Self::Error,
        }
    }
}

/// The core Plugin trait. All plugins implement this.
#[allow(dead_code, unused)]
pub trait Plugin: Send {
    /// Unique name for this plugin (e.g., "crunchr").
    fn name(&self) -> &'static str;

    /// Human-readable display name.
    fn display_name(&self) -> &str;

    /// Called once after registration.
    fn init(&mut self, ctx: &PluginContext) -> anyhow::Result<()>;

    /// Called on shutdown.
    fn shutdown(&mut self) {}

    /// Which daemon events this plugin wants to receive. None = all.
    fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
        None
    }

    /// Handle a daemon event. Return actions for the host to execute.
    fn on_event(&mut self, _event: &DaemonEvent, _app: &AppState) -> Vec<PluginAction> {
        Vec::new()
    }

    /// Handle a keyboard event when this plugin's pane is active.
    fn on_key(&mut self, _key: KeyEvent, _app: &AppState) -> Vec<PluginAction> {
        Vec::new()
    }

    /// Handle events from the plugin's own async tasks.
    fn on_plugin_event(&mut self, _event: Box<dyn Any + Send>) -> Vec<PluginAction> {
        Vec::new()
    }

    /// Commands this plugin contributes (for help overlay and keybinding dispatch).
    fn commands(&self) -> Vec<PluginCommand> {
        Vec::new()
    }

    /// Pane IDs this plugin contributes.
    fn panes(&self) -> Vec<PaneId> {
        Vec::new()
    }

    /// Render this plugin's pane.
    fn render_pane(
        &self,
        _pane_id: PaneId,
        _frame: &mut Frame,
        _area: Rect,
        _app: &AppState,
    ) {
    }

    /// Optional: contribute a segment to the status bar.
    fn status_line(&self, _app: &AppState) -> Option<String> {
        None
    }

    /// Downcast support.
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
