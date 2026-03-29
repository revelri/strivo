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
use crate::config::{CrunchrAnalysisConfig, CrunchrConfig};
use crate::recording::job::RecordingState;

use super::{
    DaemonEventKind, PaneId, Plugin, PluginAction, PluginCommand, PluginContext,
};
use types::{
    AnalysisData, ConfigModalState, CrunchrView, PipelineEvent, PipelineState,
    PickerState, ProcessingJob, RecordingFilter, SearchResult,
};

pub const PANE_ID: PaneId = "crunchr";

/// Number of static config fields in the Crunchr config modal (before channel checklist).
/// Fields: enabled, backend, api_key, endpoint, whisper_model, analysis_enabled = 6 fields.
/// Channel checkboxes start at index CRUNCHR_STATIC_FIELDS (i.e., after index 5).
const CRUNCHR_STATIC_FIELDS: usize = 6;

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

    // --- New: config modal, views, tandem ---
    /// Whether the plugin is enabled for tandem auto-processing.
    pub enabled: bool,
    /// Whether the first-run config has been completed.
    pub configured: bool,
    /// Tandem channels (auto-trigger on RecordingFinished).
    pub tandem_channels: Vec<String>,
    /// Tandem playlists.
    pub tandem_playlists: Vec<String>,
    /// Config modal state.
    pub config_modal: ConfigModalState,
    /// Draft config being edited in the modal.
    pub config_draft: Option<CrunchrConfig>,
    /// Current view mode.
    pub view: CrunchrView,
    /// Recording picker state.
    pub picker: PickerState,
    /// Cached channel list for the config modal tandem checkboxes.
    pub cached_channels: Vec<(String, String)>, // (channel_key "Platform:id", display_name)
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
            enabled: false,
            configured: false,
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
            config_modal: ConfigModalState::Hidden,
            config_draft: None,
            view: CrunchrView::Search,
            picker: PickerState::default(),
            cached_channels: Vec::new(),
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

    /// Query transcript/analysis info for a recording (used by properties modal).
    pub fn recording_info(&self, recording_id: &str) -> Option<types::CrunchrRecordingInfo> {
        let conn = self.db.as_ref()?;

        // Get video status
        let status: Option<String> = conn.query_row(
            "SELECT status FROM videos WHERE recording_id = ?1",
            [recording_id],
            |row| row.get(0),
        ).ok();

        let status = status?;

        // Get video ID for further queries
        let video_id: Option<i64> = conn.query_row(
            "SELECT id FROM videos WHERE recording_id = ?1",
            [recording_id],
            |row| row.get(0),
        ).ok();

        let video_id = video_id?;

        // Count segments
        let segment_count: usize = conn.query_row(
            "SELECT COUNT(*) FROM segments WHERE video_id = ?1",
            [video_id],
            |row| row.get::<_, i64>(0),
        ).ok().unwrap_or(0) as usize;

        // Count words (sum of word frequencies)
        let word_count: usize = conn.query_row(
            "SELECT COALESCE(SUM(count), 0) FROM word_frequency WHERE video_id = ?1",
            [video_id],
            |row| row.get::<_, i64>(0),
        ).ok().unwrap_or(0) as usize;

        // Get analysis data
        let analysis: Option<(String, String, String)> = conn.query_row(
            "SELECT summary, topics, sentiment FROM video_analysis WHERE video_id = ?1",
            [video_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            )),
        ).ok();

        let (has_analysis, summary, topics, sentiment) = if let Some((s, t, sent)) = analysis {
            let topic_list: Vec<String> = serde_json::from_str(&t).unwrap_or_default();
            (true, Some(s), topic_list, Some(sent))
        } else {
            (false, None, Vec::new(), None)
        };

        Some(types::CrunchrRecordingInfo {
            status,
            segment_count,
            word_count,
            has_analysis,
            summary,
            topics,
            sentiment,
        })
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

    /// Open the config modal, cloning current config into draft.
    fn open_config_modal(&mut self, app: &AppState) {
        // Cache channels for the tandem checklist
        self.cached_channels = app.channels.iter().map(|ch| {
            let key = format!("{}:{}", ch.platform, ch.id);
            let display = format!("[{}] {}", ch.platform, ch.display_name);
            (key, display)
        }).collect();

        // Clone current config into draft
        let mut draft = CrunchrConfig {
            enabled: self.enabled,
            configured: self.configured,
            backend: self.backend.as_ref().map_or_else(
                || "whisper-cli".to_string(),
                |b| b.backend_name().to_string(),
            ),
            api_key_env: None,
            endpoint: None,
            whisper_model: None,
            whisper_timeout_secs: 7200,
            analysis: CrunchrAnalysisConfig::default(),
            tandem_channels: self.tandem_channels.clone(),
            tandem_playlists: self.tandem_playlists.clone(),
        };
        // Try to read current config values from analysis_config
        if let Some(ref ac) = self.analysis_config {
            draft.analysis = ac.clone();
        }
        self.config_draft = Some(draft);

        self.config_modal = ConfigModalState::Active {
            selected_field: 0,
            editing: false,
            static_field_count: CRUNCHR_STATIC_FIELDS,
        };
    }

    /// Handle keys while config modal is active.
    fn handle_config_modal_key(&mut self, key: KeyEvent, _app: &AppState) -> Vec<PluginAction> {
        let ConfigModalState::Active { ref mut selected_field, ref mut editing, static_field_count } = self.config_modal else {
            return Vec::new();
        };
        let total_fields = static_field_count + self.cached_channels.len();

        // Indices: 0=enabled, 1=backend, 2=api_key, 3=endpoint, 4=whisper_model,
        //          5=analysis_enabled, 6..6+N=tandem channels, last=[Save]
        // We'll use: 0..static_field_count-1 for static fields, then channels, then Save
        let save_idx = total_fields; // Save button is after all fields

        if *editing {
            match key.code {
                KeyCode::Esc => { *editing = false; }
                KeyCode::Enter => { *editing = false; }
                KeyCode::Backspace => {
                    if let Some(ref mut draft) = self.config_draft {
                        match *selected_field {
                            2 => { if let Some(s) = draft.api_key_env.as_mut() { s.pop(); } }
                            3 => { if let Some(s) = draft.endpoint.as_mut() { s.pop(); } }
                            4 => { if let Some(s) = draft.whisper_model.as_mut() { s.pop(); } }
                            _ => {}
                        }
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut draft) = self.config_draft {
                        match *selected_field {
                            2 => draft.api_key_env.get_or_insert_with(String::new).push(c),
                            3 => draft.endpoint.get_or_insert_with(String::new).push(c),
                            4 => draft.whisper_model.get_or_insert_with(String::new).push(c),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return Vec::new();
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                *selected_field = (*selected_field + 1).min(save_idx);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                *selected_field = selected_field.saturating_sub(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if *selected_field == save_idx {
                    // Save: emit UpdateConfig action
                    if let Some(mut draft) = self.config_draft.take() {
                        draft.configured = true;
                        self.enabled = draft.enabled;
                        self.configured = true;
                        self.tandem_channels = draft.tandem_channels.clone();
                        self.tandem_playlists = draft.tandem_playlists.clone();
                        if draft.analysis.enabled {
                            self.analysis_config = Some(draft.analysis.clone());
                        } else {
                            self.analysis_config = None;
                        }
                        self.config_modal = ConfigModalState::Hidden;
                        return vec![PluginAction::UpdateConfig {
                            plugin_name: "crunchr",
                            config_update: Box::new(draft),
                        }];
                    }
                    self.config_modal = ConfigModalState::Hidden;
                } else if *selected_field == 0 {
                    // Toggle enabled
                    if let Some(ref mut draft) = self.config_draft {
                        draft.enabled = !draft.enabled;
                    }
                } else if *selected_field == 1 {
                    // Cycle backend
                    if let Some(ref mut draft) = self.config_draft {
                        draft.backend = match draft.backend.as_str() {
                            "whisper-cli" => "voxtral-api".to_string(),
                            "voxtral-api" => "voxtral-local".to_string(),
                            _ => "whisper-cli".to_string(),
                        };
                    }
                } else if *selected_field == 5 {
                    // Toggle analysis enabled
                    if let Some(ref mut draft) = self.config_draft {
                        draft.analysis.enabled = !draft.analysis.enabled;
                    }
                } else if *selected_field >= CRUNCHR_STATIC_FIELDS && *selected_field < save_idx {
                    // Tandem channel toggle
                    let ch_idx = *selected_field - CRUNCHR_STATIC_FIELDS;
                    if let Some(ref mut draft) = self.config_draft {
                        if let Some((key, _)) = self.cached_channels.get(ch_idx) {
                            if draft.tandem_channels.contains(key) {
                                draft.tandem_channels.retain(|k| k != key);
                            } else {
                                draft.tandem_channels.push(key.clone());
                                // Auto-enable when tandem channels are added
                                draft.enabled = true;
                            }
                        }
                    }
                } else if matches!(*selected_field, 2 | 3 | 4) {
                    // Text input fields - enter edit mode
                    *editing = true;
                }
            }
            KeyCode::Esc => {
                self.config_modal = ConfigModalState::Hidden;
                self.config_draft = None;
            }
            _ => {}
        }
        Vec::new()
    }

    /// Handle keys in the Search view (original behavior).
    fn handle_search_key(&mut self, key: KeyEvent, app: &AppState) -> Vec<PluginAction> {
        if self.input_active {
            match key.code {
                KeyCode::Esc => { self.input_active = false; }
                KeyCode::Enter => {
                    self.input_active = false;
                    self.execute_search();
                }
                KeyCode::Backspace => { self.search_query.pop(); }
                KeyCode::Char(c) => { self.search_query.push(c); }
                _ => {}
            }
            return Vec::new();
        }

        match key.code {
            KeyCode::Char('/') => { self.input_active = true; }
            KeyCode::Char('c') => { self.open_config_modal(app); }
            KeyCode::Tab => {
                self.view = CrunchrView::Queue;
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

    /// Handle keys in the Queue view.
    fn handle_queue_key(&mut self, key: KeyEvent) -> Vec<PluginAction> {
        match key.code {
            KeyCode::Tab => { self.view = CrunchrView::RecordingPicker; }
            KeyCode::Esc => { self.view = CrunchrView::Search; }
            _ => {}
        }
        Vec::new()
    }

    /// Handle keys in the RecordingPicker view.
    fn handle_picker_key(&mut self, key: KeyEvent, app: &AppState) -> Vec<PluginAction> {
        // Refresh visible recordings list
        self.refresh_picker_list(app);

        match key.code {
            KeyCode::Tab => { self.view = CrunchrView::Search; }
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.picker.visible_ids.is_empty() {
                    self.picker.selected = (self.picker.selected + 1) % self.picker.visible_ids.len();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.picker.visible_ids.is_empty() {
                    self.picker.selected = if self.picker.selected == 0 {
                        self.picker.visible_ids.len() - 1
                    } else {
                        self.picker.selected - 1
                    };
                }
            }
            KeyCode::Char(' ') => {
                // Toggle multi-select
                if let Some(&id) = self.picker.visible_ids.get(self.picker.selected) {
                    if !self.picker.selections.remove(&id) {
                        self.picker.selections.insert(id);
                    }
                }
            }
            KeyCode::Char('a') => {
                // Select all visible
                for &id in &self.picker.visible_ids {
                    self.picker.selections.insert(id);
                }
            }
            KeyCode::Char('f') => {
                // Cycle filter
                self.cycle_picker_filter(app);
                self.refresh_picker_list(app);
            }
            KeyCode::Enter => {
                return self.process_selected_recordings(app);
            }
            KeyCode::Esc => {
                self.picker.selections.clear();
                self.view = CrunchrView::Search;
            }
            _ => {}
        }
        Vec::new()
    }

    /// Refresh the recording picker's visible list based on the current filter.
    fn refresh_picker_list(&mut self, app: &AppState) {
        let finished: Vec<_> = app.recordings.values()
            .filter(|r| r.state == RecordingState::Finished)
            .filter(|r| !self.in_flight.contains(&r.id))
            .filter(|r| match &self.picker.filter {
                RecordingFilter::All => true,
                RecordingFilter::ByChannel(ch) => {
                    let key = format!("{}:{}", r.platform, r.channel_id);
                    key == *ch
                }
                RecordingFilter::ByPlaylist(pl) => {
                    r.playlist.as_deref() == Some(pl.as_str())
                }
            })
            .collect();

        self.picker.visible_ids = finished.iter().map(|r| r.id).collect();
        if self.picker.selected >= self.picker.visible_ids.len() {
            self.picker.selected = self.picker.visible_ids.len().saturating_sub(1);
        }
    }

    /// Cycle through recording picker filters.
    fn cycle_picker_filter(&mut self, app: &AppState) {
        // Collect unique channels from finished recordings
        let channels: Vec<String> = {
            let mut seen = HashSet::new();
            app.recordings.values()
                .filter(|r| r.state == RecordingState::Finished)
                .filter_map(|r| {
                    let key = format!("{}:{}", r.platform, r.channel_id);
                    if seen.insert(key.clone()) { Some(key) } else { None }
                })
                .collect()
        };

        self.picker.filter = match &self.picker.filter {
            RecordingFilter::All => {
                if let Some(ch) = channels.first() {
                    RecordingFilter::ByChannel(ch.clone())
                } else {
                    RecordingFilter::All
                }
            }
            RecordingFilter::ByChannel(current) => {
                let idx = channels.iter().position(|c| c == current).unwrap_or(0);
                if idx + 1 < channels.len() {
                    RecordingFilter::ByChannel(channels[idx + 1].clone())
                } else {
                    RecordingFilter::All
                }
            }
            RecordingFilter::ByPlaylist(_) => RecordingFilter::All,
        };
        self.picker.selections.clear();
    }

    /// Process selected (or focused) recordings from the picker.
    fn process_selected_recordings(&mut self, app: &AppState) -> Vec<PluginAction> {
        let ids: Vec<Uuid> = if self.picker.selections.is_empty() {
            // Process just the focused recording
            self.picker.visible_ids.get(self.picker.selected).copied().into_iter().collect()
        } else {
            self.picker.selections.drain().collect()
        };

        let mut actions = Vec::new();
        for id in ids {
            if let Some(rec) = app.recordings.get(&id) {
                let mut batch = self.queue_recording(
                    id,
                    rec.channel_name.clone(),
                    rec.stream_title.clone().unwrap_or_else(|| "Untitled".to_string()),
                    rec.output_path.clone(),
                );
                actions.append(&mut batch);
            }
        }
        if !actions.is_empty() {
            self.view = CrunchrView::Queue;
        }
        actions
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

        // Load tandem / enabled state from config
        self.enabled = crunchr_config.enabled;
        self.configured = crunchr_config.configured;
        self.tandem_channels = crunchr_config.tandem_channels.clone();
        self.tandem_playlists = crunchr_config.tandem_playlists.clone();

        // Load initial word frequencies
        self.refresh_word_frequencies();

        tracing::info!("CrunchR plugin initialized (backend: {backend_name}, enabled: {}, configured: {}, tandem_channels: {}, db: {})",
            self.enabled, self.configured, self.tandem_channels.len(), db_path.display());
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

            // Only auto-trigger if enabled AND channel/playlist matches tandem config
            if !self.enabled {
                return Vec::new();
            }

            if let Some(rec) = app.recordings.get(job_id) {
                let channel_key = format!("{}:{}", rec.platform, rec.channel_id);
                let is_tandem = self.tandem_channels.contains(&channel_key)
                    || rec.playlist.as_ref().is_some_and(|p| self.tandem_playlists.contains(p));

                if is_tandem {
                    let video_path = rec.output_path.clone();
                    let channel_name = rec.channel_name.clone();
                    let title = rec.stream_title.clone().unwrap_or_else(|| "Untitled".to_string());
                    return self.queue_recording(*job_id, channel_name, title, video_path);
                }
            }
        }
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, app: &AppState) -> Vec<PluginAction> {
        // --- First-run: auto-open config modal if not configured ---
        if !self.configured && self.config_modal == ConfigModalState::Hidden {
            self.open_config_modal(app);
        }

        // --- Config modal intercepts all keys when active ---
        if self.config_modal != ConfigModalState::Hidden {
            return self.handle_config_modal_key(key, app);
        }

        // --- View-specific key handling ---
        match self.view {
            CrunchrView::Search => self.handle_search_key(key, app),
            CrunchrView::Queue => self.handle_queue_key(key),
            CrunchrView::RecordingPicker => self.handle_picker_key(key, app),
        }
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
        // Render the active view
        match self.view {
            CrunchrView::Search => render::render(self, frame, area, app),
            CrunchrView::Queue => render::render_queue(self, frame, area),
            CrunchrView::RecordingPicker => render::render_recording_picker(self, frame, area, app),
        }

        // Overlay config modal if active
        if self.config_modal != ConfigModalState::Hidden {
            render::render_config_modal(self, frame, area);
        }
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
