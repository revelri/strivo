use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{ActivePane, AppState};
use crate::plugin::registry::PluginRegistry;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState, registry: &PluginRegistry) {
    let bar_style = Theme::hotkey_bar();
    let key_style = Theme::hotkey_key();

    // If search input is active, render a search prompt instead of normal buttons
    if app.search_active {
        let search_bar = Line::from(vec![
            Span::styled(" /", Style::new().fg(Theme::secondary()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg()))),
            Span::styled(&app.search_query, Style::new().fg(Theme::fg()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg()))),
            Span::styled("▌", Style::new().fg(Theme::primary()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg()))),
            Span::styled(
                format!("{:width$}", "", width = area.width.saturating_sub(3 + app.search_query.len() as u16) as usize),
                bar_style,
            ),
        ]);
        frame.render_widget(Paragraph::new(search_bar).style(bar_style), area);
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", bar_style));

    // Context-sensitive buttons
    match app.active_pane {
        ActivePane::Detail => {
            push_button(&mut spans, "Record", "r", bar_style, key_style);
            push_button(&mut spans, "Watch", "w", bar_style, key_style);
        }
        ActivePane::RecordingList => {
            push_button(&mut spans, "Stop", "s", bar_style, key_style);
            push_button(&mut spans, "Play", "p", bar_style, key_style);
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

    // Fill remaining space with background
    // Calculate used width
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let total_width = area.width as usize;

    // Connection status on far right: "● TW ● YT"
    let tw_indicator = if app.twitch_connected {
        Span::styled("● ", Style::new().fg(Theme::green()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    } else if app.config.twitch.is_some() {
        Span::styled("○ ", Style::new().fg(Theme::secondary()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    } else {
        Span::styled("○ ", Style::new().fg(Theme::muted()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    };
    let tw_label = Span::styled("TW ", bar_style);
    let yt_indicator = if app.youtube_connected {
        Span::styled("● ", Style::new().fg(Theme::green()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    } else if app.config.youtube.is_some() {
        Span::styled("○ ", Style::new().fg(Theme::secondary()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    } else {
        Span::styled("○ ", Style::new().fg(Theme::muted()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    };
    let yt_label = Span::styled("YT ", bar_style);
    let pa_indicator = if app.patreon_connected {
        Span::styled("● ", Style::new().fg(Theme::green()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    } else if app.config.patreon.is_some() {
        Span::styled("○ ", Style::new().fg(Theme::secondary()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())))
    } else {
        Span::styled("", bar_style)
    };
    let pa_label = if app.config.patreon.is_some() {
        Span::styled("PA", bar_style)
    } else {
        Span::styled("", bar_style)
    };

    // Plugin status indicators
    let plugin_statuses = registry.status_lines(app);
    let plugin_width: usize = plugin_statuses.iter().map(|s| s.len() + 2).sum();

    let right_width = if app.config.patreon.is_some() { 15 } else { 10 };
    let pad = total_width.saturating_sub(used + right_width + plugin_width);
    spans.push(Span::styled(" ".repeat(pad), bar_style));

    for status in &plugin_statuses {
        spans.push(Span::styled(
            format!("[{status}] "),
            Style::new().fg(Theme::secondary()).bg(Theme::hotkey_bar().bg.unwrap_or(Theme::bg())),
        ));
    }
    spans.push(tw_indicator);
    spans.push(tw_label);
    spans.push(yt_indicator);
    spans.push(yt_label);
    spans.push(pa_indicator);
    spans.push(pa_label);

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(bar_style);
    frame.render_widget(bar, area);
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
