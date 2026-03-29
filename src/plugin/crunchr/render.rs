use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use crate::app::AppState;
use crate::tui::theme::Theme;

use super::CrunchrPlugin;
use super::types::{PipelineState, SearchMode};

pub fn render(plugin: &CrunchrPlugin, frame: &mut Frame, area: Rect, _app: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_focused())
        .title(" CrunchR Intelligence ")
        .title_style(Theme::title());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !plugin.backend_available && plugin.search_results.is_empty() && plugin.queue.is_empty() {
        render_no_backend(frame, inner);
        return;
    }

    let [search_area, content_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    render_search_bar(plugin, frame, search_area);
    render_content(plugin, frame, content_area);
    render_footer(plugin, frame, footer_area);
}

fn render_no_backend(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::raw(""),
        Line::raw(""),
        Line::styled(
            "  No transcription backend available",
            Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            "  Configure in config.toml [crunchr]:",
            Style::new().fg(Theme::fg()),
        ),
        Line::styled(
            "    backend = \"voxtral-api\"   # Mistral API ($0.18/hr, diarization)",
            Style::new().fg(Theme::dim()),
        ),
        Line::styled(
            "    backend = \"voxtral-local\" # self-hosted (free, needs GPU)",
            Style::new().fg(Theme::dim()),
        ),
        Line::styled(
            "    backend = \"whisper-cli\"   # local (pip install openai-whisper)",
            Style::new().fg(Theme::dim()),
        ),
        Line::raw(""),
        Line::styled(
            "  Existing search data is still accessible.",
            Style::new().fg(Theme::muted()),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_search_bar(plugin: &CrunchrPlugin, frame: &mut Frame, area: Rect) {
    let mode_label = plugin.search_mode.label();
    let mode_style = match plugin.search_mode {
        SearchMode::FullText => Style::new().fg(Theme::primary()),
        SearchMode::Semantic => Style::new().fg(Theme::secondary()),
    };

    let cursor = if plugin.input_active { "▌" } else { "" };

    // Width-aware: hide mode label on narrow terminals
    let show_mode = area.width > 50;
    let mode_width = if show_mode { mode_label.len() as u16 + 4 } else { 0 };

    let pad_width = area.width
        .saturating_sub(5 + plugin.search_query.len() as u16 + mode_width) as usize;

    let mut spans = vec![
        Span::styled(" / ", Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD)),
        Span::styled(&plugin.search_query, Style::new().fg(Theme::fg())),
        Span::styled(cursor, Style::new().fg(Theme::primary())),
        Span::raw(format!("{:width$}", "", width = pad_width)),
    ];

    if show_mode {
        spans.push(Span::styled("[", Style::new().fg(Theme::muted())));
        spans.push(Span::styled(mode_label, mode_style));
        spans.push(Span::styled("]", Style::new().fg(Theme::muted())));
        spans.push(Span::raw(" "));
    }

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::new().fg(Theme::muted()));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(block),
        area,
    );
}

/// Main content area: shows results+analytics, queue, or empty state
fn render_content(plugin: &CrunchrPlugin, frame: &mut Frame, area: Rect) {
    // If we have search results, show results + analytics pane
    if !plugin.search_results.is_empty() {
        render_results_with_analytics(plugin, frame, area);
        return;
    }

    // If queue has items and no search, show pipeline progress
    let active_jobs: Vec<_> = plugin.queue.iter()
        .filter(|j| j.state != PipelineState::Complete && j.state != PipelineState::Failed)
        .collect();
    let recent_complete: Vec<_> = plugin.queue.iter()
        .filter(|j| j.state == PipelineState::Complete || j.state == PipelineState::Failed)
        .take(5)
        .collect();

    if !active_jobs.is_empty() || !recent_complete.is_empty() {
        render_queue(plugin, frame, area, &active_jobs, &recent_complete);
        return;
    }

    // Empty state
    let lines = vec![
        Line::raw(""),
        Line::raw(""),
        Line::styled(
            "  No transcripts yet",
            Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            "  Record a stream and transcripts will appear here automatically.",
            Style::new().fg(Theme::fg()),
        ),
        Line::raw(""),
        Line::styled(
            "  Press / to search when transcripts are available.",
            Style::new().fg(Theme::muted()),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

/// Results list (top) + analytics detail pane (bottom) when a result is selected
fn render_results_with_analytics(plugin: &CrunchrPlugin, frame: &mut Frame, area: Rect) {
    // Split: results top, analytics bottom (if analysis data exists)
    let has_analytics = plugin.selected_analysis.is_some();
    let analytics_height = if has_analytics { 7 } else { 0 };

    let areas = if has_analytics {
        let [results_area, analytics_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(analytics_height),
        ])
        .areas(area);
        (results_area, Some(analytics_area))
    } else {
        (area, None)
    };

    render_results_list(plugin, frame, areas.0);

    if let (Some(analytics_area), Some(ref analysis)) = (areas.1, &plugin.selected_analysis) {
        render_analytics_pane(frame, analytics_area, analysis);
    }
}

fn render_results_list(plugin: &CrunchrPlugin, frame: &mut Frame, area: Rect) {
    let mut lines = Vec::new();
    for (i, result) in plugin.search_results.iter().enumerate() {
        let is_selected = i == plugin.selected_result;
        let prefix = if is_selected { ">" } else { " " };

        let title_style = if is_selected {
            Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Theme::fg())
        };

        let channel_style = Style::new().fg(Theme::dim());
        let snippet_style = if is_selected {
            Style::new().fg(Theme::fg())
        } else {
            Style::new().fg(Theme::muted())
        };

        let time = format_timestamp(result.start_sec);

        lines.push(Line::from(vec![
            Span::styled(prefix, title_style),
            Span::styled(&result.video_title, title_style),
            Span::styled(format!(" ({}) ", result.channel_name), channel_style),
            Span::styled(format!("[{time}]"), Style::new().fg(Theme::secondary())),
        ]));

        // Snippet line with FTS highlight and optional speaker label
        let max_snippet_width = area.width.saturating_sub(4) as usize;
        let mut snippet_spans = vec![Span::raw("  ")];

        // Speaker label prefix (from diarization)
        if is_selected {
            if let Some(ref speaker) = plugin.selected_speaker {
                snippet_spans.push(Span::styled(
                    format!("[{speaker}] "),
                    Style::new().fg(Theme::dim()),
                ));
            }
        }

        // FTS snippet with highlight: split on >>> / <<<
        render_highlighted_snippet(&result.snippet, max_snippet_width, snippet_style, &mut snippet_spans);

        lines.push(Line::from(snippet_spans));
        lines.push(Line::raw(""));
    }

    let visible_height = area.height as usize;
    let scroll_offset = if plugin.selected_result * 3 >= visible_height {
        (plugin.selected_result * 3).saturating_sub(visible_height / 2)
    } else {
        0
    };

    let paragraph = Paragraph::new(lines).scroll((scroll_offset as u16, 0));
    frame.render_widget(paragraph, area);

    if plugin.search_results.len() * 3 > visible_height {
        let mut scrollbar_state = ScrollbarState::new(plugin.search_results.len() * 3)
            .position(scroll_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut scrollbar_state,
        );
    }
}

/// Render FTS snippet with >>> <<< markers converted to highlighted spans
fn render_highlighted_snippet<'a>(snippet: &'a str, max_width: usize, base_style: Style, spans: &mut Vec<Span<'a>>) {
    let highlight_style = Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD);
    let mut remaining = max_width;
    let mut text = snippet;

    while !text.is_empty() && remaining > 0 {
        if let Some(start) = text.find(">>>") {
            // Text before the marker
            let before = &text[..start];
            let take = before.len().min(remaining);
            if take > 0 {
                spans.push(Span::styled(&before[..take], base_style));
                remaining = remaining.saturating_sub(take);
            }
            text = &text[start + 3..]; // skip >>>

            // Find closing <<<
            if let Some(end) = text.find("<<<") {
                let highlighted = &text[..end];
                let take = highlighted.len().min(remaining);
                if take > 0 {
                    spans.push(Span::styled(&highlighted[..take], highlight_style));
                    remaining = remaining.saturating_sub(take);
                }
                text = &text[end + 3..]; // skip <<<
            } else {
                // No closing marker, render rest as highlighted
                let take = text.len().min(remaining);
                if take > 0 {
                    spans.push(Span::styled(&text[..take], highlight_style));
                }
                break;
            }
        } else {
            // No more markers, render rest as plain
            let take = text.len().min(remaining);
            if take > 0 {
                spans.push(Span::styled(&text[..take], base_style));
            }
            break;
        }
    }
}

/// Analytics detail pane showing analysis for the selected result
fn render_analytics_pane(
    frame: &mut Frame,
    area: Rect,
    analysis: &super::types::AnalysisData,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(Theme::muted()))
        .title(" Analysis ")
        .title_style(Style::new().fg(Theme::dim()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();

    // Summary
    if !analysis.summary.is_empty() {
        let summary_display: String = analysis.summary.chars().take(inner.width as usize * 2).collect();
        lines.push(Line::from(vec![
            Span::styled(" ", Style::new().fg(Theme::dim())),
            Span::styled(summary_display, Style::new().fg(Theme::fg())),
        ]));
    }

    // Topics + sentiment on one line
    if !analysis.topics.is_empty() || analysis.sentiment != "unknown" {
        let mut topic_spans = vec![Span::styled(" ", Style::new().fg(Theme::dim()))];

        for (i, topic) in analysis.topics.iter().take(6).enumerate() {
            if i > 0 {
                topic_spans.push(Span::styled(", ", Style::new().fg(Theme::muted())));
            }
            topic_spans.push(Span::styled(topic.as_str(), Style::new().fg(Theme::primary())));
        }

        if !analysis.topics.is_empty() && analysis.sentiment != "unknown" {
            topic_spans.push(Span::styled("  ", Style::new().fg(Theme::dim())));
        }

        if analysis.sentiment != "unknown" {
            let sentiment_color = match analysis.sentiment.as_str() {
                "positive" => Theme::green(),
                "negative" => Theme::red(),
                _ => Theme::muted(),
            };
            topic_spans.push(Span::styled(
                format!("({})", analysis.sentiment),
                Style::new().fg(sentiment_color),
            ));
        }

        lines.push(Line::from(topic_spans));
    }

    if lines.is_empty() {
        lines.push(Line::styled(
            "  No analysis available",
            Style::new().fg(Theme::muted()),
        ));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// Render the processing queue inline in the content area
fn render_queue(
    _plugin: &CrunchrPlugin,
    frame: &mut Frame,
    area: Rect,
    active: &[&super::types::ProcessingJob],
    complete: &[&super::types::ProcessingJob],
) {
    let mut lines = vec![
        Line::raw(""),
        Line::styled(
            "  Processing Pipeline",
            Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];

    for job in active {
        let indicator = match job.state {
            PipelineState::Pending => Span::styled("  ○ ", Style::new().fg(Theme::dim())),
            PipelineState::Failed => Span::styled("  ✗ ", Style::new().fg(Theme::red())),
            _ => Span::styled("  ● ", Style::new().fg(Theme::secondary())),
        };

        let title: String = format!("{} - {}", job.channel_name, job.title)
            .chars()
            .take(area.width.saturating_sub(6) as usize)
            .collect();

        lines.push(Line::from(vec![
            indicator,
            Span::styled(title, Style::new().fg(Theme::fg())),
        ]));

        let state_style = match job.state {
            PipelineState::Failed => Style::new().fg(Theme::red()),
            _ => Style::new().fg(Theme::dim()),
        };
        let detail = if let Some(ref err) = job.error {
            format!("    {} - {}", job.state, err)
        } else {
            format!("    {}...", job.state)
        };
        lines.push(Line::styled(detail, state_style));
        lines.push(Line::raw(""));
    }

    for job in complete {
        let indicator = if job.state == PipelineState::Complete {
            Span::styled("  ✓ ", Style::new().fg(Theme::green()))
        } else {
            Span::styled("  ✗ ", Style::new().fg(Theme::red()))
        };

        let title: String = format!("{} - {}", job.channel_name, job.title)
            .chars()
            .take(area.width.saturating_sub(6) as usize)
            .collect();

        lines.push(Line::from(vec![
            indicator,
            Span::styled(title, Style::new().fg(Theme::muted())),
        ]));
    }

    // Hint
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "  Press / to search transcripts",
        Style::new().fg(Theme::muted()),
    ));

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_footer(plugin: &CrunchrPlugin, frame: &mut Frame, area: Rect) {
    // Error state takes priority
    if let Some(ref error) = plugin.last_error {
        let error_display: String = error.chars().take(area.width.saturating_sub(4) as usize).collect();
        let line = Line::from(vec![
            Span::styled(" ⚠ ", Style::new().fg(Theme::red())),
            Span::styled(error_display, Style::new().fg(Theme::red())),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    // Contextual footer: queue status when active, word frequencies when idle
    let pending = plugin.queue.iter()
        .filter(|j| j.state != PipelineState::Complete && j.state != PipelineState::Failed)
        .count();

    let mut spans = Vec::new();

    if pending > 0 {
        let complete = plugin.queue.iter().filter(|j| j.state == PipelineState::Complete).count();
        spans.push(Span::styled(" Queue: ", Style::new().fg(Theme::dim())));
        spans.push(Span::styled(format!("{pending} pending"), Style::new().fg(Theme::secondary())));
        spans.push(Span::styled(format!(" / {complete} done"), Style::new().fg(Theme::dim())));
    } else if !plugin.word_frequencies.is_empty() {
        // Width-aware word frequency display
        let available = area.width.saturating_sub(7) as usize; // " Top: " prefix
        spans.push(Span::styled(" Top: ", Style::new().fg(Theme::dim())));
        let mut used = 0;
        for (i, (word, count)) in plugin.word_frequencies.iter().enumerate() {
            let entry = if i > 0 {
                format!(", {word}({count})")
            } else {
                format!("{word}({count})")
            };
            if used + entry.len() > available {
                break;
            }
            if i > 0 {
                spans.push(Span::styled(", ", Style::new().fg(Theme::muted())));
            }
            spans.push(Span::styled(
                format!("{word}({count})"),
                Style::new().fg(Theme::fg()),
            ));
            used += entry.len();
        }
    }

    if !spans.is_empty() {
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

fn format_timestamp(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
