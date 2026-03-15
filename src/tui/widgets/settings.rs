use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::{ActivePane, AppState};
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let focused = app.active_pane == ActivePane::Settings;
    let border_style = if focused {
        Theme::border_focused()
    } else {
        Theme::border()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Settings ")
        .title_style(Theme::title());

    let settings_items = vec![
        (
            "Recording Directory",
            app.config.recording_dir.to_string_lossy().to_string(),
        ),
        (
            "Poll Interval",
            format!("{}s", app.config.poll_interval_secs),
        ),
        (
            "Transcode Mode",
            if app.transcode_mode {
                "ON (NVENC)".to_string()
            } else {
                "OFF (passthrough)".to_string()
            },
        ),
        (
            "Twitch",
            if app.config.twitch.is_some() {
                if app.twitch_connected {
                    "Connected".to_string()
                } else {
                    "Configured (not connected)".to_string()
                }
            } else {
                "Not configured".to_string()
            },
        ),
        (
            "YouTube",
            if app.config.youtube.is_some() {
                if app.youtube_connected {
                    "Connected".to_string()
                } else {
                    "Configured (not connected)".to_string()
                }
            } else {
                "Not configured".to_string()
            },
        ),
    ];

    let items: Vec<ListItem> = settings_items
        .iter()
        .map(|(label, value)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {label}: "),
                    Style::new().fg(Theme::BLUE).add_modifier(Modifier::BOLD),
                ),
                Span::styled(value.as_str(), Style::new().fg(Theme::FG)),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.settings_selected));

    let config_path_hint = Line::from(vec![
        Span::raw(" Config: "),
        Span::styled(
            crate::config::AppConfig::config_path().to_string_lossy().to_string(),
            Style::new().fg(Theme::GRAY),
        ),
    ]);

    let list = List::new(items)
        .block(block.title_bottom(config_path_hint))
        .highlight_style(Style::new().fg(Theme::BG).bg(Theme::BLUE));

    frame.render_stateful_widget(list, area, &mut state);
}
