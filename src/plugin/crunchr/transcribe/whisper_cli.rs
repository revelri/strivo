use std::path::Path;
use std::process::Stdio;

use anyhow::Result;
use async_trait::async_trait;

use super::{TranscriptionBackend, TranscriptionResult};
use crate::plugin::crunchr::types::Segment;

pub struct WhisperCLIBackend {
    preferred_model: Option<String>,
    timeout_secs: u64,
}

impl WhisperCLIBackend {
    pub fn new(preferred_model: Option<String>, timeout_secs: u64) -> Self {
        Self {
            preferred_model,
            timeout_secs,
        }
    }
}

#[async_trait]
impl TranscriptionBackend for WhisperCLIBackend {
    async fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult> {
        let output_dir = audio_path.parent().unwrap_or(Path::new("."));

        // Build model list: preferred first, then fallbacks
        let mut models: Vec<&str> = Vec::new();
        if let Some(ref pref) = self.preferred_model {
            models.push(pref);
        }
        for m in &["base", "small", "medium", "large-v3"] {
            if !models.contains(m) {
                models.push(m);
            }
        }

        let mut last_error = String::new();

        for model in &models {
            tracing::info!("Trying whisper model: {model}");

            let result = tokio::time::timeout(
                std::time::Duration::from_secs(self.timeout_secs),
                tokio::process::Command::new("whisper")
                    .args([
                        audio_path.to_str().unwrap_or(""),
                        "--model",
                        model,
                        "--output_format",
                        "json",
                        "--output_dir",
                        output_dir.to_str().unwrap_or("."),
                        "--language",
                        "en",
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output(),
            )
            .await;

            let output = match result {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    last_error = format!("whisper process error: {e}");
                    tracing::warn!("Whisper model {model} failed: {last_error}");
                    continue;
                }
                Err(_) => {
                    last_error = format!("whisper model {model} timed out after {}s", self.timeout_secs);
                    tracing::warn!("{last_error}");
                    continue;
                }
            };

            if output.status.success() {
                let stem = audio_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("audio");
                let json_path = output_dir.join(format!("{stem}.json"));
                let json_content = tokio::fs::read_to_string(&json_path).await?;
                let parsed: serde_json::Value = serde_json::from_str(&json_content)?;

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
                                speaker: None, // whisper CLI has no diarization
                                confidence: seg["avg_logprob"].as_f64(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                // Clean up json file
                let _ = tokio::fs::remove_file(&json_path).await;

                return Ok(TranscriptionResult {
                    segments,
                    full_text,
                });
            }

            last_error = String::from_utf8_lossy(&output.stderr).to_string();
            tracing::warn!("Whisper model {model} failed, trying next...");
        }

        anyhow::bail!(
            "All whisper models failed. Last error: {}",
            last_error.chars().take(200).collect::<String>()
        )
    }

    fn supports_diarization(&self) -> bool {
        false
    }

    fn backend_name(&self) -> &'static str {
        "whisper-cli"
    }
}
