//! strivo-broll — B-roll asset suggestion from transcript topics.
//!
//! Promotes the first marketplace entry (broll-finder) from
//! "coming soon" to "ships today". The DAW-vision capability:
//! given a tagged local library of B-roll assets and a transcript's
//! topic timeline, suggest where each B-roll cut would best land in
//! the edit.
//!
//! Pure data + scoring. The library descriptor is JSON the streamer
//! curates by hand (or another plugin generates); inputs come from
//! Crunchr (topic timeline) and the editor (current EDL). No IO; no
//! filesystem dependence inside this crate.
//!
//! Algorithm:
//!
//!   1. Build a (lowercased, ascii-alphanumeric) keyword bag from each
//!      `TopicSlice.topics` plus its text fallback.
//!   2. For each asset, score against each slice via Jaccard-on-tags
//!      with a length-prior bias (longer tags = more distinctive →
//!      bigger weight).
//!   3. Return the top-K (asset, slice) pairs as [`BrollSuggestion`]s
//!      sorted by score desc, then time asc.
//!
//! Suppression rules:
//!   * Don't suggest the same asset twice within 60s.
//!   * Cap suggestions per asset to 3 across the whole stream.
//!   * Drop suggestions with score below `MIN_SCORE` (0.10) — too
//!     noisy.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Threshold below which suggestions are dropped.
pub const MIN_SCORE: f32 = 0.10;
/// Don't suggest the same asset twice within this many seconds.
pub const SAME_ASSET_COOLDOWN_SECS: f32 = 60.0;
/// Maximum number of times the same asset can be suggested.
pub const MAX_PER_ASSET: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrollAsset {
    pub id: String,
    pub path: String,
    pub duration_sec: f32,
    /// Free-form tag list. Words are matched case-insensitively after
    /// stripping non-alphanumerics.
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrollLibrary {
    pub assets: Vec<BrollAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicSlice {
    pub start_sec: f32,
    pub end_sec: f32,
    /// Top topics + text fallback feed the keyword bag.
    pub topics: Vec<String>,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrollSuggestion {
    pub time_sec: f32,
    pub asset_id: String,
    pub asset_path: String,
    pub duration_sec: f32,
    pub score: f32,
    pub matched_tags: Vec<String>,
}

/// Build a normalised keyword set from one or more strings. Used both
/// for transcript slices and asset tags, so a match is always
/// comparing apples to apples.
fn normalise_keywords<'a, I: IntoIterator<Item = &'a str>>(words: I) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in words {
        for tok in raw.split(|c: char| !c.is_ascii_alphanumeric()) {
            let lower = tok.to_ascii_lowercase();
            if lower.len() >= 3 {
                out.insert(lower);
            }
        }
    }
    out
}

/// Score one slice against one asset. Jaccard with a small length-
/// prior so longer tags don't get drowned out by short common words.
fn score_pair(slice_kws: &BTreeSet<String>, asset_kws: &BTreeSet<String>) -> (f32, Vec<String>) {
    if slice_kws.is_empty() || asset_kws.is_empty() {
        return (0.0, Vec::new());
    }
    let mut intersection: Vec<String> = Vec::new();
    let mut union_size = slice_kws.len() + asset_kws.len();
    let mut shared_len_sum = 0usize;
    for k in slice_kws {
        if asset_kws.contains(k) {
            intersection.push(k.clone());
            shared_len_sum += k.len();
            // both sides count it; union should count it once not twice
            union_size -= 1;
        }
    }
    if intersection.is_empty() {
        return (0.0, Vec::new());
    }
    let jaccard = intersection.len() as f32 / union_size.max(1) as f32;
    // Length prior: nudge upward when shared keywords are long.
    let avg_len = shared_len_sum as f32 / intersection.len() as f32;
    let length_boost = ((avg_len - 4.0).max(0.0) * 0.05).min(0.30);
    let score = (jaccard + length_boost).min(1.0);
    (score, intersection)
}

/// Top-level helper: build [`BrollSuggestion`]s from the library + the
/// slice timeline. Returns a vector sorted by time asc (for SPA
/// rendering); callers wanting score-desc can sort themselves.
pub fn suggest_brolls(
    slices: &[TopicSlice],
    library: &BrollLibrary,
    top_k: usize,
) -> Vec<BrollSuggestion> {
    if slices.is_empty() || library.assets.is_empty() || top_k == 0 {
        return Vec::new();
    }
    // Pre-compute keyword sets per asset.
    let asset_kws: Vec<BTreeSet<String>> = library
        .assets
        .iter()
        .map(|a| normalise_keywords(a.tags.iter().map(|s| s.as_str())))
        .collect();

    let mut all: Vec<BrollSuggestion> = Vec::new();
    for slice in slices {
        // Build the slice keyword bag from topics + text.
        let mut kws_iter: Vec<&str> = slice.topics.iter().map(|s| s.as_str()).collect();
        kws_iter.push(slice.text.as_str());
        let slice_kws = normalise_keywords(kws_iter);
        if slice_kws.is_empty() {
            continue;
        }
        for (asset, asset_set) in library.assets.iter().zip(asset_kws.iter()) {
            let (score, matched) = score_pair(&slice_kws, asset_set);
            if score < MIN_SCORE {
                continue;
            }
            all.push(BrollSuggestion {
                time_sec: slice.start_sec,
                asset_id: asset.id.clone(),
                asset_path: asset.path.clone(),
                duration_sec: asset.duration_sec,
                score,
                matched_tags: matched,
            });
        }
    }
    // Sort by score desc so the NMS pass keeps the strongest.
    all.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut picked: Vec<BrollSuggestion> = Vec::new();
    let mut count_per_asset: BTreeMap<String, usize> = BTreeMap::new();
    for cand in all {
        if picked.len() >= top_k {
            break;
        }
        let asset_count = count_per_asset.get(&cand.asset_id).copied().unwrap_or(0);
        if asset_count >= MAX_PER_ASSET {
            continue;
        }
        let too_close = picked.iter().any(|p| {
            p.asset_id == cand.asset_id
                && (p.time_sec - cand.time_sec).abs() < SAME_ASSET_COOLDOWN_SECS
        });
        if too_close {
            continue;
        }
        count_per_asset.insert(cand.asset_id.clone(), asset_count + 1);
        picked.push(cand);
    }
    // Final return chronological.
    picked.sort_by(|a, b| {
        a.time_sec
            .partial_cmp(&b.time_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, dur: f32, tags: &[&str]) -> BrollAsset {
        BrollAsset {
            id: id.into(),
            path: format!("/broll/{id}.mp4"),
            duration_sec: dur,
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn slice(start: f32, end: f32, topics: &[&str], text: &str) -> TopicSlice {
        TopicSlice {
            start_sec: start,
            end_sec: end,
            topics: topics.iter().map(|s| s.to_string()).collect(),
            text: text.into(),
        }
    }

    #[test]
    fn empty_inputs_yield_empty_suggestions() {
        let lib = BrollLibrary::default();
        assert!(suggest_brolls(&[], &lib, 10).is_empty());
        let slices = vec![slice(0.0, 30.0, &["diablo"], "")];
        assert!(suggest_brolls(&slices, &BrollLibrary::default(), 10).is_empty());
    }

    #[test]
    fn matches_asset_to_slice_by_topic() {
        let lib = BrollLibrary {
            assets: vec![asset("diablo-clip", 6.0, &["diablo", "speedrun"])],
        };
        let slices = vec![slice(0.0, 30.0, &["Diablo"], "world record attempt")];
        let suggestions = suggest_brolls(&slices, &lib, 10);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].asset_id, "diablo-clip");
        assert!(suggestions[0].score > 0.0);
        assert!(suggestions[0].matched_tags.contains(&"diablo".to_string()));
    }

    #[test]
    fn drops_suggestions_below_min_score() {
        // Asset tags share nothing with the slice topics → score 0,
        // dropped.
        let lib = BrollLibrary {
            assets: vec![asset("kitchen", 6.0, &["cooking", "pasta"])],
        };
        let slices = vec![slice(0.0, 30.0, &["Diablo"], "speedrun")];
        let suggestions = suggest_brolls(&slices, &lib, 10);
        assert!(suggestions.is_empty(), "got {suggestions:?}");
    }

    #[test]
    fn cooldown_prevents_same_asset_within_60s() {
        let lib = BrollLibrary {
            assets: vec![asset("diablo", 6.0, &["diablo"])],
        };
        let slices = vec![
            slice(0.0, 30.0, &["Diablo"], ""),
            slice(30.0, 60.0, &["Diablo"], ""),
            slice(120.0, 150.0, &["Diablo"], ""),
        ];
        let suggestions = suggest_brolls(&slices, &lib, 10);
        // First slice picks the asset; second falls inside cooldown;
        // third is past cooldown.
        assert_eq!(suggestions.len(), 2, "got {suggestions:?}");
        assert_eq!(suggestions[0].time_sec, 0.0);
        assert_eq!(suggestions[1].time_sec, 120.0);
    }

    #[test]
    fn max_per_asset_cap_respected() {
        let lib = BrollLibrary {
            assets: vec![asset("diablo", 6.0, &["diablo"])],
        };
        // 5 widely-spaced slices that would otherwise yield 5 picks.
        let slices: Vec<TopicSlice> = (0..5)
            .map(|i| slice((i as f32) * 200.0, (i as f32) * 200.0 + 30.0, &["Diablo"], ""))
            .collect();
        let suggestions = suggest_brolls(&slices, &lib, 10);
        assert!(suggestions.len() <= MAX_PER_ASSET, "got {suggestions:?}");
    }

    #[test]
    fn top_k_cap_respected() {
        let lib = BrollLibrary {
            assets: (0..10)
                .map(|i| asset(&format!("a{i}"), 6.0, &["diablo"]))
                .collect(),
        };
        let slices = vec![slice(0.0, 30.0, &["Diablo"], "")];
        let suggestions = suggest_brolls(&slices, &lib, 3);
        assert!(suggestions.len() <= 3);
    }

    #[test]
    fn suggestions_returned_in_chronological_order() {
        let lib = BrollLibrary {
            assets: vec![
                asset("a1", 6.0, &["alpha"]),
                asset("a2", 6.0, &["beta"]),
            ],
        };
        let slices = vec![
            slice(500.0, 530.0, &["Beta"], ""),
            slice(100.0, 130.0, &["Alpha"], ""),
        ];
        let suggestions = suggest_brolls(&slices, &lib, 10);
        assert_eq!(suggestions.len(), 2);
        assert!(suggestions[0].time_sec <= suggestions[1].time_sec);
    }

    #[test]
    fn keyword_normalisation_drops_short_tokens() {
        let kws = normalise_keywords(["a or to in".to_string()].iter().map(|s| s.as_str()));
        assert!(kws.is_empty(), "got {kws:?}");
    }

    #[test]
    fn keyword_normalisation_lowercases_and_strips_punct() {
        let kws = normalise_keywords(
            ["Diablo!!! Speedrun-Mode".to_string()]
                .iter()
                .map(|s| s.as_str()),
        );
        assert!(kws.contains("diablo"));
        assert!(kws.contains("speedrun"));
        assert!(kws.contains("mode"));
    }

    #[test]
    fn length_boost_lifts_long_keyword_match() {
        // Same Jaccard structure on both sides (2 keywords each, 1
        // shared). Matching the long word "speedrunning" should
        // out-score the short word "game" thanks to the length prior.
        let asset_long = asset("a-long", 6.0, &["speedrunning", "extra"]);
        let asset_short = asset("a-short", 6.0, &["game", "extra"]);
        let slice_long = slice(0.0, 30.0, &["speedrunning", "other"], "");
        let slice_short = slice(0.0, 30.0, &["game", "other"], "");
        let s_long = score_pair(
            &normalise_keywords(slice_long.topics.iter().map(|s| s.as_str())),
            &normalise_keywords(asset_long.tags.iter().map(|s| s.as_str())),
        ).0;
        let s_short = score_pair(
            &normalise_keywords(slice_short.topics.iter().map(|s| s.as_str())),
            &normalise_keywords(asset_short.tags.iter().map(|s| s.as_str())),
        ).0;
        assert!(s_long > s_short, "{s_long} > {s_short}");
    }

    #[test]
    fn matched_tags_returned_alphabetically() {
        let lib = BrollLibrary {
            assets: vec![asset("multi", 6.0, &["zelda", "diablo"])],
        };
        let slices = vec![slice(0.0, 30.0, &["zelda", "diablo"], "")];
        let s = &suggest_brolls(&slices, &lib, 10)[0];
        let mut expected = vec!["diablo".to_string(), "zelda".to_string()];
        expected.sort();
        assert_eq!(s.matched_tags, expected);
    }
}
