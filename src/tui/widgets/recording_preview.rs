//! Docked recording preview — third column for the RecordingList pane on
//! wide terminals. Shows the same metadata the Properties modal would,
//! continuously updating as the cursor moves through the list. Removes
//! the need to open Properties just to glance at codec/bitrate/size.
//!
//! Intentionally simple: no scroll, no plugin sections, no actions —
//! that's what the Properties modal is for. This is a read-only summary.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::AppState;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border())
        .padding(Padding::horizontal(1))
        .title(" Preview ")
        .title_style(Style::new().fg(Theme::muted()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(rec_id) = app.selected_recording_id else {
        let p =
            Paragraph::new("No recording selected").style(Style::new().fg(Theme::muted()));
        frame.render_widget(p, inner);
        return;
    };
    let Some(rec) = app.recordings.get(&rec_id) else {
        return;
    };

    let mut lines: Vec<Line> = Vec::new();

    // Title block — stream title + channel/platform chip on next line.
    let title = rec.stream_title.as_deref().unwrap_or("(no title)");
    let title_truncated: String = title.chars().take(60).collect();
    lines.push(Line::from(Span::styled(
        title_truncated,
        Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled(&rec.channel_name, Style::new().fg(Theme::primary())),
        Span::styled(
            format!(" · {}", rec.platform),
            Style::new().fg(Theme::muted()),
        ),
    ]));
    lines.push(Line::raw(""));

    // File facts
    lines.push(kv("Size", &rec.format_size()));
    lines.push(kv(
        "Date",
        &rec.started_at.format("%Y-%m-%d %H:%M").to_string(),
    ));
    lines.push(kv("Duration", &rec.format_duration()));

    // Media info if probed
    if let Some(info) = app.media_info_cache.get(&rec_id) {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Media",
            Style::new()
                .fg(Theme::secondary())
                .add_modifier(Modifier::BOLD),
        )));
        if let Some(ref c) = info.video_codec {
            lines.push(kv("Video", &format!("{c} {}", info.resolution_str())));
        }
        if let Some(ref c) = info.audio_codec {
            lines.push(kv("Audio", c));
        }
        lines.push(kv("Bitrate", &info.bitrate_str()));
        lines.push(kv("Format", &info.format_name));
    }

    // Path — truncated from the left so the filename stays visible.
    lines.push(Line::raw(""));
    let path = rec.output_path.display().to_string();
    let path_cap = inner.width.saturating_sub(4) as usize;
    let path_shown = if path.chars().count() > path_cap {
        let cut: String = path
            .chars()
            .rev()
            .take(path_cap.saturating_sub(3))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{cut}")
    } else {
        path
    };
    lines.push(Line::from(vec![
        Span::styled("Path ", Style::new().fg(Theme::dim())),
        Span::styled(path_shown, Style::new().fg(Theme::fg())),
    ]));

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("[i]", Theme::key_hint()),
        Span::raw(" Properties  "),
        Span::styled("[p]", Theme::key_hint()),
        Span::raw(" Play"),
    ]));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        inner,
    );
}

fn kv(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9}"), Style::new().fg(Theme::dim())),
        Span::styled(value.to_string(), Style::new().fg(Theme::fg())),
    ])
}
