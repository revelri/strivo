use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::AppState;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let twitch_status = if app.config.twitch.is_some() {
        Span::styled("● Twitch", Theme::status_live())
    } else {
        Span::styled("○ Twitch", Theme::status_offline())
    };

    let youtube_status = if app.config.youtube.is_some() {
        Span::styled("● YouTube", Theme::status_live())
    } else {
        Span::styled("○ YouTube", Theme::status_offline())
    };

    let recording_count = Span::styled("0 Recording", Theme::status_bar());

    let help_hint = Span::styled("?", Theme::key_hint());

    let status_msg = if app.status_message.is_empty() {
        Span::raw("")
    } else {
        Span::styled(&app.status_message, Theme::status_bar())
    };

    let line = Line::from(vec![
        Span::raw(" "),
        twitch_status,
        Span::raw(" │ "),
        youtube_status,
        Span::raw(" │ "),
        recording_count,
        Span::raw(" │ "),
        help_hint,
        Span::raw("  "),
        status_msg,
    ]);

    let bar = Paragraph::new(line).style(Theme::status_bar());
    frame.render_widget(bar, area);
}
