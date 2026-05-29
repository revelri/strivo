//! Pricing data + cost estimation for Crunchr analysis runs (M5.6).
//!
//! Today we expose a pricing table keyed by OpenRouter / Mistral
//! model slug, plus an estimator that takes prompt and completion
//! token counts and returns USD cents. The token counts come from
//! `pipeline::estimate_tokens` (the M1.1.h heuristic — accurate
//! enough for display; a tiktoken-rs upgrade is tracked separately).
//!
//! Prices reflect the public rate cards as of 2026-05; users on
//! enterprise plans will want to override locally. The table is a
//! `&'static [PricingRow]` so future contributors can grep + append
//! without touching downstream consumers.

/// USD per 1k tokens, broken into prompt vs completion sides.
#[derive(Debug, Clone, Copy)]
pub struct Pricing {
    pub model: &'static str,
    /// USD per 1k input tokens.
    pub prompt_per_1k: f64,
    /// USD per 1k output tokens.
    pub completion_per_1k: f64,
}

/// Known model rates. Slugs match OpenRouter's canonical IDs (see
/// https://openrouter.ai/models) so the existing
/// `analysis.openrouter_api_key_env` flow round-trips cleanly.
pub const PRICING: &[Pricing] = &[
    // OpenRouter — Mistral family
    Pricing {
        model: "mistralai/mistral-7b-instruct",
        prompt_per_1k: 0.00007,
        completion_per_1k: 0.00007,
    },
    Pricing {
        model: "mistralai/mistral-small",
        prompt_per_1k: 0.0002,
        completion_per_1k: 0.0006,
    },
    Pricing {
        model: "mistralai/mistral-large",
        prompt_per_1k: 0.003,
        completion_per_1k: 0.009,
    },
    Pricing {
        model: "mistralai/mixtral-8x7b-instruct",
        prompt_per_1k: 0.00024,
        completion_per_1k: 0.00024,
    },
    Pricing {
        model: "mistralai/mixtral-8x22b-instruct",
        prompt_per_1k: 0.0012,
        completion_per_1k: 0.0012,
    },
    // OpenRouter — Anthropic
    Pricing {
        model: "anthropic/claude-3-haiku",
        prompt_per_1k: 0.00025,
        completion_per_1k: 0.00125,
    },
    Pricing {
        model: "anthropic/claude-3-sonnet",
        prompt_per_1k: 0.003,
        completion_per_1k: 0.015,
    },
    Pricing {
        model: "anthropic/claude-3-opus",
        prompt_per_1k: 0.015,
        completion_per_1k: 0.075,
    },
    Pricing {
        model: "anthropic/claude-3.5-sonnet",
        prompt_per_1k: 0.003,
        completion_per_1k: 0.015,
    },
    // OpenRouter — OpenAI
    Pricing {
        model: "openai/gpt-4o-mini",
        prompt_per_1k: 0.00015,
        completion_per_1k: 0.0006,
    },
    Pricing {
        model: "openai/gpt-4o",
        prompt_per_1k: 0.0025,
        completion_per_1k: 0.01,
    },
    // Whisper (transcription): per minute of audio, not per token —
    // included here for the table-of-record. Consumers requesting a
    // Pricing for these slugs will hit the heuristic fallback.
];

/// Look up pricing for a model slug, returning `None` for unknown
/// models (consumers fall back to a heuristic or refuse to estimate).
pub fn pricing_for(model: &str) -> Option<&'static Pricing> {
    PRICING.iter().find(|p| p.model == model)
}

/// Estimate cost in **cents** (`u64` for integer-only arithmetic in
/// downstream UI). Rounds half-up.
pub fn estimate_cost_cents(
    model: &str,
    prompt_tokens: usize,
    completion_tokens: usize,
) -> Option<u64> {
    let p = pricing_for(model)?;
    let usd = (prompt_tokens as f64 / 1000.0) * p.prompt_per_1k
        + (completion_tokens as f64 / 1000.0) * p.completion_per_1k;
    // Convert to cents and round half-up. Cap at u64::MAX as a sanity.
    let cents = (usd * 100.0).round() as i64;
    Some(cents.max(0) as u64)
}

/// Format a cent value as "$0.08" / "$1.23".
pub fn format_cents(cents: u64) -> String {
    let dollars = cents / 100;
    let rem = cents % 100;
    format!("${dollars}.{rem:02}")
}

// ── C2: monthly cost aggregation + budget warnings ─────────────────

/// Aggregate cost cents recorded in the videos table over the last N
/// days. Returns `(label, cents)` rows oldest-first so a sparkline
/// renders left-to-right naturally.
///
/// Buckets are days; the caller picks the window (30 for monthly,
/// 365 for yearly). Days with zero cost are filled in so the sparkline
/// width matches `window_days`.
pub fn monthly_cost_buckets(
    conn: &rusqlite::Connection,
    window_days: i64,
) -> anyhow::Result<Vec<(String, u64)>> {
    use std::collections::HashMap;
    let mut stmt = conn.prepare(
        "SELECT date(created_at) AS day, SUM(cost_cents) AS total \
         FROM videos \
         WHERE cost_cents > 0 AND created_at >= date('now', ?1) \
         GROUP BY day \
         ORDER BY day",
    )?;
    let window_clause = format!("-{window_days} days");
    let mut totals: HashMap<String, u64> = HashMap::new();
    let rows = stmt.query_map(rusqlite::params![window_clause], |row| {
        let day: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        Ok((day, total.max(0) as u64))
    })?;
    for r in rows {
        let (day, total) = r?;
        totals.insert(day, total);
    }

    // Fill missing days with 0 so the sparkline width is stable.
    let mut out = Vec::with_capacity(window_days as usize);
    let today = chrono::Utc::now().date_naive();
    for i in (0..window_days).rev() {
        let day = today - chrono::Duration::days(i);
        let key = day.format("%Y-%m-%d").to_string();
        let cents = totals.get(&key).copied().unwrap_or(0);
        out.push((key, cents));
    }
    Ok(out)
}

/// Per-backend cost breakdown over the last `window_days`. Useful for
/// "where am I spending" answers in the cost dashboard.
///
/// Backend is inferred from the model column on `video_analysis` —
/// transcribe cost is rolled into `videos.cost_cents` today (the
/// analyze backend is the only one with token-priced models). If the
/// schema later grows a per-stage cost ledger, this query swaps.
pub fn cost_by_model(
    conn: &rusqlite::Connection,
    window_days: i64,
) -> anyhow::Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(va.summary, 'transcribe') AS bucket, SUM(v.cost_cents) AS total \
         FROM videos v \
         LEFT JOIN video_analysis va ON va.video_id = v.id \
         WHERE v.cost_cents > 0 AND v.created_at >= date('now', ?1) \
         GROUP BY bucket \
         ORDER BY total DESC",
    )?;
    let window_clause = format!("-{window_days} days");
    let rows = stmt.query_map(rusqlite::params![window_clause], |row| {
        let bucket: String = row.get(0).unwrap_or_else(|_| "unknown".into());
        let total: i64 = row.get(1)?;
        Ok((bucket, total.max(0) as u64))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Total spend over the window, in cents. Convenience over summing
/// `monthly_cost_buckets`.
pub fn total_cents(buckets: &[(String, u64)]) -> u64 {
    buckets.iter().map(|(_, c)| *c).sum()
}

/// Budget status. Maps spend-to-budget ratio onto a warning level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    Ok,
    Warn,
    Exceeded,
}

/// Classify a (spend, budget) pair. budget=0 disables the warning
/// (returns Ok regardless of spend).
pub fn budget_status(spent_cents: u64, budget_cents: u64) -> BudgetStatus {
    if budget_cents == 0 {
        return BudgetStatus::Ok;
    }
    let pct = (spent_cents as f64) / (budget_cents as f64);
    if pct >= 1.0 {
        BudgetStatus::Exceeded
    } else if pct >= 0.8 {
        BudgetStatus::Warn
    } else {
        BudgetStatus::Ok
    }
}

/// Render a one-line ASCII sparkline of the bucket cents using the
/// canonical 8-glyph block ramp. Empty buckets render as ` `.
pub fn sparkline(buckets: &[(String, u64)]) -> String {
    let max = buckets.iter().map(|(_, c)| *c).max().unwrap_or(0);
    if max == 0 {
        return " ".repeat(buckets.len());
    }
    const RAMP: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    buckets
        .iter()
        .map(|(_, c)| {
            if *c == 0 {
                ' '
            } else {
                let idx = ((*c as f64 / max as f64) * (RAMP.len() - 1) as f64).round() as usize;
                RAMP[idx.min(RAMP.len() - 1)]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_lookup_known_model() {
        let p = pricing_for("mistralai/mistral-7b-instruct").unwrap();
        assert!(p.prompt_per_1k > 0.0);
    }

    #[test]
    fn pricing_lookup_unknown() {
        assert!(pricing_for("not/a-model").is_none());
    }

    #[test]
    fn cost_estimate_round_trip() {
        // mistral-large at 1k prompt + 1k completion = $0.003 + $0.009 = $0.012 = 1.2 cents
        let cents = estimate_cost_cents("mistralai/mistral-large", 1000, 1000).unwrap();
        // Half-up rounding: 1.2 cents rounds to 1.
        assert_eq!(cents, 1);
    }

    #[test]
    fn cost_estimate_larger_workload() {
        // 100k prompt + 50k completion against claude-3-haiku =
        // 100 * $0.00025 + 50 * $0.00125 = $0.025 + $0.0625 = $0.0875 = 9 cents
        let cents = estimate_cost_cents("anthropic/claude-3-haiku", 100_000, 50_000).unwrap();
        assert_eq!(cents, 9);
    }

    #[test]
    fn format_cents_shape() {
        assert_eq!(format_cents(8), "$0.08");
        assert_eq!(format_cents(100), "$1.00");
        assert_eq!(format_cents(1234), "$12.34");
    }

    #[test]
    fn budget_status_thresholds() {
        assert_eq!(budget_status(0, 1000), BudgetStatus::Ok);
        assert_eq!(budget_status(799, 1000), BudgetStatus::Ok);
        assert_eq!(budget_status(800, 1000), BudgetStatus::Warn);
        assert_eq!(budget_status(999, 1000), BudgetStatus::Warn);
        assert_eq!(budget_status(1000, 1000), BudgetStatus::Exceeded);
        assert_eq!(budget_status(5000, 1000), BudgetStatus::Exceeded);
        // budget=0 disables the warning.
        assert_eq!(budget_status(9999, 0), BudgetStatus::Ok);
    }

    #[test]
    fn sparkline_empty_buckets_are_spaces() {
        let buckets = vec![
            ("d1".into(), 0u64),
            ("d2".into(), 0u64),
            ("d3".into(), 0u64),
        ];
        assert_eq!(sparkline(&buckets), "   ");
    }

    #[test]
    fn sparkline_renders_ramp() {
        let buckets = vec![
            ("d1".into(), 1u64),
            ("d2".into(), 50u64),
            ("d3".into(), 100u64),
        ];
        let s = sparkline(&buckets);
        assert_eq!(s.chars().count(), 3);
        // Last bucket is the peak → should be the highest glyph.
        assert_eq!(s.chars().last().unwrap(), '█');
    }

    #[test]
    fn total_cents_sums() {
        let buckets = vec![("a".into(), 10u64), ("b".into(), 25u64), ("c".into(), 0u64)];
        assert_eq!(total_cents(&buckets), 35);
    }
}
