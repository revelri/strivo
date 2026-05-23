//! Translate an [`EdlDoc`] into a [`Pipeline`] for the host DAG engine.
//!
//! This is the one-way bridge: EDL is the user-facing serializable
//! artifact; Pipeline is the in-memory executable form. Round-trip
//! (Pipeline → EDL) is not needed today — the EDL is always authored
//! first.

use super::schema::{EdlDoc, EdlKind, EdlOp};
use crate::pipeline::stage::ResourceLock;
use crate::pipeline::{Pipeline, ResourceRegistry, Stage, StageId, StageKind};

/// Build a Pipeline from an EDL. The resulting graph is linear for the
/// MVP: each op becomes a stage whose input is the prior stage. Concat
/// fan-in is preserved via explicit input references.
///
/// The returned Pipeline is *not* submitted to the registry — the caller
/// (plugin) does that after attaching plugin-specific metadata
/// (cost estimates, fallback providers, resource locks).
pub fn pipeline_from_edl(doc: &EdlDoc, _resources: &ResourceRegistry) -> Pipeline {
    let mut p = Pipeline::new(format!("{}::{}", kind_label(&doc.kind), doc.name));
    // Map op-index → stage-id so Concat ops can reference earlier Clip
    // stages by index in the EDL.
    let mut op_to_stage: Vec<Option<StageId>> = Vec::with_capacity(doc.ops.len());
    let mut prev: Option<StageId> = None;

    for op in &doc.ops {
        let kind = match op {
            EdlOp::Extract => StageKind::Extract,
            EdlOp::Transcribe { provider, .. } => StageKind::Transcribe {
                provider: provider.clone(),
            },
            EdlOp::Diarize { provider, .. } => StageKind::Diarize {
                provider: provider.clone(),
            },
            EdlOp::Subtitle => StageKind::Subtitle,
            EdlOp::Analyze { provider, .. } => StageKind::Analyze {
                provider: provider.clone(),
            },
            EdlOp::Clip { .. } => StageKind::ExportClip,
            EdlOp::Concat { .. } => StageKind::Concat,
            EdlOp::Archive { .. } => StageKind::Archive,
            EdlOp::FilterSpeaker { .. } => StageKind::Custom("filter_speaker".into()),
        };

        let inputs = match op {
            // Concat fans in from referenced Clip stages.
            EdlOp::Concat { clips, .. } => clips
                .iter()
                .filter_map(|&i| op_to_stage.get(i).and_then(|x| *x))
                .collect(),
            // Everything else chains linearly on the previous stage.
            _ => prev.iter().copied().collect(),
        };

        let stage = Stage::new(op_label(op), kind).with_inputs(inputs);
        let id = p.add_stage(stage);
        op_to_stage.push(Some(id));
        // Concat is a terminal join; downstream chains keep flowing from it.
        prev = Some(id);
    }
    p
}

/// Build a [`Pipeline`] from a Crunchr preset name + list of input
/// recordings. Plugins land on this when fanning a batch out across
/// the host DAG engine. (C1 phase 2.)
///
/// For each input we emit a per-input chain:
///
/// ```text
///   extract → transcribe(provider) → [diarize(provider)?] →
///   subtitle? → [analyze(provider, model)?]
/// ```
///
/// `transcribe(whisperx-local)` and `transcribe(voxtral-local)` carry
/// a [`ResourceLock::Gpu`] so two such stages can't run in parallel
/// across pipelines. `analyze` carries a bounded
/// [`ResourceLock::Api`] for the OpenRouter quota.
///
/// `preset_stages` is the typed sequence built from
/// [`CrunchrPreset::stages`]; the caller flattens that out of the
/// plugin crate before calling us so this crate doesn't grow a
/// strivo-plugins dependency.
pub fn pipeline_from_preset_stages(
    preset_name: &str,
    inputs: &[(String, std::path::PathBuf)],
    preset_stages: &[PresetStageBridge],
) -> Pipeline {
    let mut p = Pipeline::new(format!("crunchr::{preset_name}"));
    for (vod_id, _path) in inputs {
        let mut prev: Option<StageId> = None;
        // Every input chain starts with audio extraction.
        let extract = Stage::new(
            format!("extract:{vod_id}"),
            StageKind::Extract,
        )
        .with_inputs(prev.iter().copied().collect());
        let extract_id = p.add_stage(extract);
        prev = Some(extract_id);

        for stage in preset_stages {
            let (kind, requires) = match stage {
                PresetStageBridge::Transcribe { provider, max_attempts: _ } => {
                    let mut locks = Vec::new();
                    if provider.contains("local") {
                        locks.push(ResourceLock::Gpu);
                    } else if provider.contains("openrouter") || provider.contains("voxtral-api") {
                        locks.push(ResourceLock::Api {
                            name: "openrouter".into(),
                            cap: 8,
                        });
                    }
                    (
                        StageKind::Transcribe {
                            provider: provider.clone(),
                        },
                        locks,
                    )
                }
                PresetStageBridge::Diarize { provider, .. } => {
                    let mut locks = Vec::new();
                    if provider.contains("local") {
                        locks.push(ResourceLock::Gpu);
                    }
                    (
                        StageKind::Diarize {
                            provider: provider.clone(),
                        },
                        locks,
                    )
                }
                PresetStageBridge::Subtitle => (StageKind::Subtitle, Vec::new()),
                PresetStageBridge::Analyze {
                    provider, model, ..
                } => (
                    StageKind::Analyze {
                        provider: format!("{provider}:{model}"),
                    },
                    vec![ResourceLock::Api {
                        name: "openrouter".into(),
                        cap: 4,
                    }],
                ),
            };
            let max_attempts = match stage {
                PresetStageBridge::Transcribe { max_attempts, .. }
                | PresetStageBridge::Diarize { max_attempts, .. }
                | PresetStageBridge::Analyze { max_attempts, .. } => *max_attempts,
                PresetStageBridge::Subtitle => 1,
            };
            let s = Stage::new(format!("{}:{}", stage_label(stage), vod_id), kind)
                .with_inputs(prev.iter().copied().collect())
                .with_max_attempts(max_attempts)
                .with_requires(requires);
            prev = Some(p.add_stage(s));
        }
    }
    p
}

/// Stage shape consumed by [`pipeline_from_preset_stages`]. This is
/// the data crate's mirror of `strivo_plugins::crunchr::presets::CrunchrStage`;
/// the plugin builds and passes one of these per stage so we don't
/// pull strivo-plugins into the host crate.
#[derive(Debug, Clone)]
pub enum PresetStageBridge {
    Transcribe {
        provider: String,
        max_attempts: u8,
    },
    Diarize {
        provider: String,
        max_attempts: u8,
    },
    Subtitle,
    Analyze {
        provider: String,
        model: String,
        max_attempts: u8,
    },
}

fn stage_label(s: &PresetStageBridge) -> &'static str {
    match s {
        PresetStageBridge::Transcribe { .. } => "transcribe",
        PresetStageBridge::Diarize { .. } => "diarize",
        PresetStageBridge::Subtitle => "subtitle",
        PresetStageBridge::Analyze { .. } => "analyze",
    }
}

fn kind_label(k: &EdlKind) -> &'static str {
    match k {
        EdlKind::Batch => "batch",
        EdlKind::Edit => "edit",
        EdlKind::Preset => "preset",
    }
}

fn op_label(op: &EdlOp) -> String {
    match op {
        EdlOp::Extract => "extract".into(),
        EdlOp::Transcribe { provider, .. } => format!("transcribe:{provider}"),
        EdlOp::Diarize { provider, .. } => format!("diarize:{provider}"),
        EdlOp::Subtitle => "subtitle".into(),
        EdlOp::Analyze { provider, .. } => format!("analyze:{provider}"),
        EdlOp::Clip { label, .. } => {
            if label.is_empty() {
                "clip".into()
            } else {
                format!("clip:{label}")
            }
        }
        EdlOp::Concat { output, .. } => format!("concat→{output}"),
        EdlOp::Archive { .. } => "archive".into(),
        EdlOp::FilterSpeaker { .. } => "filter-speaker".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edl::schema::*;

    #[test]
    fn linear_pipeline_from_preset() {
        let doc = EdlDoc {
            version: EDL_VERSION,
            kind: EdlKind::Preset,
            name: "p".into(),
            inputs: vec![],
            ops: vec![
                EdlOp::Extract,
                EdlOp::Transcribe {
                    provider: "whisper-cli".into(),
                    params: Default::default(),
                },
                EdlOp::Subtitle,
            ],
            created_at: "now".into(),
            created_by: "test".into(),
        };
        let p = pipeline_from_edl(&doc, &ResourceRegistry::new());
        assert_eq!(p.stages.len(), 3);
        // Linear chain: stage 1 depends on stage 0, stage 2 on stage 1.
        assert!(p.stages[0].inputs.is_empty());
        assert_eq!(p.stages[1].inputs, vec![p.stages[0].id]);
        assert_eq!(p.stages[2].inputs, vec![p.stages[1].id]);
        assert!(p.assert_acyclic().is_ok());
    }

    #[test]
    fn preset_bridge_fans_per_input() {
        let inputs = vec![
            ("rec-1".to_string(), std::path::PathBuf::from("/tmp/a.mkv")),
            ("rec-2".to_string(), std::path::PathBuf::from("/tmp/b.mkv")),
        ];
        let stages = vec![
            PresetStageBridge::Transcribe {
                provider: "whisperx-local".into(),
                max_attempts: 3,
            },
            PresetStageBridge::Diarize {
                provider: "whisperx-local".into(),
                max_attempts: 2,
            },
            PresetStageBridge::Subtitle,
        ];
        let p = pipeline_from_preset_stages("quality-local", &inputs, &stages);
        // 2 inputs × (extract + 3 preset stages) = 8 stages.
        assert_eq!(p.stages.len(), 8);
        // Within each input, the chain is linear.
        let rec1_extract = &p.stages[0];
        let rec1_transcribe = &p.stages[1];
        assert!(rec1_extract.inputs.is_empty());
        assert_eq!(rec1_transcribe.inputs, vec![rec1_extract.id]);
        // GPU lock present on the local transcribe stage.
        assert!(rec1_transcribe
            .requires
            .iter()
            .any(|r| matches!(r, ResourceLock::Gpu)));
        assert!(p.assert_acyclic().is_ok());
    }

    #[test]
    fn preset_bridge_api_provider_uses_api_lock() {
        let inputs = vec![("rec".into(), std::path::PathBuf::from("/tmp/x.mkv"))];
        let stages = vec![PresetStageBridge::Transcribe {
            provider: "voxtral-openrouter".into(),
            max_attempts: 3,
        }];
        let p = pipeline_from_preset_stages("quality-api", &inputs, &stages);
        let transcribe = &p.stages[1]; // 0 is extract
        assert!(transcribe
            .requires
            .iter()
            .any(|r| matches!(r, ResourceLock::Api { name, .. } if name == "openrouter")));
    }

    #[test]
    fn concat_fans_in_from_clips() {
        let doc = EdlDoc {
            version: EDL_VERSION,
            kind: EdlKind::Edit,
            name: "highlight-reel".into(),
            inputs: vec![EdlInput {
                vod_id: "x".into(),
                path: None,
            }],
            ops: vec![
                EdlOp::Clip {
                    in_word: 0,
                    out_word: 10,
                    label: "a".into(),
                },
                EdlOp::Clip {
                    in_word: 20,
                    out_word: 30,
                    label: "b".into(),
                },
                EdlOp::Concat {
                    clips: vec![0, 1],
                    output: "out.mkv".into(),
                },
            ],
            created_at: "now".into(),
            created_by: "test".into(),
        };
        let p = pipeline_from_edl(&doc, &ResourceRegistry::new());
        assert_eq!(p.stages.len(), 3);
        // Concat depends on both Clip stages.
        let concat_inputs = &p.stages[2].inputs;
        assert_eq!(concat_inputs.len(), 2);
        assert!(concat_inputs.contains(&p.stages[0].id));
        assert!(concat_inputs.contains(&p.stages[1].id));
    }
}
