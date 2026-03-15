use chrono::{DateTime, Utc};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    ResolvingUrl,
    Recording,
    Stopping,
    Finished,
    Failed,
}

impl std::fmt::Display for RecordingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingState::ResolvingUrl => write!(f, "Resolving"),
            RecordingState::Recording => write!(f, "Recording"),
            RecordingState::Stopping => write!(f, "Stopping"),
            RecordingState::Finished => write!(f, "Finished"),
            RecordingState::Failed => write!(f, "Failed"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecordingJob {
    pub id: Uuid,
    pub channel_id: String,
    pub channel_name: String,
    pub platform: crate::platform::PlatformKind,
    pub state: RecordingState,
    pub output_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub bytes_written: u64,
    pub duration_secs: f64,
    pub transcode: bool,
    pub error: Option<String>,
    pub stream_title: Option<String>,
    pub watched: bool,
}

impl RecordingJob {
    pub fn new(
        channel_id: String,
        channel_name: String,
        platform: crate::platform::PlatformKind,
        output_path: PathBuf,
        transcode: bool,
        stream_title: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel_id,
            channel_name,
            platform,
            state: RecordingState::ResolvingUrl,
            output_path,
            started_at: Utc::now(),
            bytes_written: 0,
            duration_secs: 0.0,
            transcode,
            error: None,
            stream_title,
            watched: false,
        }
    }

    pub fn format_duration(&self) -> String {
        let secs = self.duration_secs as u64;
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        if hours > 0 {
            format!("{hours}:{mins:02}:{secs:02}")
        } else {
            format!("{mins}:{secs:02}")
        }
    }

    #[allow(dead_code)]
    pub fn format_size(&self) -> String {
        let bytes = self.bytes_written;
        if bytes >= 1_073_741_824 {
            format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
        } else if bytes >= 1_048_576 {
            format!("{:.1} MB", bytes as f64 / 1_048_576.0)
        } else if bytes >= 1024 {
            format!("{:.0} KB", bytes as f64 / 1024.0)
        } else {
            format!("{bytes} B")
        }
    }
}
