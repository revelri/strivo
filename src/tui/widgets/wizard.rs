use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::app::AppState;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    // Center the wizard dialog.
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(15),
        Constraint::Min(18),
        Constraint::Percentage(15),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(15),
        Constraint::Min(54),
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

    // Branch 1: A device-code flow is live — render the code, verification
    // URL, and an "Open in browser" action so the user can complete auth
    // without leaving the TUI.
    if let Some((kind, ref uri, ref user_code)) = app.pending_auth {
        let lines = vec![
            Line::raw(""),
            Line::from(Span::styled(
                format!("  Authorize {kind}"),
                Style::new()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::styled(
                "  1.  Open this URL in a browser:",
                Style::new().fg(Theme::fg()),
            ),
            Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    uri.clone(),
                    Style::new().fg(Theme::blue()).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::raw(""),
            Line::styled(
                "  2.  Enter this code when prompted:",
                Style::new().fg(Theme::fg()),
            ),
            Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    user_code.clone(),
                    Style::new()
                        .fg(Theme::secondary())
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::raw(""),
            Line::styled(
                "  StriVo is polling for authorization — this page will",
                Style::new().fg(Theme::muted()),
            ),
            Line::styled(
                "  update automatically when the platform confirms.",
                Style::new().fg(Theme::muted()),
            ),
            Line::raw(""),
            Line::from(vec![
                Span::raw("  Press "),
                Span::styled("o", Theme::key_hint()),
                Span::raw(" to open in browser, "),
                Span::styled("Esc", Theme::key_hint()),
                Span::raw(" to skip, "),
                Span::styled("q", Theme::key_hint()),
                Span::raw(" to quit"),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // Branch 2: No device-code pending, at least one platform connected —
    // render a brief success screen until the user presses Esc.
    if app.twitch_connected || app.youtube_connected || app.patreon_connected {
        let mut lines = vec![
            Line::raw(""),
            Line::from(Span::styled(
                "  Connected:",
                Style::new()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
        ];
        if app.twitch_connected {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("● ", Style::new().fg(Theme::green())),
                Span::styled("Twitch", Style::new().fg(Theme::twitch())),
            ]));
        }
        if app.youtube_connected {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("● ", Style::new().fg(Theme::green())),
                Span::styled("YouTube", Style::new().fg(Theme::youtube())),
            ]));
        }
        if app.patreon_connected {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("● ", Style::new().fg(Theme::green())),
                Span::styled("Patreon", Style::new().fg(Theme::patreon())),
            ]));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::raw("  Press "),
            Span::styled("Esc", Theme::key_hint()),
            Span::raw(" to enter StriVo"),
        ]));
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // Branch 3: First run — no platforms configured. Point the user at the
    // config file (tokens live in the OS keyring; values in plain config).
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
            "  Add platform credentials to the config file at:",
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
        Line::styled(
            "  Relaunch StriVo after saving; a device-code prompt will",
            Style::new().fg(Theme::fg()),
        ),
        Line::styled(
            "  appear here so you can finish auth without leaving the TUI.",
            Style::new().fg(Theme::fg()),
        ),
        Line::raw(""),
        Line::from(Span::styled(
            "  Tokens are stored in the OS keyring; on hosts without a",
            Style::new().fg(Theme::muted()),
        )),
        Line::from(Span::styled(
            "  keyring provider, set STRIVO_<KEY_UPPER> environment vars.",
            Style::new().fg(Theme::muted()),
        )),
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
