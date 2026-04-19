use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{ActivePane, AppState};
use crate::platform::PlatformKind;
use crate::plugin::registry::PluginRegistry;
use crate::tui::theme::Theme;

/// REC-dot pulse style keyed off the frame tick. Oscillates opacity with a
/// ~2 s ease-in-out cycle (DESIGN.md signature motion) so a live recording
/// is never mistaken for a static indicator. Never goes fully transparent.
fn rec_pulse_style(tick: u64) -> Style {
    const CYCLE: u64 = 60;
    let phase = (tick % CYCLE) as f32 / CYCLE as f32;
    let o = 0.7 + 0.3 * (std::f32::consts::TAU * phase).cos();
    let base = 0.2;
    let o = (o * (1.0 - base) + base).clamp(0.0, 1.0);
    let r = (0xFF as f32 * o) as u8;
    let g = (0x44 as f32 * o) as u8;
    let b = (0x44 as f32 * o) as u8;
    Style::new()
        .fg(Color::Rgb(r, g, b))
        .bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg()))
        .add_modifier(Modifier::BOLD)
}

pub fn render(frame: &mut Frame, area: Rect, app: &AppState, registry: &PluginRegistry) {
    let bar_style = Theme::hotkey_bar();
    let key_style = Theme::hotkey_key();
    let bar_bg = Theme::hotkey_bar().bg.unwrap_or(Theme::bg());

    // Persistent banner while the daemon socket is down — overrides
    // everything else so the user can never mistake stale data for live.
    if !app.daemon_connected {
        let msg = format!(
            " ⚠  Daemon disconnected — reconnecting (attempt {}) ",
            app.daemon_reconnect_attempts
        );
        let pad = area.width.saturating_sub(msg.chars().count() as u16) as usize;
        let line = Line::from(vec![
            Span::styled(
                msg,
                Style::new()
                    .fg(Theme::red())
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    // If search input is active, render an editable prompt with the cursor
    // visible at `search_cursor` (split the query at that char boundary).
    if app.search_active {
        let chars: Vec<char> = app.search_query.chars().collect();
        let cur = app.search_cursor.min(chars.len());
        let left: String = chars[..cur].iter().collect();
        let right: String = chars[cur..].iter().collect();

        let used = 2 + left.chars().count() + 1 + right.chars().count(); // " /" + left + cursor + right
        let pad = area.width.saturating_sub(used as u16) as usize;

        let search_bar = Line::from(vec![
            Span::styled(" /", Style::new().fg(Theme::secondary()).bg(bar_bg)),
            Span::styled(left, Style::new().fg(Theme::fg()).bg(bar_bg)),
            Span::styled(
                "▌",
                Style::new()
                    .fg(Theme::primary())
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(right, Style::new().fg(Theme::fg()).bg(bar_bg)),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(search_bar).style(bar_style), area);
        return;
    }

    // StatusBar focus mode: show navigation hints instead of normal buttons
    if app.active_pane == ActivePane::StatusBar {
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(" ", bar_style));
        push_button(&mut spans, "Select", "←→", bar_style, key_style);
        push_button(&mut spans, "Debug", "Enter", bar_style, key_style);
        push_button(&mut spans, "Back", "Esc", bar_style, key_style);

        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let indicators = build_indicators(app, bar_bg, Some(app.selected_indicator));
        let ind_width: usize = indicators.iter().map(|s| s.content.chars().count()).sum();

        let plugin_statuses = registry.status_lines(app);
        let plugin_width: usize = plugin_statuses.iter().map(|s| s.len() + 2).sum();

        let pad = (area.width as usize).saturating_sub(used + ind_width + plugin_width);
        spans.push(Span::styled(" ".repeat(pad), bar_style));

        for status in &plugin_statuses {
            spans.push(Span::styled(
                format!("[{status}] "),
                Style::new().fg(Theme::secondary()).bg(bar_bg),
            ));
        }
        spans.extend(indicators);

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    // If a transient status message is live (set within the last ~5 s, per the
    // app-level auto-dismiss tick), render it in place of the hotkey bar so
    // one-shot feedback actually reaches the user.
    if !app.status_message.is_empty() {
        let msg = app.status_message.clone();
        let pad = area.width.saturating_sub(msg.chars().count() as u16 + 1) as usize;
        let line = Line::from(vec![
            Span::styled(" ", bar_style),
            Span::styled(
                msg,
                Style::new()
                    .fg(Theme::fg())
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", bar_style));

    // Filter-active indicator — visible whenever a search filter is in force
    // but the input is not focused. Spells out what Esc will do.
    if !app.search_query.is_empty() {
        let (matched, total) = filter_counts(app);
        let label = format!("[/{}] {}/{} · Esc clears ", app.search_query, matched, total);
        spans.push(Span::styled(
            label,
            Style::new()
                .fg(Theme::secondary())
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Pulsing REC dot when a recording is active (DESIGN.md signature motion).
    let active_recordings = app.active_recording_count();
    if active_recordings > 0 {
        spans.push(Span::styled("● ", rec_pulse_style(app.tick_counter)));
        spans.push(Span::styled(
            format!("REC({active_recordings}) "),
            Style::new()
                .fg(Theme::red())
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Context-sensitive buttons
    match app.active_pane {
        ActivePane::Detail => {
            push_button(&mut spans, "Record", "r", bar_style, key_style);
            push_button(&mut spans, "Watch", "w", bar_style, key_style);
        }
        ActivePane::RecordingList => {
            push_button(&mut spans, "Stop", "s", bar_style, key_style);
            push_button(&mut spans, "Play", "p", bar_style, key_style);
            push_button(&mut spans, "Info", "i", bar_style, key_style);
        }
        _ => {}
    }

    // Always-visible buttons
    push_button(&mut spans, "Search", "/", bar_style, key_style);
    push_button(&mut spans, "Intel", "I", bar_style, key_style);
    push_button(&mut spans, "Config", "C", bar_style, key_style);
    push_button(&mut spans, "Help", "?", bar_style, key_style);
    push_button(&mut spans, "Recordings", "L", bar_style, key_style);
    push_button(&mut spans, "Log", "F", bar_style, key_style);
    push_button(&mut spans, "Quit", "q", bar_style, key_style);

    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let total_width = area.width as usize;

    // Build platform indicators (hidden when unconfigured)
    let indicators = build_indicators(app, bar_bg, None);
    let ind_width: usize = indicators.iter().map(|s| s.content.chars().count()).sum();

    // Plugin status indicators
    let plugin_statuses = registry.status_lines(app);
    let plugin_width: usize = plugin_statuses.iter().map(|s| s.len() + 2).sum();

    let pad = total_width.saturating_sub(used + ind_width + plugin_width);
    spans.push(Span::styled(" ".repeat(pad), bar_style));

    for status in &plugin_statuses {
        spans.push(Span::styled(
            format!("[{status}] "),
            Style::new().fg(Theme::secondary()).bg(bar_bg),
        ));
    }
    spans.extend(indicators);

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(bar_style);
    frame.render_widget(bar, area);
}

/// Build the platform indicator spans. Only configured platforms are shown.
/// `highlight_idx` highlights one indicator when in StatusBar focus mode.
fn build_indicators<'a>(
    app: &AppState,
    bar_bg: ratatui::style::Color,
    highlight_idx: Option<usize>,
) -> Vec<Span<'a>> {
    let bar_style = Theme::hotkey_bar();
    let mut spans = Vec::new();
    let mut idx = 0usize;

    let configured = configured_platforms(app);

    for kind in &configured {
        let (connected, has_errors) = match kind {
            PlatformKind::Twitch => (app.twitch_connected, app.platform_errors.get(kind).is_some_and(|e| !e.is_empty())),
            PlatformKind::YouTube => (app.youtube_connected, app.platform_errors.get(kind).is_some_and(|e| !e.is_empty())),
            PlatformKind::Patreon => (app.patreon_connected, app.platform_errors.get(kind).is_some_and(|e| !e.is_empty())),
        };

        let is_highlighted = highlight_idx == Some(idx);

        let (bullet, color) = if connected && !has_errors {
            ("● ", Theme::green())
        } else if connected && has_errors {
            ("● ", Theme::secondary()) // amber: connected but errors
        } else {
            ("○ ", Theme::secondary()) // amber: configured but not connected
        };

        let label = match kind {
            PlatformKind::Twitch => "TW ",
            PlatformKind::YouTube => "YT ",
            PlatformKind::Patreon => "PA ",
        };

        let style = if is_highlighted {
            Style::new().fg(color).bg(bar_bg).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new().fg(color).bg(bar_bg)
        };

        let label_style = if is_highlighted {
            Style::new().fg(Theme::fg()).bg(bar_bg).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            bar_style
        };

        spans.push(Span::styled(bullet, style));
        spans.push(Span::styled(label, label_style));
        idx += 1;
    }

    spans
}

/// Return the list of configured platforms in display order.
pub fn configured_platforms(app: &AppState) -> Vec<PlatformKind> {
    let mut platforms = Vec::new();
    if app.config.twitch.is_some() {
        platforms.push(PlatformKind::Twitch);
    }
    if app.config.youtube.is_some() {
        platforms.push(PlatformKind::YouTube);
    }
    if app.config.patreon.is_some() {
        platforms.push(PlatformKind::Patreon);
    }
    platforms
}

fn push_button<'a>(
    spans: &mut Vec<Span<'a>>,
    label: &'a str,
    key: &'a str,
    bar_style: Style,
    key_style: Style,
) {
    spans.push(Span::styled(label, bar_style));
    spans.push(Span::styled(" [", bar_style));
    spans.push(Span::styled(key, key_style));
    spans.push(Span::styled("]", bar_style));
    spans.push(Span::styled(" ", bar_style)); // 1-char gap
}

/// Returns `(matched, total)` for the pane the filter currently applies to.
/// Used by the filter-active indicator so the user can see how many items
/// survived the query.
fn filter_counts(app: &AppState) -> (usize, usize) {
    match app.active_pane {
        ActivePane::RecordingList => {
            let total = app.recordings.len();
            let matched = if app.search_filtered_recordings.is_empty() {
                total
            } else {
                app.search_filtered_recordings.len()
            };
            (matched, total)
        }
        // Sidebar / Detail both filter the channel list
        _ => {
            let total = app.channels.len();
            let matched = if app.search_filtered_channels.is_empty() {
                total
            } else {
                app.search_filtered_channels.len()
            };
            (matched, total)
        }
    }
}
