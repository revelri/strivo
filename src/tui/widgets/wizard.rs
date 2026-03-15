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
            "  Welcome to StreaVo!",
            Style::new().fg(Theme::PURPLE).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::styled(
            "  No platforms configured yet.",
            Style::new().fg(Theme::FG),
        ),
        Line::raw(""),
        Line::styled(
            "  To get started, add your platform credentials",
            Style::new().fg(Theme::FG),
        ),
        Line::styled(
            "  to the config file at:",
            Style::new().fg(Theme::FG),
        ),
        Line::raw(""),
        Line::styled(
            format!("  {}", crate::config::AppConfig::config_path().display()),
            Style::new().fg(Theme::BLUE).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            "  [twitch]",
            Style::new().fg(Theme::TWITCH),
        ),
        Line::styled(
            "  client_id = \"your_client_id\"",
            Style::new().fg(Theme::GRAY),
        ),
        Line::styled(
            "  client_secret = \"your_secret\"",
            Style::new().fg(Theme::GRAY),
        ),
        Line::raw(""),
        Line::styled(
            "  [youtube]",
            Style::new().fg(Theme::YOUTUBE),
        ),
        Line::styled(
            "  client_id = \"your_client_id\"",
            Style::new().fg(Theme::GRAY),
        ),
        Line::styled(
            "  client_secret = \"your_secret\"",
            Style::new().fg(Theme::GRAY),
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
