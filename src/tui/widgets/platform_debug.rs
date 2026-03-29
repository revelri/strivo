use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::app::AppState;
use crate::platform::PlatformKind;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState, kind: PlatformKind) {
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(20),
        Constraint::Min(14),
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

    let platform_name = match kind {
        PlatformKind::Twitch => "Twitch",
        PlatformKind::YouTube => "YouTube",
        PlatformKind::Patreon => "Patreon",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_focused())
        .title(format!(" {platform_name} Status "))
        .title_style(Theme::title());

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let mut lines = Vec::new();

    // Connection status
    let (connected, config_present) = match kind {
        PlatformKind::Twitch => (app.twitch_connected, app.config.twitch.is_some()),
        PlatformKind::YouTube => (app.youtube_connected, app.config.youtube.is_some()),
        PlatformKind::Patreon => (app.patreon_connected, app.config.patreon.is_some()),
    };

    let status_label = if connected {
        Span::styled("  Connected", Style::new().fg(Theme::green()).add_modifier(Modifier::BOLD))
    } else if config_present {
        Span::styled("  Not Connected", Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("  Not Configured", Style::new().fg(Theme::red()).add_modifier(Modifier::BOLD))
    };

    lines.push(Line::from(vec![
        Span::styled(" Status: ", Style::new().fg(Theme::fg())),
        status_label,
    ]));

    // Config info
    let config_hint = match kind {
        PlatformKind::Twitch => {
            if config_present {
                "  Config: [twitch] section present"
            } else {
                "  Config: Add [twitch] with client_id and client_secret to config.toml"
            }
        }
        PlatformKind::YouTube => {
            if config_present {
                let has_cookies = app.config.youtube.as_ref()
                    .and_then(|y| y.cookies_path.as_ref())
                    .is_some();
                if has_cookies {
                    "  Config: [youtube] section present (cookies configured)"
                } else {
                    "  Config: [youtube] section present (no cookies_path for Premium)"
                }
            } else {
                "  Config: Add [youtube] with client_id and client_secret to config.toml"
            }
        }
        PlatformKind::Patreon => {
            if config_present {
                "  Config: [patreon] section present"
            } else {
                "  Config: Add [patreon] with client_id and client_secret to config.toml"
            }
        }
    };
    lines.push(Line::styled(config_hint, Style::new().fg(Theme::dim())));
    lines.push(Line::raw(""));

    // Errors
    let errors = app.platform_errors.get(&kind);
    let error_count = errors.map_or(0, |e| e.len());

    if error_count > 0 {
        lines.push(Line::styled(
            format!("  Recent Errors ({error_count})"),
            Style::new().fg(Theme::red()).add_modifier(Modifier::BOLD),
        ));
        if let Some(errors) = errors {
            let max_width = inner.width.saturating_sub(4) as usize;
            for err in errors.iter().rev().take(5) {
                let display: String = err.chars().take(max_width).collect();
                lines.push(Line::styled(
                    format!("  - {display}"),
                    Style::new().fg(Theme::red()),
                ));
            }
        }
    } else {
        lines.push(Line::styled(
            "  No recent errors",
            Style::new().fg(Theme::muted()),
        ));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("[Esc]", Theme::key_hint()),
        Span::raw(" Close"),
    ]));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner,
    );
}
