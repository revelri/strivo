use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{ActivePane, AppState, RecordingListView};
use crate::platform::PlatformKind;
use crate::recording::job::RecordingState;
use crate::tui::theme::Theme;

use crate::tui::anim::glyphs::spinner_frame;

pub fn render(frame: &mut Frame, area: Rect, app: &mut AppState) {
    if app.recording_list_view == RecordingListView::Grid {
        render_grid(frame, area, app);
        return;
    }
    let border_style = app.pane_border(&ActivePane::RecordingList);

    let active_count = app.active_recording_count();
    let title = if !app.search_query.is_empty() {
        format!(
            " Recordings [/{query}] ({active_count} active) ",
            query = app.search_query
        )
    } else {
        format!(" Recordings ({active_count} active) ")
    };

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
                Style::new().fg(Theme::muted()),
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
        items.push(ListItem::new(Line::from(Theme::section_rule_spans(
            day_label,
        ))));

        for rec in recs {
            // Apply search filter
            if !app.recording_matches_filter(&rec.id) {
                continue;
            }

            let list_idx = items.len();

            let secs = app.clock.elapsed().as_secs_f32();
            let state_prefix = match rec.state {
                RecordingState::Recording => {
                    // C6.7 — heartbeat: ● → ◉ alternation at 1 Hz.
                    let glyph = if crate::tui::anim::reduce_motion() {
                        "● "
                    } else if (secs * 2.0) as i32 % 2 == 0 {
                        "● "
                    } else {
                        "◉ "
                    };
                    Span::styled(glyph, Theme::status_recording())
                }
                RecordingState::ResolvingUrl => {
                    let f = spinner_frame(secs);
                    Span::styled(format!("{f} "), Style::new().fg(Theme::secondary()))
                }
                RecordingState::Stopping => {
                    // C2.5 — crossfade between the ◼ block and a dimmer ◻
                    // outline at 0.5 Hz so the user sees it's actively stopping
                    // vs. a stuck state.
                    let glyph =
                        if !crate::tui::anim::reduce_motion() && (secs * 2.0) as i32 % 2 == 0 {
                            "◼ "
                        } else {
                            "◻ "
                        };
                    Span::styled(glyph, Style::new().fg(Theme::secondary()))
                }
                RecordingState::Failed => {
                    // C2.6 — Failed flash: slow breathing pulse between theme
                    // red and a brighter tint so the error stays eye-catching
                    // without demanding per-job transition bookkeeping.
                    let flash = if crate::tui::anim::reduce_motion() {
                        0.0
                    } else {
                        crate::tui::anim::easing::Ease::InOutSine
                            .apply(crate::tui::anim::pulse_phase(secs, 1.4))
                    };
                    let bright = ratatui::style::Color::Rgb(255, 120, 120);
                    let color = crate::tui::theme::Theme::blend_for(Theme::red(), bright, flash);
                    Span::styled("✗ ", Style::new().fg(color).add_modifier(Modifier::BOLD))
                }
                RecordingState::Finished => Span::raw("  "),
            };

            let name_base = Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD);
            let title_base = Style::new().fg(Theme::fg());
            let hl_style = Style::new()
                .fg(Theme::secondary())
                .add_modifier(Modifier::BOLD);
            let channel_spans = crate::tui::widgets::highlight::highlight_spans(
                &rec.channel_name,
                &app.search_query,
                name_base,
                hl_style,
            );
            let title_text: String = rec.stream_title.as_deref().unwrap_or("stream").to_string();
            let title_spans = crate::tui::widgets::highlight::highlight_spans(
                &title_text,
                &app.search_query,
                title_base,
                hl_style,
            );

            let platform_icon = match rec.platform {
                PlatformKind::Twitch => Span::styled(" \u{F1E8}", Style::new().fg(Theme::twitch())),
                PlatformKind::YouTube => Span::styled(" 󰗃", Style::new().fg(Theme::youtube())),
                PlatformKind::Patreon => Span::styled(" ", Style::new().fg(Theme::patreon())),
            };

            let duration = Span::styled(rec.format_duration(), Style::new().fg(Theme::muted()));

            let marker = if app.recording_selections_set.contains(&rec.id) {
                Span::styled(
                    " ▌ ",
                    Style::new()
                        .fg(Theme::primary())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("   ")
            };

            let mut row_spans: Vec<Span> = vec![marker, state_prefix];
            row_spans.extend(channel_spans);
            row_spans.push(Span::styled(" — ", Style::new().fg(Theme::dim())));
            row_spans.extend(title_spans);
            row_spans.push(Span::styled(" — ", Style::new().fg(Theme::dim())));
            row_spans.push(duration);
            row_spans.push(Span::styled(" —", Style::new().fg(Theme::dim())));
            row_spans.push(platform_icon);
            items.push(ListItem::new(Line::from(row_spans)));

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

/// M5.4 grid renderer. 3 cols × N rows of thumbnail cells. Each cell
/// renders the cached protocol if present, a placeholder otherwise.
/// The cursor moves cell-by-cell using selected_recording (linear);
/// h/l jump ±1 cell, j/k jump ±3 (one row).
pub fn render_grid(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let border_style = app.pane_border(&ActivePane::RecordingList);
    let total = app.sorted_recordings().len();
    let title = format!(" Recordings · Grid ({total}) [Tab to list] ");
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title)
        .title_style(Theme::title());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if total == 0 {
        let hint = Paragraph::new(Line::styled(
            "  No recordings yet",
            Style::new().fg(Theme::muted()),
        ));
        frame.render_widget(hint, inner);
        return;
    }

    const COLS: u16 = 3;
    let cell_h: u16 = 8;
    let rows = inner.height.saturating_sub(1) / cell_h;
    let visible = (rows as usize) * (COLS as usize);
    if rows == 0 {
        return;
    }

    // Center the cursor in the visible window when possible.
    let sel = app.selected_recording.min(total.saturating_sub(1));
    let scroll_start = if visible >= total {
        0usize
    } else {
        sel.saturating_sub(visible / 2).min(total - visible)
    };

    // Gather (id, channel_name, duration_str) for the visible window.
    let visible_recs: Vec<(uuid::Uuid, std::path::PathBuf, String, String)> = {
        let recs = app.sorted_recordings();
        recs.iter()
            .skip(scroll_start)
            .take(visible)
            .map(|r| {
                (
                    r.id,
                    r.output_path.clone(),
                    r.channel_name.clone(),
                    r.format_duration(),
                )
            })
            .collect()
    };

    // Best-effort spawn of decode jobs for any visible recording that
    // doesn't have a decoded protocol yet. The actual ffmpeg + image
    // decode happens in the run loop; here we just mark the in-flight
    // set so the run loop knows what's wanted.
    let to_decode: Vec<(uuid::Uuid, std::path::PathBuf)> = visible_recs
        .iter()
        .filter_map(|(id, path, _, _)| {
            if app.recording_thumb_protocols.contains_key(id)
                || app.recording_thumb_in_flight.contains(id)
            {
                None
            } else {
                Some((*id, path.clone()))
            }
        })
        .collect();
    for (id, _) in &to_decode {
        app.recording_thumb_in_flight.insert(*id);
    }
    // The actual spawn happens in src/tui/mod.rs; we publish the
    // wishlist via AppState so the run loop can pull it on the next
    // tick without us holding a sender here.
    app.pending_recording_thumb_jobs.extend(to_decode);

    // Render row by row.
    let row_constraints: Vec<Constraint> = std::iter::repeat(Constraint::Length(cell_h))
        .take(rows as usize)
        .collect();
    let row_rects = Layout::vertical(row_constraints).split(inner);

    let local_index_of_sel = sel.checked_sub(scroll_start);

    for (row_idx, row_rect) in row_rects.iter().enumerate() {
        let col_constraints: [Constraint; 3] = [
            Constraint::Ratio(1, COLS as u32),
            Constraint::Ratio(1, COLS as u32),
            Constraint::Ratio(1, COLS as u32),
        ];
        let cells = Layout::horizontal(col_constraints).split(*row_rect);
        for (col_idx, cell_rect) in cells.iter().enumerate() {
            let local = row_idx * (COLS as usize) + col_idx;
            let Some((id, _, channel_name, duration)) = visible_recs.get(local) else {
                continue;
            };
            let is_selected = local_index_of_sel == Some(local);
            let cell_border = if is_selected {
                Style::new()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Theme::dim())
            };
            let label = format!(" {channel_name} · {duration} ");
            let cell_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(cell_border)
                .title(label);
            let cell_inner = cell_block.inner(*cell_rect);
            frame.render_widget(cell_block, *cell_rect);

            if let Some(proto) = app.recording_thumb_protocols.get_mut(id) {
                let image_widget = ratatui_image::StatefulImage::default();
                frame.render_stateful_widget(image_widget, cell_inner, proto);
            } else {
                let placeholder =
                    Paragraph::new(Line::styled("  decoding…", Style::new().fg(Theme::muted())));
                frame.render_widget(placeholder, cell_inner);
            }
        }
    }
}
