pub mod voxtral_api;
pub mod voxtral_local;
pub mod whisper_cli;

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

use super::types::Segment;
use crate::config::CrunchrConfig;

/// Result of a transcription operation.
pub struct TranscriptionResult {
    pub segments: Vec<Segment>,
    pub full_text: String,
}

/// Backend abstraction for transcription providers.
#[allow(dead_code)]
#[async_trait]
pub trait TranscriptionBackend: Send + Sync {
    async fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult>;
    fn supports_diarization(&self) -> bool;
    fn backend_name(&self) -> &'static str;
}

/// Create the appropriate backend from config.
pub fn create_backend(config: &CrunchrConfig) -> Box<dyn TranscriptionBackend> {
    match config.backend.as_str() {
        "voxtral-api" | "voxtral" => {
            let api_key = config
                .api_key_env
                .as_deref()
                .and_then(|env_name| std::env::var(env_name).ok())
                .unwrap_or_default();

            if api_key.is_empty() {
                tracing::warn!(
                    "Voxtral API backend selected but no API key found (env: {:?}). Falling back to whisper-cli.",
                    config.api_key_env
                );
                Box::new(whisper_cli::WhisperCLIBackend::new(
                    config.whisper_model.clone(),
                    config.whisper_timeout_secs,
                ))
            } else {
                Box::new(voxtral_api::VoxtralApiBackend::new(api_key))
            }
        }
        "voxtral-local" => {
            let endpoint = config
                .endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:8000/v1".to_string());

            Box::new(voxtral_local::VoxtralLocalBackend::new(endpoint))
        }
        _ => Box::new(whisper_cli::WhisperCLIBackend::new(
            config.whisper_model.clone(),
            config.whisper_timeout_secs,
        )),
    }
}
