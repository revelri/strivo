use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect) {
    // Center the wizard dialog
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(20),
        Constraint::Min(16),
        Constraint::Percentage(20),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(15),
        Constraint::Min(50),
        Constraint::Percentage(15),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_focused())
        .title(" Setup Wizard ")
        .title_style(Theme::title());

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            "  Welcome to StriVo!",
            Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::styled(
            "  No platforms configured yet.",
            Style::new().fg(Theme::fg()),
        ),
        Line::raw(""),
        Line::styled(
            "  To get started, add your platform credentials",
            Style::new().fg(Theme::fg()),
        ),
        Line::styled(
            "  to the config file at:",
            Style::new().fg(Theme::fg()),
        ),
        Line::raw(""),
        Line::styled(
            format!("  {}", crate::config::AppConfig::config_path().display()),
            Style::new().fg(Theme::blue()).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            "  [twitch]",
            Style::new().fg(Theme::twitch()),
        ),
        Line::styled(
            "  client_id = \"your_client_id\"",
            Style::new().fg(Theme::muted()),
        ),
        Line::styled(
            "  client_secret = \"your_secret\"",
            Style::new().fg(Theme::muted()),
        ),
        Line::raw(""),
        Line::styled(
            "  [youtube]",
            Style::new().fg(Theme::youtube()),
        ),
        Line::styled(
            "  client_id = \"your_client_id\"",
            Style::new().fg(Theme::muted()),
        ),
        Line::styled(
            "  client_secret = \"your_secret\"",
            Style::new().fg(Theme::muted()),
        ),
        Line::raw(""),
        Line::styled(
            "  [patreon]",
            Style::new().fg(Theme::patreon()),
        ),
        Line::styled(
            "  client_id = \"your_client_id\"",
            Style::new().fg(Theme::muted()),
        ),
        Line::styled(
            "  client_secret = \"your_secret\"",
            Style::new().fg(Theme::muted()),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Press "),
            Span::styled("Esc", Theme::key_hint()),
            Span::raw(" to dismiss, "),
            Span::styled("q", Theme::key_hint()),
            Span::raw(" to quit"),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
