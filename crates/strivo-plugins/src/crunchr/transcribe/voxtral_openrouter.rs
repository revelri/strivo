use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

use super::{TranscriptionBackend, TranscriptionResult};
use crate::crunchr::types::Segment;

/// Voxtral via OpenRouter's `/audio/transcriptions` endpoint.
/// Default transcription backend. $0.003/min for `mistralai/voxtral-mini-transcribe`.
/// OpenRouter's transcription response is plain `text` + `usage` — no segments or
/// diarization. Tandem diarization keeps falling back to the direct Mistral API.
pub struct VoxtralOpenRouterBackend {
    api_key: String,
    model: String,
}

impl VoxtralOpenRouterBackend {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "mistralai/voxtral-mini-transcribe".to_string()),
        }
    }
}

#[async_trait]
impl TranscriptionBackend for VoxtralOpenRouterBackend {
    async fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult> {
        let audio_bytes = tokio::fs::read(audio_path).await?;
        let audio_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &audio_bytes);

        let format = audio_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_else(|| "wav".to_string());

        let request_body = serde_json::json!({
            "model": self.model,
            "input_audio": {
                "data": audio_b64,
                "format": format,
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://openrouter.ai/api/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("X-Title", "StriVo CrunchR")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            anyhow::bail!(
                "OpenRouter transcription returned {status}: {}",
                body.chars().take(300).collect::<String>()
            );
        }

        let parsed: serde_json::Value = response.json().await?;
        let full_text = parsed["text"].as_str().unwrap_or("").trim().to_string();

        // OpenRouter's transcription endpoint returns a single text blob and no
        // segments. Emit one synthetic segment so the downstream pipeline
        // (analysis, search index) has something to chew on.
        let segments = if full_text.is_empty() {
            Vec::new()
        } else {
            vec![Segment {
                index: 0,
                start_sec: 0.0,
                end_sec: parsed["usage"]["seconds"].as_f64().unwrap_or(0.0),
                text: full_text.clone(),
                speaker: None,
                confidence: None,
                words: None, // openrouter returns plain text, no timings
            }]
        };

        Ok(TranscriptionResult {
            segments,
            full_text,
        })
    }

    fn supports_diarization(&self) -> bool {
        false
    }

    fn backend_name(&self) -> &'static str {
        "voxtral-openrouter"
    }
}
