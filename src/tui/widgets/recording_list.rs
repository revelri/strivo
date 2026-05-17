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

/// 10-frame braille spinner. Advances every 80 ms off the frame clock so the
/// animation speed is independent of render cadence.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(elapsed_secs: f32) -> &'static str {
    if crate::tui::anim::reduce_motion() {
        return "⟳";
    }
    let idx = ((elapsed_secs / 0.08) as usize) % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let border_style = app.pane_border(&ActivePane::RecordingList);

    let active_count = app.active_recording_count();
    let title = if !app.search_query.is_empty() {
        format!(" Recordings [/{query}] ({active_count} active) ", query = app.search_query)
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
        items.push(ListItem::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(day_label, Theme::day_header()),
            Span::raw(" "),
            Span::styled("━", Style::new().fg(Theme::dim())),
            Span::styled("━━━", Style::new().fg(Theme::muted())),
            Span::styled("━", Style::new().fg(Theme::dim())),
        ])));

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
                    let glyph = if !crate::tui::anim::reduce_motion()
                        && (secs * 2.0) as i32 % 2 == 0
                    {
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
                    let color =
                        crate::tui::theme::Theme::blend_for(Theme::red(), bright, flash);
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
            let title_text: String = rec
                .stream_title
                .as_deref()
                .unwrap_or("stream")
                .to_string();
            let title_spans = crate::tui::widgets::highlight::highlight_spans(
                &title_text,
                &app.search_query,
                title_base,
                hl_style,
            );

            let platform_icon = match rec.platform {
                PlatformKind::Twitch => Span::styled(
                    " \u{F1E8}",
                    Style::new().fg(Theme::twitch()),
                ),
                PlatformKind::YouTube => Span::styled(
                    " 󰗃",
                    Style::new().fg(Theme::youtube()),
                ),
                PlatformKind::Patreon => Span::styled(
                    " ",
                    Style::new().fg(Theme::patreon()),
                ),
            };

            let duration = Span::styled(
                rec.format_duration(),
                Style::new().fg(Theme::muted()),
            );

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
