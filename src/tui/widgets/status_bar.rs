use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::AppState;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let twitch_status = if app.twitch_connected {
        Span::styled("● Twitch", Theme::status_live())
    } else if app.config.twitch.is_some() {
        Span::styled("○ Twitch", Style::new().fg(Theme::YELLOW))
    } else {
        Span::styled("○ Twitch", Theme::status_offline())
    };

    let youtube_status = if app.youtube_connected {
        Span::styled("● YouTube", Theme::status_live())
    } else if app.config.youtube.is_some() {
        Span::styled("○ YouTube", Style::new().fg(Theme::YELLOW))
    } else {
        Span::styled("○ YouTube", Theme::status_offline())
    };

    let active = app.active_recording_count();
    let rec_style = if active > 0 {
        Theme::status_recording()
    } else {
        Theme::status_bar()
    };
    let recording_count = Span::styled(
        format!("{active} Rec"),
        rec_style,
    );

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

use ratatui::style::Style;
