//! strivo-pipelines-dag — cross-plugin pipeline DAG composer.
//!
//! The Pipelines page was empty: it had no data source. The DAW
//! vision wants this surface alive — streamers should be able to
//! see the chain of plugins that turns a fresh recording into a
//! published artefact. This crate composes the well-known pipelines
//! from the capability tags every plugin already declares (iter 1)
//! and lays them out for rendering.
//!
//! Pure data:
//!
//!   * [`Node`] — one plugin in a pipeline, with the capabilities it
//!     produces / consumes.
//!   * [`Edge`] — directional link `from -> to` tagged with the
//!     capability that flows along it.
//!   * [`Pipeline`] — named DAG (id, label, description, nodes,
//!     edges).
//!   * [`default_pipelines`] — the four canonical pipelines the SPA
//!     renders today.
//!   * [`validate`] — every edge endpoint must exist in `nodes`, and
//!     `produces`/`consumes` lists must line up (each edge's tag must
//!     appear in the `from` node's produces and the `to` node's
//!     consumes). Catches typos at unit-test time.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    /// Capability tags this node produces (matches the well-known
    /// constants in `strivo_core::plugin::capability`).
    pub produces: Vec<String>,
    pub consumes: Vec<String>,
    /// One-line description for the SPA chip tooltip.
    pub blurb: String,
    /// Shipping status — "available" for plugins that exist today,
    /// "roadmap" for slots the DAW-vision plan reserves.
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Build the canonical pipeline set. Strings are owned (cheap clones
/// on every call) so the SPA can mutate freely without changing the
/// crate's contract.
pub fn default_pipelines() -> Vec<Pipeline> {
    vec![
        clip_mining_pipeline(),
        publish_prep_pipeline(),
        analytics_pipeline(),
        captions_pipeline(),
    ]
}

fn n(id: &str, label: &str, produces: &[&str], consumes: &[&str], blurb: &str, status: &str) -> Node {
    Node {
        id: id.into(),
        label: label.into(),
        produces: produces.iter().map(|s| s.to_string()).collect(),
        consumes: consumes.iter().map(|s| s.to_string()).collect(),
        blurb: blurb.into(),
        status: status.into(),
    }
}

fn e(from: &str, to: &str, via: &str) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        via: via.into(),
    }
}

fn clip_mining_pipeline() -> Pipeline {
    Pipeline {
        id: "clip_mining".into(),
        name: "Clip mining".into(),
        description:
            "Recording → cuepoint extraction → highlight detection → clip cuts → thumbnail candidates.".into(),
        nodes: vec![
            n("recording", "Recording",
              &["recording"], &[],
              "Source VOD captured by StriVo core.", "available"),
            n("cuepoints", "Cuepoints",
              &["scene_detection"], &["recording"],
              "ffmpeg scene-change cuepoints.", "available"),
            n("clipper", "Clipper",
              &["highlight_detection", "clip_extraction"], &["scene_detection"],
              "Highlight scoring + clip cuts (lossless).", "available"),
            n("thumbnails", "Thumbnails",
              &["thumbnail_ranking"], &["highlight_detection"],
              "Frame-saliency ranking + 9:16 facecam crops.", "available"),
        ],
        edges: vec![
            e("recording", "cuepoints", "recording"),
            e("cuepoints", "clipper", "scene_detection"),
            e("clipper", "thumbnails", "highlight_detection"),
        ],
    }
}

fn publish_prep_pipeline() -> Pipeline {
    Pipeline {
        id: "publish_prep".into(),
        name: "Publish prep".into(),
        description:
            "Crunchr transcript → chapters / brand-safety / cross-format draft set → Casebook briefing.".into(),
        nodes: vec![
            n("crunchr", "Crunchr",
              &["transcription", "diarisation", "word_timestamps", "topic_segmentation"], &["recording"],
              "Whisper-driven transcript + topic segmentation.", "available"),
            n("chapters", "Chapters",
              &["chapters"], &["transcription", "topic_segmentation"],
              "Heuristic YouTube/Twitch chapter markers.", "available"),
            n("brandsafe", "Brandsafe",
              &["brand_safety"], &["transcription"],
              "Slur / profanity / restricted-game / music-mention scan.", "available"),
            n("reuse", "Reuse",
              &["publish_queue"], &["transcription", "chapters", "highlight_detection"],
              "Cross-format draft set (YT long / Shorts / TikTok / Patreon / podcast / blog).", "available"),
            n("casebook", "Casebook",
              &["reporting"], &["transcription", "chapters", "highlight_detection", "brand_safety", "fraud_detection"],
              "Markdown briefing fusing every upstream signal.", "available"),
        ],
        edges: vec![
            e("crunchr", "chapters", "transcription"),
            e("crunchr", "brandsafe", "transcription"),
            e("crunchr", "reuse", "transcription"),
            e("chapters", "reuse", "chapters"),
            e("crunchr", "casebook", "transcription"),
            e("chapters", "casebook", "chapters"),
            e("brandsafe", "casebook", "brand_safety"),
        ],
    }
}

fn analytics_pipeline() -> Pipeline {
    Pipeline {
        id: "analytics".into(),
        name: "Audience analytics".into(),
        description:
            "Insights compare + retention overlay + multi-signal heatmap + Viewguard cross-stream trend.".into(),
        nodes: vec![
            n("insights", "Insights",
              &["stream_comparison", "audience_retention"], &["transcription"],
              "Word freq + topics + stream comparison + per-recording retention proxy.", "available"),
            n("heatmap", "Heatmap",
              &["audience_retention"], &["transcription", "scene_detection", "highlight_detection", "brand_safety"],
              "Multi-signal retention overlay (talk + action + highlight − brandsafe).", "available"),
            n("viewguard", "Viewguard",
              &["fraud_detection"], &[],
              "Per-stream viewbot verdicts + cross-stream trend dashboard.", "available"),
        ],
        edges: vec![],
    }
}

fn captions_pipeline() -> Pipeline {
    Pipeline {
        id: "captions".into(),
        name: "Captions & translation".into(),
        description: "Crunchr → caption file generation in SRT / VTT / TXT with translator-trait backend.".into(),
        nodes: vec![
            n("crunchr_c", "Crunchr",
              &["transcription"], &["recording"],
              "Transcript source.", "available"),
            n("captions", "Captions",
              &["captions", "translation"], &["transcription"],
              "SRT / VTT / TXT export + pluggable translator (identity today, NLLB next).", "available"),
        ],
        edges: vec![e("crunchr_c", "captions", "transcription")],
    }
}

/// Sanity-check a pipeline: every edge endpoint must resolve to a node
/// in `nodes`, and the edge's `via` capability must appear in both the
/// producer's `produces` and the consumer's `consumes`.
pub fn validate(p: &Pipeline) -> Result<(), String> {
    use std::collections::HashMap;
    let by_id: HashMap<&str, &Node> = p.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for e in &p.edges {
        let from = by_id
            .get(e.from.as_str())
            .ok_or_else(|| format!("edge {} -> {} : unknown from", e.from, e.to))?;
        let to = by_id
            .get(e.to.as_str())
            .ok_or_else(|| format!("edge {} -> {} : unknown to", e.from, e.to))?;
        if !from.produces.iter().any(|c| c == &e.via) {
            return Err(format!(
                "edge {} -> {} via '{}': producer doesn't list it",
                e.from, e.to, e.via
            ));
        }
        if !to.consumes.iter().any(|c| c == &e.via) {
            return Err(format!(
                "edge {} -> {} via '{}': consumer doesn't list it",
                e.from, e.to, e.via
            ));
        }
    }
    Ok(())
}

/// Topological order of a pipeline's nodes (used by the SPA renderer
/// to lay nodes left-to-right). Kahn's algorithm; cycles produce an
/// Err. Pure pipelines have no cycles by construction.
pub fn topo_order(p: &Pipeline) -> Result<Vec<String>, String> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let mut in_deg: HashMap<&str, u32> = p.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut out_edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &p.edges {
        *in_deg.entry(e.to.as_str()).or_insert(0) += 1;
        out_edges.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }
    let mut queue: VecDeque<&str> = in_deg
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(k, _)| *k)
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        out.push(id.to_string());
        if let Some(succs) = out_edges.get(id) {
            for succ in succs {
                if let Some(d) = in_deg.get_mut(succ) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(*succ);
                    }
                }
            }
        }
    }
    if out.len() != p.nodes.len() {
        return Err(format!(
            "pipeline {} has a cycle: only ordered {}/{} nodes",
            p.id,
            out.len(),
            p.nodes.len()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pipelines_validate() {
        for p in default_pipelines() {
            validate(&p).unwrap_or_else(|e| panic!("pipeline {} fails validation: {e}", p.id));
        }
    }

    #[test]
    fn default_pipelines_have_unique_ids() {
        let ps = default_pipelines();
        let mut ids: Vec<&str> = ps.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn topo_order_returns_full_set_for_acyclic() {
        let p = clip_mining_pipeline();
        let order = topo_order(&p).unwrap();
        assert_eq!(order.len(), p.nodes.len());
        // Source must come before consumers.
        let pos = |id: &str| order.iter().position(|s| s == id).unwrap();
        assert!(pos("recording") < pos("cuepoints"));
        assert!(pos("cuepoints") < pos("clipper"));
        assert!(pos("clipper") < pos("thumbnails"));
    }

    #[test]
    fn topo_order_handles_pipeline_with_no_edges() {
        let p = analytics_pipeline(); // analytics has no edges between its nodes
        let order = topo_order(&p).unwrap();
        assert_eq!(order.len(), p.nodes.len());
    }

    #[test]
    fn validate_catches_unknown_edge_endpoint() {
        let mut p = clip_mining_pipeline();
        p.edges.push(e("ghost", "clipper", "scene_detection"));
        assert!(validate(&p).is_err());
    }

    #[test]
    fn validate_catches_mismatched_capability_tag() {
        let mut p = clip_mining_pipeline();
        p.edges.push(e("recording", "clipper", "not_a_real_cap"));
        assert!(validate(&p).is_err());
    }

    #[test]
    fn validate_catches_consumer_not_listing_capability() {
        // Synthesise a pipeline where the from-node produces something
        // the to-node never says it consumes.
        let p = Pipeline {
            id: "synth".into(),
            name: "Synth".into(),
            description: "".into(),
            nodes: vec![
                n("a", "A", &["transcription"], &[], "", "available"),
                n("b", "B", &[], &["scene_detection"], "", "available"),
            ],
            edges: vec![e("a", "b", "transcription")],
        };
        let err = validate(&p).unwrap_err();
        assert!(err.contains("consumer doesn't list it"), "got {err}");
    }

    #[test]
    fn topo_order_errors_on_cycle() {
        let p = Pipeline {
            id: "cycle".into(),
            name: "Cycle".into(),
            description: "".into(),
            nodes: vec![
                n("a", "A", &["transcription"], &["recording"], "", "available"),
                n("b", "B", &["recording"], &["transcription"], "", "available"),
            ],
            edges: vec![
                e("a", "b", "transcription"),
                e("b", "a", "recording"),
            ],
        };
        assert!(topo_order(&p).is_err());
    }

    #[test]
    fn publish_prep_includes_casebook_with_multiple_predecessors() {
        let p = publish_prep_pipeline();
        let casebook_predecessors: Vec<&str> = p
            .edges
            .iter()
            .filter(|e| e.to == "casebook")
            .map(|e| e.from.as_str())
            .collect();
        assert!(casebook_predecessors.len() >= 3, "got {casebook_predecessors:?}");
    }

    #[test]
    fn clip_mining_pipeline_only_uses_available_status() {
        let p = clip_mining_pipeline();
        for node in &p.nodes {
            assert_eq!(node.status, "available", "node {} expected available", node.id);
        }
    }
}
