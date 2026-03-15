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
        ("Enter/l", "Select / expand"),
        ("Esc/h", "Go back"),
        ("r", "Start recording"),
        ("w", "Watch in mpv"),
        ("a", "Toggle auto-record"),
        ("t", "Toggle transcode mode"),
        ("s", "Settings"),
        ("L", "Recording list"),
        ("F", "Log viewer"),
        ("q", "Quit"),
        ("?", "Toggle this help"),
    ];

    let mut lines: Vec<Line> = vec![Line::raw("")];
    for (key, desc) in keybinds {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{key:>10}"),
                Style::new().fg(Theme::YELLOW).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(desc, Style::new().fg(Theme::FG)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn render_confirm(frame: &mut Frame, area: Rect, message: &str) {
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(35),
        Constraint::Length(7),
        Constraint::Percentage(35),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Min(40),
        Constraint::Percentage(25),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Theme::YELLOW))
        .title(" Confirm ")
        .title_style(Style::new().fg(Theme::YELLOW).add_modifier(Modifier::BOLD));

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let lines = vec![
        Line::raw(""),
        Line::styled(
            format!("  {message}"),
            Style::new().fg(Theme::FG),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("[y]", Theme::key_hint()),
            Span::raw(" Yes  "),
            Span::styled("[n]", Theme::key_hint()),
            Span::raw(" No"),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
