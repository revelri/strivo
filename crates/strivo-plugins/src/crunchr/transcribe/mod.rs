pub mod voxtral_api;
pub mod voxtral_local;
pub mod voxtral_openrouter;
pub mod whisper_cli;
pub mod whisperx_local;

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

use super::types::Segment;
use strivo_core::config::CrunchrConfig;

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
///
/// Default backend is `voxtral-openrouter` (Voxtral Mini Transcribe via OpenRouter,
/// $0.003/min). Requires an env var named in `api_key_env` (defaults to
/// `OPENROUTER_API_KEY`). Falls back to `whisper-cli` when the key is missing.
pub fn create_backend(config: &CrunchrConfig) -> Box<dyn TranscriptionBackend> {
    match config.backend.as_str() {
        "voxtral-openrouter" | "openrouter" => {
            let api_key = lookup_api_key(config.api_key_env.as_deref(), "OPENROUTER_API_KEY");

            if api_key.is_empty() {
                tracing::warn!(
                    "voxtral-openrouter backend selected but no API key found (env: {:?}). Falling back to whisper-cli.",
                    config.api_key_env.as_deref().unwrap_or("OPENROUTER_API_KEY")
                );
                Box::new(whisper_cli::WhisperCLIBackend::new(
                    config.whisper_model.clone(),
                    config.whisper_timeout_secs,
                ))
            } else {
                Box::new(voxtral_openrouter::VoxtralOpenRouterBackend::new(
                    api_key, None,
                ))
            }
        }
        "voxtral-api" | "voxtral" => {
            let api_key = lookup_api_key(config.api_key_env.as_deref(), "MISTRAL_API_KEY");

            if api_key.is_empty() {
                tracing::warn!(
                    "voxtral-api backend selected but no API key found (env: {:?}). Falling back to whisper-cli.",
                    config.api_key_env.as_deref().unwrap_or("MISTRAL_API_KEY")
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
        "whisperx-local" => {
            // pyannote needs HF_TOKEN — read from config.api_key_env if set,
            // else the canonical HF_TOKEN env var. We don't fail open here;
            // the Python orchestrator skips diarization with a clear warning
            // when the token is missing, so transcription still works.
            let var = config.api_key_env.as_deref().unwrap_or("HF_TOKEN");
            if std::env::var(var).is_err() && config.diarize {
                tracing::warn!(
                    "whisperx-local: diarize=true but {var} is not set; pyannote will be skipped."
                );
            }
            Box::new(whisperx_local::WhisperxLocalBackend::new(
                config.whisper_timeout_secs,
                config.diarize,
            ))
        }
        "whisper-cli" => Box::new(whisper_cli::WhisperCLIBackend::new(
            config.whisper_model.clone(),
            config.whisper_timeout_secs,
        )),
        // Unknown backend → treat as default (voxtral-openrouter with key, else whisper-cli).
        _ => {
            let api_key = lookup_api_key(config.api_key_env.as_deref(), "OPENROUTER_API_KEY");
            if api_key.is_empty() {
                Box::new(whisper_cli::WhisperCLIBackend::new(
                    config.whisper_model.clone(),
                    config.whisper_timeout_secs,
                ))
            } else {
                Box::new(voxtral_openrouter::VoxtralOpenRouterBackend::new(
                    api_key, None,
                ))
            }
        }
    }
}

/// Resolve the API key from the configured env var, falling back to a
/// well-known default env var when `api_key_env` is unset.
fn lookup_api_key(configured: Option<&str>, default_var: &str) -> String {
    let var = configured.unwrap_or(default_var);
    std::env::var(var).unwrap_or_default()
}
