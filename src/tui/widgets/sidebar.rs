use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::{ActivePane, AppState};
use crate::platform::PlatformKind;
use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let focused = app.active_pane == ActivePane::Sidebar;
    let border_style = if focused {
        Theme::border_focused()
    } else {
        Theme::border()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Channels ")
        .title_style(Theme::title());

    if app.channels.is_empty() {
        let placeholder = List::new(vec![ListItem::new("  No channels")])
            .block(block)
            .style(Style::new().fg(Theme::GRAY));
        frame.render_widget(placeholder, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut current_platform: Option<PlatformKind> = None;

    for channel in &app.channels {
        // Add platform header when platform changes
        if current_platform != Some(channel.platform) {
            if current_platform.is_some() {
                items.push(ListItem::new(Line::raw("")));
            }
            let (label, color) = match channel.platform {
                PlatformKind::Twitch => ("[T] Twitch", Theme::TWITCH),
                PlatformKind::YouTube => ("[Y] YouTube", Theme::YOUTUBE),
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(label, Style::new().fg(color).add_modifier(Modifier::BOLD)),
            ])));
            items.push(ListItem::new(Line::styled("──────────────────", Style::new().fg(Theme::DIM))));
            current_platform = Some(channel.platform);
        }

        let status_dot = if channel.is_live {
            Span::styled("● ", Theme::status_live())
        } else {
            Span::styled("○ ", Theme::status_offline())
        };

        let name = Span::styled(
            &channel.display_name,
            Style::new().fg(Theme::FG),
        );

        items.push(ListItem::new(Line::from(vec![status_dot, name])));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Theme::selected());

    let mut state = ListState::default();
    // Map app's selected_channel index to the actual list index (accounting for headers)
    if !app.channels.is_empty() {
        let mut list_idx = 0;
        let mut current_plat: Option<PlatformKind> = None;
        for (i, ch) in app.channels.iter().enumerate() {
            if current_plat != Some(ch.platform) {
                if current_plat.is_some() {
                    list_idx += 1; // empty line
                }
                list_idx += 2; // header + separator
                current_plat = Some(ch.platform);
            }
            if i == app.selected_channel {
                state.select(Some(list_idx));
                break;
            }
            list_idx += 1;
        }
    }

    frame.render_stateful_widget(list, area, &mut state);
}
