use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::{AppState, UiEventLevel};
use crate::tui::theme::Theme;

/// Shift+E pop-over: scrollable list of the last ~100 user-facing events.
/// Reads from `AppState.event_ring`; the entries come from both daemon-
/// event mirrors (synchronous push in `handle_daemon_event`) and the
/// tracing-subscriber bridge (`tui::log_bridge`).
pub fn render(frame: &mut Frame, area: Rect, app: &AppState, enter_progress: f32) {
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(10),
        Constraint::Min(20),
        Constraint::Percentage(10),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(10),
        Constraint::Min(60),
        Constraint::Percentage(10),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let border_style = Theme::border_ramp(enter_progress).add_modifier(Modifier::BOLD);

    let block = Block::default()
        .title(" Event Log · last 100 ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);

    let inner = block.inner(center);
    frame.render_widget(block, center);

    if app.event_ring.is_empty() {
        let hint = Paragraph::new(Line::from(Span::styled(
            "no events yet",
            Style::new().fg(Theme::muted()),
        )))
        .alignment(Alignment::Center);
        frame.render_widget(hint, inner);
        return;
    }

    // Render newest-first. `event_ring` pushes to the back, so iter().rev()
    // gives the right order.
    let rows = inner.height as usize;
    let total = app.event_ring.len();
    let start = app.event_log_scroll.min(total.saturating_sub(1));
    let end = (start + rows).min(total);

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for ev in app.event_ring.iter().rev().skip(start).take(end - start) {
        let level_color = match ev.level {
            UiEventLevel::Trace => Theme::muted(),
            UiEventLevel::Debug => Theme::muted(),
            UiEventLevel::Info => Theme::primary(),
            UiEventLevel::Warn => Theme::yellow(),
            UiEventLevel::Error => Theme::red(),
        };
        let ts = ev
            .at
            .with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string();
        lines.push(Line::from(vec![
            Span::styled(ts, Style::new().fg(Theme::muted())),
            Span::raw("  "),
            Span::styled(
                format!("{:<5}", ev.level.label()),
                Style::new().fg(level_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:<10}", ev.source),
                Style::new().fg(Theme::secondary()),
            ),
            Span::raw(" "),
            Span::raw(ev.message.clone()),
        ]));
    }

    let footer = format!(
        " {}-{} of {}  [j/k scroll, g top, G bottom, Esc close] ",
        start + 1,
        end,
        total
    );
    let para = Paragraph::new(lines).style(Style::new().fg(Theme::fg()));
    let [body_area, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    frame.render_widget(para, body_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer,
            Style::new().fg(Theme::muted()),
        )))
        .alignment(Alignment::Right),
        footer_area,
    );
}
