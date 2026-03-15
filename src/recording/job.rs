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

#[derive(Debug, Clone)]
pub struct RecordingJob {
    pub id: Uuid,
    pub channel_id: String,
    pub channel_name: String,
    pub state: RecordingState,
    pub output_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub bytes_written: u64,
    pub duration_secs: f64,
    pub transcode: bool,
}
