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
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
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
}
