use std::str::FromStr;

use cron::Schedule;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState},
};

use crate::app::AppState;
use crate::tui::theme::Theme;

/// Render the schedule pane: one row per configured ScheduleEntry,
/// showing channel, cron, duration, and the next computed fire time.
pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let border_style = app.pane_border(&crate::app::ActivePane::Schedule);
    let block = Block::default()
        .title(Line::from(vec![
            Span::raw(" Schedule "),
            Span::styled(
                format!("({})", app.config.schedule.len()),
                Style::new().fg(Theme::muted()),
            ),
            Span::raw(" "),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);

    if app.config.schedule.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let hint = ratatui::widgets::Paragraph::new(Line::from(vec![Span::styled(
            "  No schedules configured. Add `[[schedule]]` rows to config.toml.",
            Style::new().fg(Theme::muted()),
        )]));
        frame.render_widget(hint, inner);
        return;
    }

    let now = chrono::Utc::now();
    let mut items: Vec<ListItem> = Vec::with_capacity(app.config.schedule.len());

    for entry in &app.config.schedule {
        let cron_expr = if entry.cron.split_whitespace().count() == 5 {
            format!("0 {}", entry.cron)
        } else {
            entry.cron.clone()
        };
        let next_label = match Schedule::from_str(&cron_expr) {
            Ok(sched) => sched
                .upcoming(chrono::Utc)
                .next()
                .map(|dt| {
                    let local = dt.with_timezone(&chrono::Local);
                    let delta = dt - now;
                    let until = if delta.num_days() > 0 {
                        format!("in {}d", delta.num_days())
                    } else if delta.num_hours() > 0 {
                        format!("in {}h", delta.num_hours())
                    } else if delta.num_minutes() > 0 {
                        format!("in {}m", delta.num_minutes())
                    } else {
                        "soon".to_string()
                    };
                    format!("{} ({})", local.format("%a %b %-d · %H:%M"), until)
                })
                .unwrap_or_else(|| "—".to_string()),
            Err(e) => format!("cron error: {e}"),
        };

        items.push(ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<24}", entry.channel),
                Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:<22}", entry.cron),
                Style::new().fg(Theme::secondary()),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:<6}", entry.duration),
                Style::new().fg(Theme::muted()),
            ),
            Span::raw(" "),
            Span::styled(next_label, Style::new().fg(Theme::fg())),
        ])));
    }

    let mut state = ListState::default();
    if !items.is_empty() {
        let sel = app.selected_schedule.min(items.len() - 1);
        state.select(Some(sel));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Theme::selected());
    frame.render_stateful_widget(list, area, &mut state);
}
