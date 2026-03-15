use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState},
};

use crate::app::{ActivePane, AppState};
use crate::platform::PlatformKind;
use crate::recording::job::RecordingState;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let focused = app.active_pane == ActivePane::RecordingList;
    let border_style = if focused {
        Theme::border_focused()
    } else {
        Theme::border()
    };

    let active_count = app.active_recording_count();
    let title = format!(" Recordings ({active_count} active) ");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title)
        .title_style(Theme::title());

    let day_groups = app.recordings_by_day();

    if day_groups.is_empty() {
        let items = vec![
            ListItem::new(Line::raw("")),
            ListItem::new(Line::styled(
                "  No recordings yet",
                Style::new().fg(Theme::GRAY),
            )),
            ListItem::new(Line::raw("")),
            ListItem::new(Line::from(vec![
                Span::raw("  Press "),
                Span::styled("r", Theme::key_hint()),
                Span::raw(" on a live channel to start recording"),
            ])),
        ];
        let list = List::new(items).block(block);
        frame.render_widget(list, area);
        return;
    }

    // Build day-grouped display
    // Track which list items are selectable recordings vs headers
    let mut items: Vec<ListItem> = Vec::new();
    let mut selectable_indices: Vec<usize> = Vec::new(); // list index → recording flat index
    for (day_label, recs) in &day_groups {
        // Day header
        if !items.is_empty() {
            items.push(ListItem::new(Line::raw("")));
        }
        items.push(ListItem::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(day_label, Theme::day_header()),
        ])));

        for rec in recs {
            let list_idx = items.len();

            let state_prefix = match rec.state {
                RecordingState::Recording => {
                    Span::styled("● ", Theme::status_recording())
                }
                RecordingState::ResolvingUrl => {
                    Span::styled("⟳ ", Style::new().fg(Theme::YELLOW))
                }
                RecordingState::Stopping => {
                    Span::styled("◼ ", Style::new().fg(Theme::YELLOW))
                }
                RecordingState::Failed => {
                    Span::styled("✗ ", Theme::error())
                }
                RecordingState::Finished => {
                    Span::raw("  ")
                }
            };

            let channel_name = Span::styled(
                &rec.channel_name,
                Style::new().fg(Theme::FG).add_modifier(Modifier::BOLD),
            );

            let title_text = rec
                .stream_title
                .as_deref()
                .unwrap_or("stream");

            let platform_icon = match rec.platform {
                PlatformKind::Twitch => Span::styled(
                    " \u{F1E8}",
                    Style::new().fg(Theme::TWITCH),
                ),
                PlatformKind::YouTube => Span::styled(
                    " 󰗃",
                    Style::new().fg(Theme::YOUTUBE),
                ),
            };

            let duration = Span::styled(
                rec.format_duration(),
                Style::new().fg(Theme::GRAY),
            );

            items.push(ListItem::new(Line::from(vec![
                Span::raw("   "),
                state_prefix,
                channel_name,
                Span::styled(" — ", Style::new().fg(Theme::DIM)),
                Span::styled(title_text, Style::new().fg(Theme::FG)),
                Span::styled(" — ", Style::new().fg(Theme::DIM)),
                duration,
                Span::styled(" —", Style::new().fg(Theme::DIM)),
                platform_icon,
            ])));

            selectable_indices.push(list_idx);
        }
    }

    let mut state = ListState::default();
    if !selectable_indices.is_empty() {
        let sel = app.selected_recording.min(selectable_indices.len() - 1);
        state.select(Some(selectable_indices[sel]));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Theme::selected());

    frame.render_stateful_widget(list, area, &mut state);
}
