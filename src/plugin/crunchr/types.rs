use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    Pending,
    ExtractingAudio,
    Transcribing,
    Chunking,
    Analyzing,
    Complete,
    Failed,
}

impl std::fmt::Display for PipelineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "Pending"),
            Self::ExtractingAudio => write!(f, "Extracting Audio"),
            Self::Transcribing => write!(f, "Transcribing"),
            Self::Chunking => write!(f, "Chunking"),
            Self::Analyzing => write!(f, "Analyzing"),
            Self::Complete => write!(f, "Complete"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessingJob {
    pub recording_id: Uuid,
    pub channel_name: String,
    pub title: String,
    pub video_path: PathBuf,
    pub audio_path: Option<PathBuf>,
    pub state: PipelineState,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    FullText,
    Semantic,
}

impl SearchMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::FullText => "FTS",
            Self::Semantic => "SEM",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Self::FullText => Self::Semantic,
            Self::Semantic => Self::FullText,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub video_title: String,
    pub channel_name: String,
    pub snippet: String,
    pub start_sec: f64,
    pub score: f64,
    pub video_path: Option<String>,
}

/// Events returned from async pipeline tasks.
pub enum PipelineEvent {
    AudioExtracted {
        recording_id: Uuid,
        audio_path: PathBuf,
    },
    TranscriptionComplete {
        recording_id: Uuid,
        segments: Vec<Segment>,
        full_text: String,
    },
    ChunkingComplete {
        recording_id: Uuid,
        video_id: i64,
        chunks: Vec<ChunkData>,
        word_frequencies: Vec<(String, usize)>,
    },
    AnalysisComplete {
        recording_id: Uuid,
        summary: String,
        topics: String,
        sentiment: String,
    },
    StageError {
        recording_id: Uuid,
        error: String,
    },
}

/// Chunk data produced by async chunking, to be written to DB.
#[derive(Debug, Clone)]
pub struct ChunkData {
    pub text: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub token_count: usize,
}

/// Analysis data for a video, fetched from video_analysis table.
#[derive(Debug, Clone, Default)]
pub struct AnalysisData {
    pub summary: String,
    pub topics: Vec<String>,
    pub sentiment: String,
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub index: usize,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub speaker: Option<String>,
    pub confidence: Option<f64>,
}
