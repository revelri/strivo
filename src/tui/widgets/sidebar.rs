use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState},
};

use crate::app::{ActivePane, AppState};
use crate::platform::PlatformKind;
use crate::tui::theme::Theme;

/// Entries in the sidebar list — used to map list indices to selectable channels
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum SidebarEntry {
    Header,
    Separator,
    Channel(usize),    // index into app.channels
    StreamTitle(usize), // index into app.channels (sub-line, not selectable)
}

pub fn render(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let focused = app.active_pane == ActivePane::Sidebar;
    let border_style = if focused {
        Theme::border_focused()
    } else {
        Theme::border()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
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

    // Sort channels: group by platform, then within each group: recording > live > offline
    let mut twitch_indices: Vec<usize> = Vec::new();
    let mut youtube_indices: Vec<usize> = Vec::new();

    for (i, ch) in app.channels.iter().enumerate() {
        match ch.platform {
            PlatformKind::Twitch => twitch_indices.push(i),
            PlatformKind::YouTube => youtube_indices.push(i),
        }
    }

    let sort_key = |idx: &usize| -> u8 {
        let ch = &app.channels[*idx];
        if app.is_channel_recording(&ch.id) {
            0
        } else if ch.is_live {
            1
        } else {
            2
        }
    };

    twitch_indices.sort_by_key(sort_key);
    youtube_indices.sort_by_key(sort_key);

    // Update sidebar_order so navigation follows visual sort
    app.sidebar_order = twitch_indices.iter().chain(youtube_indices.iter()).copied().collect();

    let inner_width = area.width.saturating_sub(2) as usize; // inside borders

    let mut items: Vec<ListItem> = Vec::new();
    let mut entries: Vec<SidebarEntry> = Vec::new();

    // Helper to add a platform group
    let add_group = |platform: PlatformKind,
                         indices: &[usize],
                         items: &mut Vec<ListItem>,
                         entries: &mut Vec<SidebarEntry>| {
        if indices.is_empty() {
            return;
        }

        // Separator before second group
        if !items.is_empty() {
            items.push(ListItem::new(Line::raw("")));
            entries.push(SidebarEntry::Separator);
        }

        // Platform header with Nerd Font icon
        let (label, icon, color) = match platform {
            PlatformKind::Twitch => ("Twitch", " \u{F1E8}", Theme::TWITCH),   //
            PlatformKind::YouTube => ("YouTube", " 󰗃", Theme::YOUTUBE), // 󰗃
        };
        // Pad to fill width: name left, icon right
        let header_text = format!("{label}");
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!(" {header_text}"),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                icon,
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])));
        entries.push(SidebarEntry::Header);

        // Separator line
        let sep = "─".repeat(inner_width.saturating_sub(1));
        items.push(ListItem::new(Line::styled(
            format!(" {sep}"),
            Style::new().fg(Theme::DIM),
        )));
        entries.push(SidebarEntry::Separator);

        for &idx in indices {
            let ch = &app.channels[idx];
            let is_recording = app.is_channel_recording(&ch.id);

            // Build right-side indicators
            let mut right_parts: Vec<Span> = Vec::new();

            // Live/recording indicator
            if is_recording {
                right_parts.push(Span::styled("●", Theme::status_recording()));
            } else if ch.is_live {
                right_parts.push(Span::styled("◉", Theme::status_live()));
            }

            // Unwatched count
            let unwatched = app.unwatched_count_for_channel(&ch.id);
            if unwatched > 0 {
                if !right_parts.is_empty() {
                    right_parts.push(Span::raw(" "));
                }
                right_parts.push(Span::styled(
                    format!("[{unwatched}]"),
                    Style::new().fg(Theme::YELLOW),
                ));
            }

            // Channel name style: bold if auto_record (tracked)
            let name_style = if ch.auto_record {
                Style::new().fg(Theme::FG).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Theme::FG)
            };

            // Build the line: " ChannelName              indicators"
            let name = &ch.display_name;
            let right_text: String = right_parts.iter().map(|s| s.content.as_ref()).collect::<Vec<&str>>().join("");
            let right_display_len = right_text.chars().count();
            let name_max = inner_width.saturating_sub(2 + right_display_len + 1); // " " prefix + gap
            let display_name: String = if name.chars().count() > name_max {
                name.chars().take(name_max.saturating_sub(1)).collect::<String>() + "…"
            } else {
                name.to_string()
            };
            let pad = inner_width.saturating_sub(1 + display_name.chars().count() + right_display_len + 1);

            let mut spans = vec![
                Span::raw(" "),
                Span::styled(display_name, name_style),
                Span::raw(" ".repeat(pad)),
            ];
            spans.extend(right_parts);

            items.push(ListItem::new(Line::from(spans)));
            entries.push(SidebarEntry::Channel(idx));

            // Stream title sub-line for live channels
            if ch.is_live {
                if let Some(ref title) = ch.stream_title {
                    let max_title_width = inner_width.saturating_sub(4); // "   " prefix + margin
                    let is_selected = idx == app.selected_channel && focused;

                    let display_title = if is_selected && title.chars().count() > max_title_width {
                        // Autoscroll: use scroll_offsets
                        let offset = app.scroll_offsets.get(&idx).copied().unwrap_or(0);
                        let padded = format!("{title}   {title}"); // wrap around
                        let total_len = padded.chars().count();
                        let start = offset % total_len;
                        padded.chars().skip(start).take(max_title_width).collect::<String>()
                    } else if title.chars().count() > max_title_width {
                        title.chars().take(max_title_width.saturating_sub(1)).collect::<String>() + "…"
                    } else {
                        title.clone()
                    };

                    items.push(ListItem::new(Line::from(vec![
                        Span::raw("   "),
                        Span::styled(display_title, Theme::stream_subtitle()),
                    ])));
                    entries.push(SidebarEntry::StreamTitle(idx));
                }
            }
        }
    };

    add_group(PlatformKind::Twitch, &twitch_indices, &mut items, &mut entries);
    add_group(PlatformKind::YouTube, &youtube_indices, &mut items, &mut entries);

    // Find the list index for the selected channel
    let mut state = ListState::default();
    for (list_idx, entry) in entries.iter().enumerate() {
        if let SidebarEntry::Channel(ch_idx) = entry {
            if *ch_idx == app.selected_channel {
                state.select(Some(list_idx));
                break;
            }
        }
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Theme::selected());

    frame.render_stateful_widget(list, area, &mut state);
}
