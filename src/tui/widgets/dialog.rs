use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::tui::theme::Theme;

pub fn render_help(frame: &mut Frame, area: Rect) {
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(15),
        Constraint::Min(18),
        Constraint::Percentage(15),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Min(44),
        Constraint::Percentage(20),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_focused())
        .title(" Help ")
        .title_style(Theme::title());

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let keybinds = vec![
        ("j/k, ↑/↓", "Navigate channels"),
        ("Enter", "Select channel"),
        ("r", "Start recording"),
        ("w", "Watch in mpv"),
        ("a", "Toggle auto-record"),
        ("t", "Toggle transcode mode"),
        ("s", "Settings"),
        ("l", "Recording list"),
        ("q", "Quit"),
        ("Esc", "Close dialog / Go back"),
        ("?", "Toggle this help"),
    ];

    let mut lines: Vec<Line> = vec![Line::raw("")];
    for (key, desc) in keybinds {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{key:>10}"), Style::new().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(desc, Style::new().fg(Theme::FG)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
