//! Build a `Vec<Span>` from a label + fuzzy needle, painting the
//! matched character indices in a highlight style. Used by the
//! Sidebar and RecordingList row renderers (M4.follow.b).
//!
//! When the needle is empty the whole label renders in the base style
//! in one span. When the needle doesn't match (fuzzy_match → None),
//! same single-span fallback so the row still renders.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::search::fuzzy_match;

/// Build highlight spans for `label`.
///
/// - `needle`: the current search query (empty disables highlighting).
/// - `base`: style applied to non-matching characters.
/// - `hl`: style applied to characters at the FuzzyMatch span indices.
///
/// Allocates one `Span` per highlight run + one per non-highlight run.
pub fn highlight_spans(label: &str, needle: &str, base: Style, hl: Style) -> Vec<Span<'static>> {
    if needle.is_empty() {
        return vec![Span::styled(label.to_string(), base)];
    }
    let Some(m) = fuzzy_match(needle, label) else {
        return vec![Span::styled(label.to_string(), base)];
    };
    if m.spans.is_empty() {
        return vec![Span::styled(label.to_string(), base)];
    }

    let mut out: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut buf_is_hl = false;
    let highlight_set: std::collections::HashSet<usize> = m.spans.iter().copied().collect();

    for (i, c) in label.chars().enumerate() {
        let want_hl = highlight_set.contains(&i);
        if buf.is_empty() {
            buf.push(c);
            buf_is_hl = want_hl;
            continue;
        }
        if want_hl == buf_is_hl {
            buf.push(c);
        } else {
            let style = if buf_is_hl { hl } else { base };
            out.push(Span::styled(std::mem::take(&mut buf), style));
            buf.push(c);
            buf_is_hl = want_hl;
        }
    }
    if !buf.is_empty() {
        let style = if buf_is_hl { hl } else { base };
        out.push(Span::styled(buf, style));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn empty_needle_one_span() {
        let spans = highlight_spans(
            "shroud",
            "",
            Style::default(),
            Style::default().fg(Color::Cyan),
        );
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn miss_returns_one_span() {
        let spans = highlight_spans(
            "abc",
            "xyz",
            Style::default(),
            Style::default().fg(Color::Cyan),
        );
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn prefix_match_splits_into_two_spans() {
        let spans = highlight_spans(
            "shroud",
            "shr",
            Style::default(),
            Style::default().fg(Color::Cyan),
        );
        // Highlight "shr" then plain "oud".
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "shr");
        assert_eq!(spans[1].content, "oud");
    }

    #[test]
    fn split_match_alternates_runs() {
        // "a_c" against "abXcd" — matches indices 0, 3 (a, c). Output:
        //   hl "a", base "bX", hl "c", base "d"
        let spans = highlight_spans(
            "abXcd",
            "ac",
            Style::default(),
            Style::default().fg(Color::Cyan),
        );
        let labels: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(labels, vec!["a", "bX", "c", "d"]);
    }
}
