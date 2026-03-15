use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::{ActivePane, AppState};
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
        .border_style(border_style)
        .title(title)
        .title_style(Theme::title());

    let recordings = app.sorted_recordings();

    if recordings.is_empty() {
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

    let items: Vec<ListItem> = recordings
        .iter()
        .map(|rec| {
            let state_indicator = match rec.state {
                RecordingState::ResolvingUrl => {
                    Span::styled("⟳ ", Style::new().fg(Theme::YELLOW))
                }
                RecordingState::Recording => {
                    Span::styled("● ", Theme::status_recording())
                }
                RecordingState::Stopping => {
                    Span::styled("◼ ", Style::new().fg(Theme::YELLOW))
                }
                RecordingState::Finished => {
                    Span::styled("✓ ", Style::new().fg(Theme::GREEN))
                }
                RecordingState::Failed => {
                    Span::styled("✗ ", Theme::error())
                }
            };

            let name = Span::styled(
                &rec.channel_name,
                Style::new().fg(Theme::FG).add_modifier(Modifier::BOLD),
            );

            let details = if rec.state == RecordingState::Recording {
                format!(" {} · {}", rec.format_duration(), rec.format_size())
            } else {
                format!(" {}", rec.state)
            };

            ListItem::new(Line::from(vec![
                state_indicator,
                name,
                Span::styled(details, Style::new().fg(Theme::GRAY)),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    if !recordings.is_empty() {
        state.select(Some(app.selected_recording.min(recordings.len() - 1)));
    }

    let keybinds = Line::from(vec![
        Span::raw(" "),
        Span::styled("[s]", Theme::key_hint()),
        Span::raw(" Stop  "),
        Span::styled("[p]", Theme::key_hint()),
        Span::raw(" Play  "),
        Span::styled("[Esc]", Theme::key_hint()),
        Span::raw(" Back"),
    ]);

    // We render the list + keybinds in the same block
    let list = List::new(items)
        .block(
            block.title_bottom(keybinds),
        )
        .highlight_style(Theme::selected());

    frame.render_stateful_widget(list, area, &mut state);
}
