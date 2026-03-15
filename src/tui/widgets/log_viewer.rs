use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use crate::app::{ActivePane, AppState};
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let focused = app.active_pane == ActivePane::Log;
    let border_style = if focused {
        Theme::border_focused()
    } else {
        Theme::border()
    };

    let total = app.log_lines.len();
    let scroll_indicator = if app.log_auto_scroll {
        "LIVE"
    } else {
        "PAUSED"
    };

    let title = format!(" Log [{scroll_indicator}] ({total} lines) ");

    let keybinds = Line::from(vec![
        Span::raw(" "),
        Span::styled("[j/k]", Theme::key_hint()),
        Span::raw(" Scroll  "),
        Span::styled("[g/G]", Theme::key_hint()),
        Span::raw(" Top/Bottom  "),
        Span::styled("[c]", Theme::key_hint()),
        Span::raw(" Clear  "),
        Span::styled("[Esc]", Theme::key_hint()),
        Span::raw(" Back"),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title)
        .title_style(if app.log_auto_scroll {
            Style::new().fg(Theme::GREEN).add_modifier(Modifier::BOLD)
        } else {
            Theme::title()
        })
        .title_bottom(keybinds);

    if app.log_lines.is_empty() {
        let placeholder = Paragraph::new("  No log entries yet. Log file will appear here as events occur.")
            .style(Style::new().fg(Theme::GRAY))
            .block(block);
        frame.render_widget(placeholder, area);
        return;
    }

    let inner_height = block.inner(area).height as usize;

    // Calculate visible window
    let start = if app.log_auto_scroll {
        app.log_lines.len().saturating_sub(inner_height)
    } else {
        app.log_scroll
    };

    let lines: Vec<Line> = app.log_lines[start..]
        .iter()
        .take(inner_height)
        .enumerate()
        .map(|(_, line)| colorize_log_line(line))
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);

    // Scrollbar
    if total > inner_height {
        let scrollbar_area = Rect {
            x: area.x + area.width - 1,
            y: area.y + 1,
            width: 1,
            height: area.height.saturating_sub(2),
        };
        let mut scrollbar_state = ScrollbarState::new(total.saturating_sub(inner_height))
            .position(start);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::new().fg(Theme::PURPLE))
            .track_style(Style::new().fg(Theme::DIM));
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

fn colorize_log_line(line: &str) -> Line<'_> {
    // Color based on log level
    let style = if line.contains(" ERROR ") {
        Style::new().fg(Color::Rgb(243, 139, 168))
    } else if line.contains(" WARN ") {
        Style::new().fg(Color::Rgb(249, 226, 175))
    } else if line.contains(" INFO ") {
        Style::new().fg(Color::Rgb(205, 214, 244))
    } else if line.contains(" DEBUG ") {
        Style::new().fg(Color::Rgb(88, 91, 112))
    } else if line.contains(" TRACE ") {
        Style::new().fg(Color::Rgb(69, 71, 90))
    } else {
        Style::new().fg(Theme::GRAY)
    };

    Line::styled(line, style)
}
