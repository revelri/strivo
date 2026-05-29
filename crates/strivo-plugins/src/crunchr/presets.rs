//! Crunchr wiring presets — provider chains saved as TOML.
//!
//! A preset is a named sequence of [`CrunchrStage`]s, each picking a
//! backend + params for one verb (Transcribe / Diarize / Subtitle /
//! Analyze). Users pick a preset when submitting a recording for
//! processing instead of relying on the monolithic `CrunchrConfig.backend`
//! field.
//!
//! Presets are loadable from `~/.config/strivo/crunchr/presets/*.toml` and
//! three defaults ship in-binary so the user has working choices on day
//! one.
//!
//! M4 MVP scope: the preset is the data model + load/save + EDL builder.
//! The in-flight pipeline.rs state machine still drives execution today;
//! a follow-up commit (C1 phase 2) refactors that to consume preset
//! stages directly via the host DAG engine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default preset name applied when the user submits a job without
/// picking one. Mirrors the historical `CrunchrConfig.backend = "whisper-cli"`
/// behavior so existing configs upgrade transparently.
pub const DEFAULT_PRESET_NAME: &str = "fast-cheap";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CrunchrStage {
    /// Transcribe via the named backend (whisper-cli, whisperx-local,
    /// voxtral-api, voxtral-local, voxtral-openrouter).
    Transcribe {
        provider: String,
        #[serde(default)]
        params: BTreeMap<String, toml::Value>,
        #[serde(default = "default_max_attempts")]
        max_attempts: u8,
        #[serde(default)]
        fallback_provider: Option<String>,
    },
    /// Diarize an existing transcript. `provider` is the diarization
    /// backend (typically `whisperx_local` or `voxtral_api`). Stages
    /// without an upstream Transcribe in the same preset get rejected at
    /// validation time.
    Diarize {
        provider: String,
        #[serde(default = "default_max_attempts")]
        max_attempts: u8,
    },
    /// Emit `.vtt` and `.srt` subtitle sidecars from the transcript.
    Subtitle,
    /// LLM-driven analysis (summary, topics, sentiment). `model` is an
    /// OpenRouter / provider model name; the existing `analysis.rs`
    /// dispatcher routes by prefix.
    Analyze {
        provider: String,
        model: String,
        #[serde(default = "default_max_attempts")]
        max_attempts: u8,
    },
}

const fn default_max_attempts() -> u8 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrunchrPreset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub stages: Vec<CrunchrStage>,
}

impl CrunchrPreset {
    /// Three first-party presets shipped in-binary so the user has
    /// working choices without writing any TOML.
    pub fn builtins() -> Vec<Self> {
        vec![
            CrunchrPreset {
                name: "fast-cheap".into(),
                description: "Local whisper-cli transcription only. \
                              No diarization, no LLM analysis. Costs $0."
                    .into(),
                stages: vec![
                    CrunchrStage::Transcribe {
                        provider: "whisper-cli".into(),
                        params: Default::default(),
                        max_attempts: 3,
                        fallback_provider: None,
                    },
                    CrunchrStage::Subtitle,
                ],
            },
            CrunchrPreset {
                name: "quality-local".into(),
                description: "WhisperX with word-level alignment + local \
                              pyannote diarization. Requires GPU; costs $0."
                    .into(),
                stages: vec![
                    CrunchrStage::Transcribe {
                        provider: "whisperx-local".into(),
                        params: Default::default(),
                        max_attempts: 3,
                        fallback_provider: Some("whisper-cli".into()),
                    },
                    CrunchrStage::Diarize {
                        provider: "whisperx-local".into(),
                        max_attempts: 2,
                    },
                    CrunchrStage::Subtitle,
                ],
            },
            CrunchrPreset {
                name: "quality-api".into(),
                description: "Voxtral API for transcription + speaker \
                              diarization. Pay-per-minute; no local GPU \
                              needed."
                    .into(),
                stages: vec![
                    CrunchrStage::Transcribe {
                        provider: "voxtral-api".into(),
                        params: Default::default(),
                        max_attempts: 3,
                        fallback_provider: Some("voxtral-openrouter".into()),
                    },
                    CrunchrStage::Subtitle,
                ],
            },
        ]
    }

    /// Validate preset structure: at least one stage, Diarize/Analyze
    /// must follow a Transcribe.
    pub fn validate(&self) -> Result<()> {
        if self.stages.is_empty() {
            anyhow::bail!("preset '{}' has no stages", self.name);
        }
        let mut transcribed = false;
        for stage in &self.stages {
            match stage {
                CrunchrStage::Transcribe { .. } => transcribed = true,
                CrunchrStage::Diarize { .. } | CrunchrStage::Analyze { .. } => {
                    if !transcribed {
                        anyhow::bail!(
                            "preset '{}': {:?} stage needs a Transcribe earlier in the chain",
                            self.name,
                            stage
                        );
                    }
                }
                CrunchrStage::Subtitle => {}
            }
        }
        Ok(())
    }
}

/// Resolve the user's preset directory. Honors `STRIVO_CONFIG_DIR` for
/// tests; falls back to `~/.config/strivo/crunchr/presets/`.
pub fn presets_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("STRIVO_CONFIG_DIR") {
        return PathBuf::from(custom).join("crunchr").join("presets");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("strivo")
        .join("crunchr")
        .join("presets")
}

/// Load every preset on disk. Validation failures are logged and the
/// preset is skipped (one bad file doesn't shadow the whole library).
pub fn load_all() -> Vec<CrunchrPreset> {
    let dir = presets_dir();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        match load_file(&path) {
            Ok(p) => out.push(p),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "preset load failed");
            }
        }
    }
    out
}

pub fn load_file(path: &Path) -> Result<CrunchrPreset> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let preset: CrunchrPreset =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    preset
        .validate()
        .with_context(|| format!("validate {}", path.display()))?;
    Ok(preset)
}

pub fn save(preset: &CrunchrPreset) -> Result<PathBuf> {
    preset.validate()?;
    let dir = presets_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.toml", preset.name));
    let tmp = path.with_extension("tmp");
    let text = toml::to_string_pretty(preset)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Merge built-in presets with user-authored ones. Names collide → user
/// wins (so users can override `fast-cheap` with their own version).
pub fn library() -> Vec<CrunchrPreset> {
    let user = load_all();
    let user_names: std::collections::HashSet<String> =
        user.iter().map(|p| p.name.clone()).collect();
    let mut out: Vec<CrunchrPreset> = CrunchrPreset::builtins()
        .into_iter()
        .filter(|p| !user_names.contains(&p.name))
        .collect();
    out.extend(user);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_validate() {
        for p in CrunchrPreset::builtins() {
            p.validate().expect("builtin preset must validate");
        }
    }

    #[test]
    fn diarize_without_transcribe_rejected() {
        let p = CrunchrPreset {
            name: "bad".into(),
            description: String::new(),
            stages: vec![CrunchrStage::Diarize {
                provider: "whisperx-local".into(),
                max_attempts: 1,
            }],
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn empty_preset_rejected() {
        let p = CrunchrPreset {
            name: "empty".into(),
            description: String::new(),
            stages: vec![],
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn preset_roundtrip_toml() {
        let p = CrunchrPreset::builtins().into_iter().next().unwrap();
        let text = toml::to_string_pretty(&p).unwrap();
        let parsed: CrunchrPreset = toml::from_str(&text).unwrap();
        assert_eq!(parsed.name, p.name);
        assert_eq!(parsed.stages.len(), p.stages.len());
    }
}
