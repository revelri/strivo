pub mod analysis;
mod db;
mod pipeline;
pub mod render;
pub mod transcribe;
pub mod types;

use std::any::Any;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use uuid::Uuid;

use crate::app::{AppState, DaemonEvent};
use crate::config::CrunchrAnalysisConfig;
use crate::recording::job::RecordingState;

use super::{
    DaemonEventKind, PaneId, Plugin, PluginAction, PluginCommand, PluginContext,
};
use types::{AnalysisData, PipelineEvent, PipelineState, ProcessingJob, SearchResult};

pub const PANE_ID: PaneId = "crunchr";

pub struct CrunchrPlugin {
    db: Option<rusqlite::Connection>,
    data_dir: PathBuf,
    /// Concurrency guard: recording IDs currently being processed.
    in_flight: HashSet<Uuid>,
    /// Transcription backend (whisper-cli or voxtral).
    backend: Option<Arc<dyn transcribe::TranscriptionBackend>>,
    /// Analysis config for OpenRouter LLM.
    analysis_config: Option<CrunchrAnalysisConfig>,
    pub queue: Vec<ProcessingJob>,
    pub search_results: Vec<SearchResult>,
    pub search_query: String,
    pub search_mode: types::SearchMode,
    pub selected_result: usize,
    pub input_active: bool,
    pub word_frequencies: Vec<(String, i64)>,
    pub backend_available: bool,
    /// Last error message for display.
    pub last_error: Option<String>,
    /// Analysis data for the currently selected search result.
    pub selected_analysis: Option<AnalysisData>,
    /// Speaker label for the currently selected search result.
    pub selected_speaker: Option<String>,
    /// Previous selected_result index (for detecting selection changes).
    prev_selected: usize,
}

impl CrunchrPlugin {
    pub fn new() -> Self {
        Self {
            db: None,
            data_dir: PathBuf::new(),
            in_flight: HashSet::new(),
            backend: None,
            analysis_config: None,
            queue: Vec::new(),
            search_results: Vec::new(),
            search_query: String::new(),
            search_mode: types::SearchMode::FullText,
            selected_result: 0,
            input_active: false,
            word_frequencies: Vec::new(),
            backend_available: false,
            last_error: None,
            selected_analysis: None,
            selected_speaker: None,
            prev_selected: usize::MAX,
        }
    }

    fn execute_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_results.clear();
            return;
        }

        let Some(conn) = self.db.as_ref() else {
            self.last_error = Some("DB not available".to_string());
            return;
        };

        match db::fts_search(conn, &self.search_query, 50) {
            Ok(results) => {
                self.search_results = results;
                self.last_error = None;
            }
            Err(e) => {
                tracing::warn!("Search error: {e}");
                self.search_results.clear();
                self.last_error = Some(format!("Search error: {e}"));
            }
        }
        self.selected_result = 0;
        self.refresh_selected_analysis();
    }

    fn refresh_selected_analysis(&mut self) {
        self.selected_analysis = None;
        self.selected_speaker = None;

        let Some(result) = self.search_results.get(self.selected_result) else { return };
        let Some(conn) = self.db.as_ref() else { return };

        self.selected_analysis = db::get_analysis_for_chunk(conn, result.chunk_id).ok().flatten();
        self.selected_speaker = db::get_speaker_for_chunk(conn, result.chunk_id).ok().flatten();
        self.prev_selected = self.selected_result;
    }

    fn refresh_word_frequencies(&mut self) {
        let Some(conn) = self.db.as_ref() else { return };
        match db::get_top_words(conn, 20) {
            Ok(words) => self.word_frequencies = words,
            Err(e) => tracing::warn!("Word frequency error: {e}"),
        }
    }

    fn queue_recording(&mut self, recording_id: Uuid, channel_name: String, title: String, video_path: PathBuf) -> Vec<PluginAction> {
        if self.in_flight.contains(&recording_id) {
            return Vec::new();
        }
        if self.queue.iter().any(|j| j.recording_id == recording_id) {
            return Vec::new();
        }

        self.in_flight.insert(recording_id);

        if let Some(conn) = self.db.as_ref() {
            if let Err(e) = db::insert_video(
                conn,
                &recording_id.to_string(),
                &channel_name,
                &title,
                &video_path.to_string_lossy(),
            ) {
                self.last_error = Some(format!("DB insert error: {e}"));
                self.in_flight.remove(&recording_id);
                return vec![PluginAction::SetStatus(format!("CrunchR DB error: {e}"))];
            }
        }

        let job = ProcessingJob {
            recording_id,
            channel_name,
            title,
            video_path,
            audio_path: None,
            state: PipelineState::Pending,
            error: None,
        };
        self.queue.push(job);

        self.start_next_stage(recording_id)
    }

    fn start_next_stage(&mut self, recording_id: Uuid) -> Vec<PluginAction> {
        let Some(job_idx) = self.queue.iter().position(|j| j.recording_id == recording_id) else {
            return Vec::new();
        };

        let current_state = self.queue[job_idx].state;

        match current_state {
            PipelineState::Pending => {
                let video_path = self.queue[job_idx].video_path.clone();
                self.queue[job_idx].state = PipelineState::ExtractingAudio;
                if let Some(conn) = self.db.as_ref() {
                    let _ = db::update_video_status(conn, &recording_id.to_string(), "extracting_audio", None);
                }
                let output_dir = self.data_dir.join("audio");
                vec![PluginAction::SpawnTask {
                    plugin_name: "crunchr",
                    future: Box::pin(pipeline::extract_audio(recording_id, video_path, output_dir)),
                }]
            }
            PipelineState::ExtractingAudio => {
                if !self.backend_available {
                    self.queue[job_idx].state = PipelineState::Failed;
                    self.queue[job_idx].error = Some("No transcription backend available".to_string());
                    self.in_flight.remove(&recording_id);
                    return vec![PluginAction::SetStatus("CrunchR: no transcription backend".to_string())];
                }
                let audio_path = self.queue[job_idx].audio_path.clone().unwrap_or_default();
                self.queue[job_idx].state = PipelineState::Transcribing;
                if let Some(conn) = self.db.as_ref() {
                    let _ = db::update_video_status(conn, &recording_id.to_string(), "transcribing", None);
                }
                // Use the TranscriptionBackend trait
                let backend = self.backend.clone().unwrap();
                vec![PluginAction::SpawnTask {
                    plugin_name: "crunchr",
                    future: Box::pin(async move {
                        match backend.transcribe(&audio_path).await {
                            Ok(result) => Box::new(PipelineEvent::TranscriptionComplete {
                                recording_id,
                                segments: result.segments,
                                full_text: result.full_text,
                            }) as Box<dyn Any + Send>,
                            Err(e) => Box::new(PipelineEvent::StageError {
                                recording_id,
                                error: format!("Transcription failed: {e}"),
                            }) as Box<dyn Any + Send>,
                        }
                    }),
                }]
            }
            PipelineState::Transcribing => {
                self.queue[job_idx].state = PipelineState::Chunking;
                if let Some(conn) = self.db.as_ref() {
                    let _ = db::update_video_status(conn, &recording_id.to_string(), "chunking", None);
                }

                // Read segments from DB (fast sync), then spawn CPU-intensive chunking as async task
                let rec_id_str = recording_id.to_string();
                let conn = self.db.as_ref();
                let video_id = conn
                    .and_then(|c| db::get_video_id_by_recording(c, &rec_id_str).ok().flatten());
                let segments = video_id
                    .and_then(|vid| conn.and_then(|c| db::get_segments_for_video(c, vid).ok()));

                if let (Some(vid), Some(segs)) = (video_id, segments) {
                    let seg_structs: Vec<types::Segment> = segs
                        .iter()
                        .map(|(idx, start, end, text)| types::Segment {
                            index: *idx,
                            start_sec: *start,
                            end_sec: *end,
                            text: text.clone(),
                            speaker: None,
                            confidence: None,
                        })
                        .collect();

                    // Spawn chunking + word frequency computation off the event loop
                    vec![PluginAction::SpawnTask {
                        plugin_name: "crunchr",
                        future: Box::pin(async move {
                            // CPU-intensive work in spawn_blocking
                            let result = tokio::task::spawn_blocking(move || {
                                let chunks = pipeline::chunk_segments(&seg_structs, 512);
                                let all_text: String = chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" ");
                                let freqs = pipeline::word_frequencies(&all_text);
                                let chunk_data: Vec<types::ChunkData> = chunks.into_iter().map(|c| types::ChunkData {
                                    text: c.text,
                                    start_sec: c.start_sec,
                                    end_sec: c.end_sec,
                                    token_count: c.token_count,
                                }).collect();
                                (chunk_data, freqs)
                            }).await;

                            match result {
                                Ok((chunks, word_frequencies)) => {
                                    Box::new(PipelineEvent::ChunkingComplete {
                                        recording_id,
                                        video_id: vid,
                                        chunks,
                                        word_frequencies,
                                    }) as Box<dyn Any + Send>
                                }
                                Err(e) => {
                                    Box::new(PipelineEvent::StageError {
                                        recording_id,
                                        error: format!("Chunking failed: {e}"),
                                    }) as Box<dyn Any + Send>
                                }
                            }
                        }),
                    }]
                } else {
                    if let Some(job) = self.queue.iter_mut().find(|j| j.recording_id == recording_id) {
                        job.state = PipelineState::Failed;
                        job.error = Some("No segments found for chunking".to_string());
                    }
                    self.in_flight.remove(&recording_id);
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn handle_pipeline_event(&mut self, event: PipelineEvent) -> Vec<PluginAction> {
        match event {
            PipelineEvent::AudioExtracted { recording_id, audio_path } => {
                if let Some(job) = self.queue.iter_mut().find(|j| j.recording_id == recording_id) {
                    job.audio_path = Some(audio_path.clone());
                }
                if let Some(conn) = self.db.as_ref() {
                    let _ = db::update_video_audio_path(
                        conn,
                        &recording_id.to_string(),
                        &audio_path.to_string_lossy(),
                    );
                }
                self.start_next_stage(recording_id)
            }
            PipelineEvent::TranscriptionComplete { recording_id, segments, full_text } => {
                if let Some(conn) = self.db.as_ref() {
                    let rec_id_str = recording_id.to_string();
                    let _ = db::update_video_transcript(conn, &rec_id_str, &full_text);

                    if let Ok(Some(video_id)) = db::get_video_id_by_recording(conn, &rec_id_str) {
                        let seg_data: Vec<(usize, f64, f64, &str, Option<&str>, Option<f64>)> = segments
                            .iter()
                            .map(|s| (
                                s.index,
                                s.start_sec,
                                s.end_sec,
                                s.text.as_str(),
                                s.speaker.as_deref(),
                                s.confidence,
                            ))
                            .collect();
                        let _ = db::insert_segments(conn, video_id, &seg_data);
                    }
                }

                // Clean up WAV file
                if let Some(job) = self.queue.iter().find(|j| j.recording_id == recording_id) {
                    if let Some(ref audio_path) = job.audio_path {
                        if let Err(e) = std::fs::remove_file(audio_path) {
                            tracing::debug!("Failed to clean up WAV: {e}");
                        }
                    }
                }

                self.start_next_stage(recording_id)
            }
            PipelineEvent::ChunkingComplete { recording_id, video_id, chunks, word_frequencies } => {
                // Write chunk + word frequency results to DB (fast sync writes)
                if let Some(conn) = self.db.as_ref() {
                    let chunk_tuples: Vec<(usize, &str, f64, f64, usize)> = chunks
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (i, c.text.as_str(), c.start_sec, c.end_sec, c.token_count))
                        .collect();
                    let _ = db::insert_chunks(conn, video_id, &chunk_tuples);
                    let _ = db::insert_word_frequencies(conn, video_id, &word_frequencies);
                }

                let rec_id_str = recording_id.to_string();

                // If analysis is enabled, start it
                if let Some(ref analysis_cfg) = self.analysis_config {
                    if analysis_cfg.enabled {
                        if let Some(conn) = self.db.as_ref() {
                            let _ = db::update_video_status(conn, &rec_id_str, "analyzing", None);
                        }
                        if let Some(job) = self.queue.iter_mut().find(|j| j.recording_id == recording_id) {
                            job.state = PipelineState::Analyzing;
                        }

                        let transcript = self.db.as_ref()
                            .and_then(|c| {
                                c.query_row(
                                    "SELECT transcript_text FROM videos WHERE recording_id = ?1",
                                    [&rec_id_str],
                                    |row| row.get::<_, Option<String>>(0),
                                ).ok().flatten()
                            })
                            .unwrap_or_default();

                        let channel_name = self.queue.iter()
                            .find(|j| j.recording_id == recording_id)
                            .map(|j| j.channel_name.clone())
                            .unwrap_or_default();
                        let title = self.queue.iter()
                            .find(|j| j.recording_id == recording_id)
                            .map(|j| j.title.clone())
                            .unwrap_or_default();

                        let cfg = analysis_cfg.clone();
                        self.refresh_word_frequencies();
                        return vec![PluginAction::SpawnTask {
                            plugin_name: "crunchr",
                            future: Box::pin(async move {
                                match analysis::analyze_transcript(&cfg, &channel_name, &title, &transcript).await {
                                    Ok(result) => {
                                        let topics_json = serde_json::to_string(&result.topics).unwrap_or_default();
                                        Box::new(PipelineEvent::AnalysisComplete {
                                            recording_id,
                                            summary: result.summary,
                                            topics: topics_json,
                                            sentiment: result.sentiment,
                                        }) as Box<dyn Any + Send>
                                    }
                                    Err(e) => {
                                        tracing::warn!("Analysis failed (non-fatal): {e}");
                                        Box::new(PipelineEvent::AnalysisComplete {
                                            recording_id,
                                            summary: String::new(),
                                            topics: "[]".to_string(),
                                            sentiment: "unknown".to_string(),
                                        }) as Box<dyn Any + Send>
                                    }
                                }
                            }),
                        }];
                    }
                }

                // No analysis, mark complete
                if let Some(conn) = self.db.as_ref() {
                    let _ = db::update_video_status(conn, &rec_id_str, "complete", None);
                }
                if let Some(job) = self.queue.iter_mut().find(|j| j.recording_id == recording_id) {
                    job.state = PipelineState::Complete;
                }
                self.in_flight.remove(&recording_id);
                self.refresh_word_frequencies();
                Vec::new()
            }
            PipelineEvent::AnalysisComplete { recording_id, summary, topics, sentiment } => {
                // Store analysis results in DB
                if let Some(conn) = self.db.as_ref() {
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO video_analysis (video_id, summary, topics, sentiment) \
                         SELECT id, ?1, ?2, ?3 FROM videos WHERE recording_id = ?4",
                        rusqlite::params![summary, topics, sentiment, recording_id.to_string()],
                    );
                    let _ = db::update_video_status(conn, &recording_id.to_string(), "complete", None);
                }
                if let Some(job) = self.queue.iter_mut().find(|j| j.recording_id == recording_id) {
                    job.state = PipelineState::Complete;
                }
                self.in_flight.remove(&recording_id);
                self.refresh_word_frequencies();
                Vec::new()
            }
            PipelineEvent::StageError { recording_id, error } => {
                if let Some(job) = self.queue.iter_mut().find(|j| j.recording_id == recording_id) {
                    job.state = PipelineState::Failed;
                    job.error = Some(error.clone());
                }
                if let Some(conn) = self.db.as_ref() {
                    let _ = db::update_video_status(
                        conn,
                        &recording_id.to_string(),
                        "failed",
                        Some(&error),
                    );
                }
                self.in_flight.remove(&recording_id);
                self.last_error = Some(error.clone());
                vec![PluginAction::SetStatus(format!("CrunchR error: {error}"))]
            }
        }
    }
}

impl Plugin for CrunchrPlugin {
    fn name(&self) -> &'static str {
        "crunchr"
    }

    fn display_name(&self) -> &str {
        "CrunchR Intelligence"
    }

    fn init(&mut self, ctx: &PluginContext) -> anyhow::Result<()> {
        self.data_dir = ctx.data_dir.clone();

        std::fs::create_dir_all(&ctx.data_dir)?;
        std::fs::create_dir_all(&ctx.cache_dir)?;

        // Migrate old DB name if needed
        let old_db = ctx.data_dir.join("sloptube.db");
        let new_db = ctx.data_dir.join("crunchr.db");
        if old_db.exists() && !new_db.exists() {
            let _ = std::fs::rename(&old_db, &new_db);
        }

        // Open persistent DB connection
        let db_path = ctx.data_dir.join("crunchr.db");
        self.db = Some(db::open_and_init(&db_path)?);

        // Create transcription backend from config
        let crunchr_config = &ctx.config.crunchr;
        let backend = transcribe::create_backend(crunchr_config);
        let backend_name = backend.backend_name();

        // Check if the backend is actually usable
        self.backend_available = match backend_name {
            "whisper-cli" => pipeline::is_whisper_available(),
            "voxtral" => true, // API backends are always "available" (may fail at runtime)
            _ => false,
        };

        if !self.backend_available && backend_name == "whisper-cli" {
            tracing::info!("CrunchR: whisper CLI not found, transcription disabled");
        }

        self.backend = Some(Arc::from(backend));

        // Analysis config
        let analysis = &crunchr_config.analysis;
        if analysis.enabled {
            self.analysis_config = Some(analysis.clone());
            tracing::info!("CrunchR: analysis enabled (model: {})", analysis.model);
        }

        // Load initial word frequencies
        self.refresh_word_frequencies();

        tracing::info!("CrunchR plugin initialized (backend: {backend_name}, db: {})", db_path.display());
        Ok(())
    }

    fn shutdown(&mut self) {
        self.db.take();
        self.backend.take();
        tracing::info!("CrunchR plugin shutting down");
    }

    fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
        Some(vec![DaemonEventKind::RecordingFinished])
    }

    fn on_event(&mut self, event: &DaemonEvent, app: &AppState) -> Vec<PluginAction> {
        if let DaemonEvent::RecordingFinished { job_id, final_state, .. } = event {
            if *final_state != RecordingState::Finished {
                return Vec::new();
            }

            if let Some(rec) = app.recordings.get(job_id) {
                let video_path = rec.output_path.clone();
                let channel_name = rec.channel_name.clone();
                let title = rec.stream_title.clone().unwrap_or_else(|| "Untitled".to_string());

                return self.queue_recording(*job_id, channel_name, title, video_path);
            }
        }
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, _app: &AppState) -> Vec<PluginAction> {
        if self.input_active {
            match key.code {
                KeyCode::Esc => {
                    self.input_active = false;
                }
                KeyCode::Enter => {
                    self.input_active = false;
                    self.execute_search();
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return Vec::new();
        }

        match key.code {
            KeyCode::Char('/') => {
                self.input_active = true;
            }
            KeyCode::Tab => {
                self.search_mode = self.search_mode.toggle();
                if !self.search_query.is_empty() {
                    self.execute_search();
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.search_results.is_empty() {
                    self.selected_result = (self.selected_result + 1) % self.search_results.len();
                    self.refresh_selected_analysis();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.search_results.is_empty() {
                    self.selected_result = if self.selected_result == 0 {
                        self.search_results.len() - 1
                    } else {
                        self.selected_result - 1
                    };
                    self.refresh_selected_analysis();
                }
            }
            KeyCode::Enter => {
                if let Some(result) = self.search_results.get(self.selected_result) {
                    if let Some(ref path) = result.video_path {
                        return vec![PluginAction::PlayFile(PathBuf::from(path))];
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
        if let Ok(pipeline_event) = event.downcast::<PipelineEvent>() {
            return self.handle_pipeline_event(*pipeline_event);
        }
        Vec::new()
    }

    fn commands(&self) -> Vec<PluginCommand> {
        vec![PluginCommand {
            name: "Intelligence",
            description: "CrunchR transcript search",
            key: KeyCode::Char('I'),
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
        render::render(self, frame, area, app);
    }

    fn status_line(&self, _app: &AppState) -> Option<String> {
        let pending = self.queue.iter().filter(|j| {
            j.state != PipelineState::Complete && j.state != PipelineState::Failed
        }).count();

        if pending > 0 {
            Some(format!("CR:{pending}"))
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
