use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

use super::{TranscriptionBackend, TranscriptionResult};
use crate::plugin::crunchr::types::Segment;

/// Voxtral Transcribe 2 backend using the Mistral API.
/// $0.003/min, 4% WER, diarization included.
pub struct VoxtralApiBackend {
    api_key: String,
}

impl VoxtralApiBackend {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl TranscriptionBackend for VoxtralApiBackend {
    async fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult> {
        let audio_bytes = tokio::fs::read(audio_path).await?;
        let audio_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &audio_bytes,
        );

        let file_name = audio_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("audio.wav");

        let request_body = serde_json::json!({
            "model": "mistral-audio-latest",
            "temperature": 0.0,
            "response_format": "verbose_json",
            "file": {
                "data": audio_b64,
                "name": file_name,
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.mistral.ai/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            anyhow::bail!("Voxtral API returned {status}: {}", body.chars().take(300).collect::<String>());
        }

        let parsed: serde_json::Value = response.json().await?;

        let full_text = parsed["text"].as_str().unwrap_or("").to_string();

        let segments = parsed["segments"]
            .as_array()
            .map(|segs| {
                segs.iter()
                    .enumerate()
                    .map(|(i, seg)| Segment {
                        index: i,
                        start_sec: seg["start"].as_f64().unwrap_or(0.0),
                        end_sec: seg["end"].as_f64().unwrap_or(0.0),
                        text: seg["text"].as_str().unwrap_or("").trim().to_string(),
                        speaker: seg["speaker"].as_str().map(String::from),
                        confidence: seg["avg_logprob"].as_f64(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(TranscriptionResult {
            segments,
            full_text,
        })
    }

    fn supports_diarization(&self) -> bool {
        true
    }

    fn backend_name(&self) -> &'static str {
        "voxtral-api"
    }
}
