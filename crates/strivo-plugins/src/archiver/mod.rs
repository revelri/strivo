//! Archiver — channel back-catalog organization + auto-archive trigger.
//!
//! Watches `DaemonEvent::ChannelsUpdated` / `RecordingFinished` to
//! drive tandem auto-archives. The TUI picker pane retired with the
//! TUI deletion; the webui surfaces the same archive state directly
//! from `archiver.db`.

use std::any::Any;
use std::path::PathBuf;

use strivo_core::events::DaemonEvent;
use strivo_core::platform::ChannelEntry;
use strivo_core::plugin::{
    DaemonEventKind, Plugin, PluginAction, PluginContext, StatusSlot,
};
use strivo_core::recording::job::RecordingState;

pub mod db;
pub mod downloader;
pub mod scanner;
pub mod templates;
pub mod types;

pub struct ArchiverPlugin {
    data_dir: PathBuf,
    db_path: PathBuf,
    channels: Vec<ChannelEntry>,
    tandem_channels: Vec<String>,
    tandem_playlists: Vec<String>,
    enabled: bool,
}

impl Default for ArchiverPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchiverPlugin {
    pub fn new() -> Self {
        Self {
            data_dir: PathBuf::new(),
            db_path: PathBuf::new(),
            channels: Vec::new(),
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
            enabled: true,
        }
    }

    fn start_archive(&mut self, _channel: &ChannelEntry) -> Vec<PluginAction> {
        // Headless trigger hook — the webui dispatches the actual
        // archive job via PluginRpc verbs in the post-TUI world.
        Vec::new()
    }
}

impl Plugin for ArchiverPlugin {
    fn name(&self) -> &'static str { "archiver" }
    fn display_name(&self) -> &str { "Archiver" }

    fn init(&mut self, ctx: &PluginContext) -> anyhow::Result<()> {
        self.data_dir = ctx.data_dir.join("plugins").join("archiver");
        std::fs::create_dir_all(&self.data_dir)?;
        self.db_path = self.data_dir.join("archiver.db");
        Ok(())
    }

    fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
        Some(vec![
            DaemonEventKind::ChannelsUpdated,
            DaemonEventKind::RecordingFinished,
            DaemonEventKind::ScheduleFired,
        ])
    }

    fn on_event(
        &mut self,
        event: &DaemonEvent,
        ctx: &strivo_core::plugin::VerbContext,
    ) -> Vec<PluginAction> {
        match event {
            DaemonEvent::ChannelsUpdated(channels) => {
                self.channels = channels.clone();
            }
            DaemonEvent::RecordingFinished {
                job_id, final_state, ..
            } => {
                if *final_state != RecordingState::Finished || !self.enabled {
                    return Vec::new();
                }
                if let Some(rec) = ctx.recordings.get(job_id) {
                    let channel_key = format!("{}:{}", rec.platform, rec.channel_id);
                    let is_tandem = self.tandem_channels.contains(&channel_key)
                        || rec
                            .playlist
                            .as_ref()
                            .is_some_and(|p| self.tandem_playlists.contains(p));
                    if is_tandem {
                        let channel = self
                            .channels
                            .iter()
                            .find(|c| c.id == rec.channel_id)
                            .cloned();
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

    fn status_line(&self) -> Option<String> { None }
    fn status_slot(&self) -> StatusSlot { StatusSlot::None }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
