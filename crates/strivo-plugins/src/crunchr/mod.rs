//! Crunchr — Whisper / Voxtral transcription orchestrator.
//!
//! The active transcription pipeline lives in the `transcribe`
//! submodule; this `mod.rs` was the TUI host (transcript view, search
//! modal, speaker editor). With the TUI deletion the plugin is now a
//! headless trigger shell — it listens for `RecordingFinished` events
//! and would queue tandem-configured channels through the pipeline.
//! The webui's transcript surface reads `crunchr.db` directly.

use std::any::Any;
use std::path::PathBuf;

use strivo_core::events::DaemonEvent;
use strivo_core::plugin::{
    DaemonEventKind, Plugin, PluginAction, PluginContext, StatusSlot,
};
use strivo_core::recording::job::RecordingState;

pub mod analysis;
pub mod cost;
pub mod db;
pub mod presets;
pub mod transcribe;
pub mod types;
pub mod voice_samples;

pub struct CrunchrPlugin {
    data_dir: PathBuf,
    db_path: PathBuf,
    tandem_channels: Vec<String>,
    tandem_playlists: Vec<String>,
    enabled: bool,
}

impl Default for CrunchrPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl CrunchrPlugin {
    pub fn new() -> Self {
        Self {
            data_dir: PathBuf::new(),
            db_path: PathBuf::new(),
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
            enabled: true,
        }
    }

    fn queue_recording(
        &mut self,
        _job_id: uuid::Uuid,
        _channel_name: String,
        _title: String,
        _video_path: PathBuf,
    ) -> Vec<PluginAction> {
        // Headless queue hook. The webui dispatches transcription jobs
        // via PluginRpc verbs in the post-TUI architecture.
        Vec::new()
    }
}

impl Plugin for CrunchrPlugin {
    fn name(&self) -> &'static str { "crunchr" }
    fn display_name(&self) -> &str { "Crunchr" }

    fn init(&mut self, ctx: &PluginContext) -> anyhow::Result<()> {
        self.data_dir = ctx.data_dir.join("plugins").join("crunchr");
        std::fs::create_dir_all(&self.data_dir)?;
        self.db_path = self.data_dir.join("crunchr.db");
        Ok(())
    }

    fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
        Some(vec![DaemonEventKind::RecordingFinished])
    }

    fn on_event(
        &mut self,
        event: &DaemonEvent,
        ctx: &strivo_core::plugin::VerbContext,
    ) -> Vec<PluginAction> {
        if let DaemonEvent::RecordingFinished {
            job_id,
            final_state,
            ..
        } = event
        {
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

                let crunchr_auto_marker = rec
                    .output_path
                    .parent()
                    .map(|p| p.join(".crunchr-auto"))
                    .map(|m| m.exists())
                    .unwrap_or(false);

                if is_tandem || crunchr_auto_marker {
                    let video_path = rec.output_path.clone();
                    let channel_name = rec.channel_name.clone();
                    let title = rec
                        .stream_title
                        .clone()
                        .unwrap_or_else(|| "Untitled".to_string());
                    return self.queue_recording(*job_id, channel_name, title, video_path);
                }
            }
        }
        Vec::new()
    }

    fn status_line(&self) -> Option<String> { None }
    fn status_slot(&self) -> StatusSlot { StatusSlot::None }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
