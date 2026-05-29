//! `whisperx-local` transcription backend.
//!
//! Shells out to the bundled `scripts/whisperx_diarize.py` orchestrator,
//! which runs Whisper + pyannote in a two-stage VRAM-aware pipeline so a
//! diarized transcript fits on an 8 GB GPU.
//!
//! Discovery rules for the Python script and interpreter, in order:
//!
//! 1. `STRIVO_WHISPERX_SCRIPT` env var — full path to a `.py` file.
//! 2. The script that ships next to this crate at `scripts/whisperx_diarize.py`
//!    relative to either `CARGO_MANIFEST_DIR` (dev) or the current binary's
//!    parent (release / installed).
//! 3. A user override at `~/.config/strivo/whisperx_diarize.py`.
//!
//! Python interpreter: `STRIVO_WHISPERX_PYTHON` env var, falling back to
//! `python3` then `python` on PATH.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{TranscriptionBackend, TranscriptionResult};
use crate::crunchr::types::Segment;

/// Backend that delegates to the bundled WhisperX orchestrator.
pub struct WhisperxLocalBackend {
    timeout_secs: u64,
    diarize: bool,
}

impl WhisperxLocalBackend {
    pub fn new(timeout_secs: u64, diarize: bool) -> Self {
        Self {
            timeout_secs,
            diarize,
        }
    }
}

/// Best-effort lookup of the Python interpreter to launch.
fn pick_python() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("STRIVO_WHISPERX_PYTHON") {
        let p = PathBuf::from(env);
        if p.exists() {
            return Some(p);
        }
    }
    for candidate in ["python3", "python"] {
        if let Ok(found) = which::which(candidate) {
            return Some(found);
        }
    }
    None
}

/// Best-effort lookup of the orchestrator script. Returns `None` only if
/// neither the bundled copy nor a user override exists.
fn pick_script() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("STRIVO_WHISPERX_SCRIPT") {
        let p = PathBuf::from(env);
        if p.exists() {
            return Some(p);
        }
    }
    // Dev tree: cargo-resolved manifest directory.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("whisperx_diarize.py");
    if manifest.exists() {
        return Some(manifest);
    }
    // Installed binary: sibling of the running executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sib = parent.join("whisperx_diarize.py");
            if sib.exists() {
                return Some(sib);
            }
        }
    }
    // User override.
    if let Some(home) = dirs_home() {
        let cfg = home
            .join(".config")
            .join("strivo")
            .join("whisperx_diarize.py");
        if cfg.exists() {
            return Some(cfg);
        }
    }
    None
}

/// Tiny stand-in for the `directories` crate so we don't add a dep just for
/// `$HOME` lookup. The HOME env var is set on every supported platform.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Probe whether the backend is launchable. Used by the host pane to decide
/// whether to surface the "no transcription backend" hint.
pub fn is_available() -> bool {
    pick_python().is_some() && pick_script().is_some()
}

#[async_trait]
impl TranscriptionBackend for WhisperxLocalBackend {
    async fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult> {
        let python = pick_python().context(
            "whisperx-local: no Python interpreter found (set STRIVO_WHISPERX_PYTHON or install python3)",
        )?;
        let script = pick_script().context(
            "whisperx-local: orchestrator script missing (expected scripts/whisperx_diarize.py or STRIVO_WHISPERX_SCRIPT)",
        )?;

        // Write result alongside the input WAV so the pipeline's existing
        // post-transcription cleanup catches the temp file.
        let output_dir = audio_path.parent().unwrap_or(Path::new("."));
        let stem = audio_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("audio");
        let result_path = output_dir.join(format!("{stem}.whisperx.json"));

        let mut cmd = tokio::process::Command::new(&python);
        cmd.arg(&script)
            .arg(audio_path)
            .arg(&result_path)
            .arg(if self.diarize {
                "--diarize"
            } else {
                "--no-diarize"
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        tracing::info!(
            "whisperx-local: launching {} {} {} {} ({})",
            python.display(),
            script.display(),
            audio_path.display(),
            result_path.display(),
            if self.diarize {
                "diarize=on"
            } else {
                "diarize=off"
            }
        );

        let started = std::time::Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            cmd.output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("whisperx-local: timed out after {}s", self.timeout_secs))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "whisperx-local: exit {} after {:.1}s: {}",
                output.status,
                started.elapsed().as_secs_f64(),
                stderr.chars().take(500).collect::<String>()
            );
        }

        let raw = tokio::fs::read_to_string(&result_path)
            .await
            .with_context(|| format!("reading {}", result_path.display()))?;
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).context("whisperx-local: malformed result JSON")?;
        let _ = tokio::fs::remove_file(&result_path).await;

        let full_text = parsed["text"].as_str().unwrap_or("").trim().to_string();
        let segments = parsed["segments"]
            .as_array()
            .map(|segs| {
                segs.iter()
                    .enumerate()
                    .map(|(i, seg)| {
                        // whisperx emits word-level timings in seg["words"];
                        // preserve them for the Editor plugin (C5).
                        let words: Option<Vec<crate::crunchr::types::WordTiming>> =
                            seg["words"].as_array().map(|ws| {
                                ws.iter()
                                    .map(|w| crate::crunchr::types::WordTiming {
                                        w: w["word"].as_str().unwrap_or("").to_string(),
                                        s: w["start"].as_f64().unwrap_or(0.0),
                                        e: w["end"].as_f64().unwrap_or(0.0),
                                        c: w["score"].as_f64().or_else(|| w["confidence"].as_f64()),
                                    })
                                    .collect()
                            });
                        Segment {
                            index: seg["index"].as_u64().map(|n| n as usize).unwrap_or(i),
                            start_sec: seg["start"].as_f64().unwrap_or(0.0),
                            end_sec: seg["end"].as_f64().unwrap_or(0.0),
                            text: seg["text"].as_str().unwrap_or("").trim().to_string(),
                            speaker: seg["speaker"].as_str().map(String::from),
                            confidence: seg["confidence"].as_f64(),
                            words,
                        }
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
        self.diarize
    }

    fn backend_name(&self) -> &'static str {
        "whisperx-local"
    }
}
