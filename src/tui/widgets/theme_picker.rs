//! Theme picker overlay (Ctrl+T).
//!
//! Two-column modal: left = scrollable theme list with source badge, right =
//! live swatch grid and sample widgets rendered in the *currently applied*
//! preview theme. The preview is installed the moment the user moves the
//! cursor (see [`crate::app::AppState::preview_picker_selection`]), so the
//! right pane always reflects what would be committed on Enter.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
    },
    Frame,
};

use crate::app::AppState;
use crate::tui::anim::{easing::Ease, pulse_phase, reduce_motion};
use crate::tui::theme::{builtin_themes, Theme};

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let Some(state) = app.theme_picker.as_ref() else {
        return;
    };

    // 70% width, 70% height, capped so it doesn't engulf huge terminals.
    let h = area.height.saturating_mul(7) / 10;
    let h = h.min(28).max(18);
    let w = area.width.saturating_mul(7) / 10;
    let w = w.min(80).max(56);

    let [_, row, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(h),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(w),
        Constraint::Fill(1),
    ])
    .areas(row);

    frame.render_widget(Clear, center);

    // 180 ms fade-in: fg fades bg → primary on the border as it mounts.
    let enter = enter_progress(state.opened_at);
    let border_style = Style::new().fg(blend(Theme::dim(), Theme::primary(), enter));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .padding(Padding::horizontal(1))
        .title(" Theme Picker ")
        .title_style(Theme::title());
    let inner = block.inner(center);
    frame.render_widget(block, center);

    let [list_area, preview_area] =
        Layout::horizontal([Constraint::Length(28), Constraint::Fill(1)]).areas(inner);

    render_theme_list(frame, list_area, state);
    render_preview(frame, preview_area, state);
}

fn render_theme_list(frame: &mut Frame, area: Rect, state: &crate::app::ThemePickerState) {
    let builtins: std::collections::HashSet<String> =
        builtin_themes().into_iter().map(|t| t.name).collect();

    let items: Vec<ListItem> = state
        .themes
        .iter()
        .map(|name| {
            let source = if builtins.contains(name) {
                Span::styled(" built-in", Style::new().fg(Theme::muted()))
            } else {
                Span::styled(" user", Style::new().fg(Theme::secondary()))
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {name}"), Style::new().fg(Theme::fg())),
                source,
            ]))
        })
        .collect();

    let mut lstate = ListState::default();
    lstate.select(Some(state.selected));

    let list = List::new(items)
        .highlight_style(Theme::selected().add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    frame.render_stateful_widget(list, area, &mut lstate);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ↑/↓ ", Theme::key_hint()),
        Span::raw("preview  "),
        Span::styled("Enter", Theme::key_hint()),
        Span::raw(" commit  "),
        Span::styled("Esc", Theme::key_hint()),
        Span::raw(" revert  "),
        Span::styled("R", Theme::key_hint()),
        Span::raw(" rescan"),
    ]))
    .style(Style::new().fg(Theme::muted()));
    let footer_area = Rect {
        x: area.x,
        y: area.y.saturating_add(area.height).saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(footer, footer_area);
}

fn render_preview(frame: &mut Frame, area: Rect, state: &crate::app::ThemePickerState) {
    let [head, swatch, sample] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Fill(1),
    ])
    .areas(area);

    let name = state
        .themes
        .get(state.selected)
        .cloned()
        .unwrap_or_default();
    let head_line = Paragraph::new(Line::from(vec![
        Span::styled("  Preview: ", Style::new().fg(Theme::muted())),
        Span::styled(name, Theme::title()),
    ]));
    frame.render_widget(head_line, head);

    // Swatch grid — one column per slot, colored block + hex label.
    let slots: [(&str, ratatui::style::Color); 12] = [
        ("bg", Theme::bg()),
        ("fg", Theme::fg()),
        ("surface", Theme::surface()),
        ("overlay", Theme::overlay()),
        ("primary", Theme::primary()),
        ("secondary", Theme::secondary()),
        ("dim", Theme::dim()),
        ("muted", Theme::muted()),
        ("red", Theme::red()),
        ("green", Theme::green()),
        ("yellow", Theme::yellow()),
        ("blue", Theme::blue()),
    ];

    let cols = Layout::horizontal(vec![Constraint::Fill(1); slots.len()]).split(swatch);
    for (i, (label, color)) in slots.iter().enumerate() {
        let cell = cols[i];
        let block = Paragraph::new(vec![
            Line::from(Span::styled("  ", Style::new().bg(*color))),
            Line::from(Span::styled("  ", Style::new().bg(*color))),
            Line::from(Span::styled(*label, Style::new().fg(Theme::muted()))),
            Line::from(Span::styled(hex(*color), Style::new().fg(Theme::dim()))),
        ]);
        frame.render_widget(block, cell);
    }

    // Sample widgets: imitate the real UI so the user can see how the theme
    // reads in practice, not just as raw color swatches.
    let pulse = if reduce_motion() {
        1.0
    } else {
        Ease::InOutSine.apply(pulse_phase(state.opened_at.elapsed().as_secs_f32(), 2.0))
    };
    let rec_bg = brightness(Theme::red(), 0.75 + 0.25 * pulse);
    let live_bg = brightness(Theme::green(), 0.75 + 0.25 * pulse);

    let lines = vec![
        Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                " LIVE ",
                Style::new()
                    .fg(Theme::bg())
                    .bg(live_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                " REC ",
                Style::new()
                    .fg(Theme::bg())
                    .bg(rec_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "stream title here",
                Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("█ ", Style::new().fg(Theme::primary())),
            Span::styled(
                "Selected row",
                Style::new().fg(Theme::fg()).bg(Theme::surface()),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("  Unfocused row", Style::new().fg(Theme::muted())),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("[?]", Theme::key_hint()),
            Span::raw(" Help   "),
            Span::styled("[/]", Theme::key_hint()),
            Span::raw(" Search   "),
            Span::styled("[q]", Theme::key_hint()),
            Span::raw(" Quit"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("error message in theme::error()", Theme::error()),
        ]),
    ];

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::new().fg(Theme::fg()).bg(Theme::bg()));
    frame.render_widget(para, sample);
}

fn hex(c: ratatui::style::Color) -> String {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "—".to_string(),
    }
}

/// Linear blend between two RGB colors — used for the border fade-in.
fn blend(a: ratatui::style::Color, b: ratatui::style::Color, t: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f32 + (br as f32 - ar as f32) * t).round() as u8,
            (ag as f32 + (bg as f32 - ag as f32) * t).round() as u8,
            (ab as f32 + (bb as f32 - ab as f32) * t).round() as u8,
        ),
        _ => b,
    }
}

/// Scale an RGB color's brightness by `factor` (0..=1).
fn brightness(c: ratatui::style::Color, factor: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    let f = factor.clamp(0.0, 1.0);
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * f).round().clamp(0.0, 255.0) as u8,
            (g as f32 * f).round().clamp(0.0, 255.0) as u8,
            (b as f32 * f).round().clamp(0.0, 255.0) as u8,
        ),
        _ => c,
    }
}

/// Eased 0→1 over the first 180 ms of the overlay's life. Uses [`Ease::Standard`].
fn enter_progress(opened_at: std::time::Instant) -> f32 {
    if reduce_motion() {
        return 1.0;
    }
    let elapsed = opened_at.elapsed().as_secs_f32();
    let t = (elapsed / 0.18).clamp(0.0, 1.0);
    Ease::Standard.apply(t)
}
