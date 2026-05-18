/// Check if `needle` is a subsequence of `haystack` (chars appear in order).
/// Case-sensitive; callers should lowercase both strings for case-insensitive matching.
pub fn fuzzy_subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay_iter = haystack.chars();
    for nc in needle.chars() {
        loop {
            match hay_iter.next() {
                Some(hc) if hc == nc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// One fuzzy match — score plus the haystack character indices that
/// matched. Higher score = better match. Used by the upgraded TUI
/// filter (M4.2.c) for sorting + highlight-span rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub score: i32,
    /// 0-based indices into the haystack of the chars that matched
    /// (in order). Renderers wrap these chars in a highlight span.
    pub spans: Vec<usize>,
}

/// Score a needle against a haystack. Returns `None` when the needle
/// is not a subsequence. Scoring rewards:
/// - prefix matches (start at index 0)
/// - consecutive runs (no gaps between matched chars)
/// - matches at word boundaries (after a space / '_' / '-')
/// - shorter haystacks (less noise)
///
/// All matching is case-insensitive on ASCII; non-ASCII chars compare
/// strict-equal so unicode haystacks still work for users who type the
/// exact char.
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<FuzzyMatch> {
    if needle.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            spans: Vec::new(),
        });
    }
    let hay_chars: Vec<char> = haystack.chars().collect();
    let mut spans: Vec<usize> = Vec::with_capacity(needle.chars().count());
    let mut h_idx = 0usize;
    let mut score: i32 = 0;
    let mut prev_match: Option<usize> = None;
    for nc in needle.chars() {
        let nc_lower = nc.to_ascii_lowercase();
        loop {
            if h_idx >= hay_chars.len() {
                return None;
            }
            let hc = hay_chars[h_idx];
            let matches = hc == nc || hc.to_ascii_lowercase() == nc_lower;
            if matches {
                // Bonuses.
                let mut delta: i32 = 10;
                if let Some(prev) = prev_match {
                    if h_idx == prev + 1 {
                        delta += 8; // consecutive run
                    }
                }
                if h_idx == 0 {
                    delta += 12; // prefix
                } else if matches!(hay_chars[h_idx - 1], ' ' | '_' | '-' | '/' | '.') {
                    delta += 6; // word boundary
                }
                score += delta;
                spans.push(h_idx);
                prev_match = Some(h_idx);
                h_idx += 1;
                break;
            }
            h_idx += 1;
        }
    }
    // Shorter haystacks are tighter matches.
    score -= (hay_chars.len() as i32 - spans.len() as i32).max(0) / 4;
    Some(FuzzyMatch { score, spans })
}

/// Simple Levenshtein distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── fuzzy_subsequence ────────────────────────────────────────────

    #[test]
    fn subsequence_exact_match() {
        assert!(fuzzy_subsequence("hello", "hello"));
    }

    #[test]
    fn subsequence_chars_in_order() {
        assert!(fuzzy_subsequence("shr", "shroud"));
        assert!(fuzzy_subsequence("abc", "aXbXc"));
    }

    #[test]
    fn subsequence_no_match() {
        assert!(!fuzzy_subsequence("xyz", "hello"));
    }

    #[test]
    fn subsequence_out_of_order() {
        assert!(!fuzzy_subsequence("ba", "abc"));
    }

    #[test]
    fn subsequence_empty_needle() {
        assert!(fuzzy_subsequence("", "anything"));
    }

    #[test]
    fn subsequence_empty_haystack() {
        assert!(!fuzzy_subsequence("a", ""));
    }

    #[test]
    fn subsequence_both_empty() {
        assert!(fuzzy_subsequence("", ""));
    }

    #[test]
    fn subsequence_needle_longer_than_haystack() {
        assert!(!fuzzy_subsequence("abcdef", "abc"));
    }

    #[test]
    fn subsequence_case_sensitive() {
        assert!(!fuzzy_subsequence("A", "abc"));
        assert!(fuzzy_subsequence("a", "abc"));
    }

    #[test]
    fn subsequence_unicode() {
        assert!(fuzzy_subsequence("日本", "日X本"));
        assert!(!fuzzy_subsequence("本日", "日本"));
    }

    // ── levenshtein ──────────────────────────────────────────────────

    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein("kitten", "kitten"), 0);
    }

    #[test]
    fn levenshtein_classic() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn levenshtein_empty_a() {
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn levenshtein_empty_b() {
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn levenshtein_both_empty() {
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn levenshtein_single_edit() {
        assert_eq!(levenshtein("cat", "bat"), 1); // substitution
        assert_eq!(levenshtein("cat", "cats"), 1); // insertion
        assert_eq!(levenshtein("cats", "cat"), 1); // deletion
    }

    #[test]
    fn levenshtein_symmetric() {
        assert_eq!(levenshtein("abc", "xyz"), levenshtein("xyz", "abc"));
    }

    // ── fuzzy_match (M4.2.c) ─────────────────────────────────────────

    #[test]
    fn fuzzy_match_finds_subsequence() {
        let m = fuzzy_match("shr", "shroud").unwrap();
        assert_eq!(m.spans, vec![0, 1, 2]);
        // Prefix + consecutive run is the maximum reward.
        assert!(m.score > 30);
    }

    #[test]
    fn fuzzy_match_case_insensitive() {
        assert!(fuzzy_match("SHR", "shroud").is_some());
    }

    #[test]
    fn fuzzy_match_ranks_prefix_higher() {
        let prefix = fuzzy_match("foo", "foobar").unwrap();
        let middle = fuzzy_match("foo", "abfoo").unwrap();
        assert!(prefix.score > middle.score);
    }

    #[test]
    fn fuzzy_match_word_boundary_bonus() {
        // "ot" at start of "other" (word boundary after space) should
        // outscore "ot" in the middle of "thother".
        let boundary = fuzzy_match("ot", "the other").unwrap();
        let middle = fuzzy_match("ot", "thothery").unwrap();
        assert!(boundary.score > middle.score);
    }

    #[test]
    fn fuzzy_match_returns_none_on_miss() {
        assert!(fuzzy_match("xyz", "abc").is_none());
    }

    #[test]
    fn fuzzy_match_empty_needle_zero_score() {
        let m = fuzzy_match("", "anything").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.spans.is_empty());
    }
}
