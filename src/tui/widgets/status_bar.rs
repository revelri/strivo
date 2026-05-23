use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{ActivePane, AppState};
use crate::platform::PlatformKind;
use crate::plugin::registry::PluginRegistry;
use crate::tui::anim::{easing::Ease, pulse_phase, reduce_motion};
use crate::tui::theme::Theme;

/// REC-dot pulse style. Time-based 2 s ease-in-out cycle (DESIGN.md signature
/// motion); never fully transparent (min opacity 0.25). Honors reduced motion.
fn rec_pulse_style(elapsed_secs: f32) -> Style {
    let opacity = if reduce_motion() {
        1.0
    } else {
        let p = pulse_phase(elapsed_secs, 2.0);
        let eased = Ease::InOutSine.apply(p);
        0.25 + 0.75 * eased
    };
    let red = Theme::red();
    let bg = Theme::hotkey_bar().bg.unwrap_or(Theme::bg());
    let fg = scale_rgb(red, opacity, bg);
    Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
}

/// Search-cursor opacity tween — smooth 1.2 s ease-in-out blink (replaces the
/// ratatui hard-blink which felt jittery against the bar background).
fn search_cursor_color(elapsed_secs: f32) -> Color {
    if reduce_motion() {
        return Theme::primary();
    }
    let p = pulse_phase(elapsed_secs, 1.2);
    let eased = Ease::InOutSine.apply(p);
    let opacity = 0.35 + 0.65 * eased;
    scale_rgb(
        Theme::primary(),
        opacity,
        Theme::hotkey_bar().bg.unwrap_or(Theme::bg()),
    )
}

/// Status-message alpha — 200 ms Ease::Standard enter ramp, hold until 4.5 s,
/// then 500 ms InCubic fade-out blend against the hotkey-bar background.
/// App-level dismissal happens at ~5 s, so this is a perceptual hand-off
/// rather than a separate timer.
fn status_message_color(age_secs: f32) -> Color {
    if reduce_motion() {
        return Theme::fg();
    }
    let alpha = if age_secs < 0.2 {
        Ease::Standard.apply((age_secs / 0.2).clamp(0.0, 1.0))
    } else if age_secs < 4.5 {
        1.0
    } else {
        let t = ((age_secs - 4.5) / 0.5).clamp(0.0, 1.0);
        1.0 - Ease::InCubic.apply(t)
    };
    scale_rgb(
        Theme::fg(),
        alpha,
        Theme::hotkey_bar().bg.unwrap_or(Theme::bg()),
    )
}

/// Blend `color` toward `bg` by `(1 - opacity)`. Returns a flattened RGB so
/// terminals without true-color alpha still see the dimming effect.
fn scale_rgb(color: Color, opacity: f32, bg: Color) -> Color {
    let opacity = opacity.clamp(0.0, 1.0);
    let (cr, cg, cb) = match color {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        _ => return color,
    };
    let (br, bg_, bb) = match bg {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        _ => (0.0, 0.0, 0.0),
    };
    Color::Rgb(
        (br + (cr - br) * opacity).round().clamp(0.0, 255.0) as u8,
        (bg_ + (cg - bg_) * opacity).round().clamp(0.0, 255.0) as u8,
        (bb + (cb - bb) * opacity).round().clamp(0.0, 255.0) as u8,
    )
}

pub fn render(frame: &mut Frame, area: Rect, app: &AppState, registry: &PluginRegistry) {
    let bar_style = Theme::hotkey_bar();
    let key_style = Theme::hotkey_key();
    let bar_bg = Theme::hotkey_bar().bg.unwrap_or(Theme::bg());

    // Persistent banner while the daemon socket is down — overrides
    // everything else so the user can never mistake stale data for live.
    if !app.daemon_connected {
        // 200 ms enter ramp: the banner's fg eases from bar_bg → red so the
        // appearance is a smooth reveal instead of a jarring color flip.
        let enter = app
            .daemon_disconnected_at
            .map(|at| (at.elapsed().as_secs_f32() / 0.2).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let enter = if reduce_motion() {
            1.0
        } else {
            Ease::Standard.apply(enter)
        };
        let msg = format!(
            " ⚠  Daemon disconnected — reconnecting (attempt {}) ",
            app.daemon_reconnect_attempts
        );
        let pad = area.width.saturating_sub(msg.chars().count() as u16) as usize;
        let line = Line::from(vec![
            Span::styled(
                msg,
                Style::new()
                    .fg(scale_rgb(Theme::red(), enter, bar_bg))
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    // If search input is active, render an editable prompt with the cursor
    // visible at `search_cursor` (split the query at that char boundary).
    if app.search_active {
        let chars: Vec<char> = app.search_query.chars().collect();
        let cur = app.search_cursor.min(chars.len());
        let left: String = chars[..cur].iter().collect();
        let right: String = chars[cur..].iter().collect();

        let used = 2 + left.chars().count() + 1 + right.chars().count(); // " /" + left + cursor + right
        let pad = area.width.saturating_sub(used as u16) as usize;

        let cursor_color = search_cursor_color(app.clock.elapsed().as_secs_f32());
        let search_bar = Line::from(vec![
            Span::styled(" /", Style::new().fg(Theme::secondary()).bg(bar_bg)),
            Span::styled(left, Style::new().fg(Theme::fg()).bg(bar_bg)),
            Span::styled(
                "▌",
                Style::new()
                    .fg(cursor_color)
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(right, Style::new().fg(Theme::fg()).bg(bar_bg)),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(search_bar).style(bar_style), area);
        return;
    }

    // StatusBar focus mode: show navigation hints instead of normal buttons
    if app.active_pane == ActivePane::StatusBar {
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(" ", bar_style));
        push_button(
            &mut spans, "Select", "←→", bar_style, key_style, None, bar_bg,
        );
        push_button(
            &mut spans, "Debug", "Enter", bar_style, key_style, None, bar_bg,
        );
        push_button(
            &mut spans, "Back", "Esc", bar_style, key_style, None, bar_bg,
        );

        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let indicators = build_indicators(app, bar_bg, Some(app.selected_indicator));
        let ind_width: usize = indicators.iter().map(|s| s.content.chars().count()).sum();

        let plugin_statuses = registry.status_lines(app);
        let plugin_width: usize = plugin_statuses.iter().map(|s| s.len() + 2).sum();

        let pad = (area.width as usize).saturating_sub(used + ind_width + plugin_width);
        spans.push(Span::styled(" ".repeat(pad), bar_style));

        for status in &plugin_statuses {
            spans.push(Span::styled(
                format!("[{status}] "),
                Style::new().fg(Theme::secondary()).bg(bar_bg),
            ));
        }
        spans.extend(indicators);

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    // Async task tail — shown above the hotkey strip when at least one
    // task is registered (active or recently-terminal). Falls back to
    // the playback / status / hotkey strip when the registry is empty.
    if !app.tasks.is_empty() {
        let active = app.tasks.active();
        let mut summary_parts: Vec<String> = Vec::new();
        for t in app.tasks.iter().take(3) {
            summary_parts.push(format!(
                "{} {} {}",
                t.kind.label(),
                t.title,
                t.progress.label()
            ));
        }
        let summary = summary_parts.join(" · ");
        let extra = if app.tasks.len() > 3 {
            format!(" (+{})", app.tasks.len() - 3)
        } else {
            String::new()
        };
        let body = format!(" {active} active · {summary}{extra}");
        let pad = area.width.saturating_sub(body.chars().count() as u16 + 1) as usize;
        let line = Line::from(vec![
            Span::styled(
                body,
                Style::new()
                    .fg(Theme::secondary())
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    // Playback overlay takes priority over the hotkey strip while mpv
    // is running. Shows pause icon + pos/duration + speed + volume +
    // the abbreviated file name.
    if let Some(ref pb) = app.playback {
        let secs_to_mmss = |s: f64| -> String {
            let s = s.max(0.0) as u64;
            format!("{}:{:02}", s / 60, s % 60)
        };
        let pos = secs_to_mmss(pb.position_secs);
        let dur = pb
            .duration_secs
            .map(secs_to_mmss)
            .unwrap_or_else(|| "?".to_string());
        let icon = if pb.is_paused { "⏸" } else { "▶" };
        let mut label = pb.file_label.clone();
        if label.len() > 30 {
            label.truncate(27);
            label.push('…');
        }
        let body = format!(
            " {icon} {pos} / {dur}  {:.2}x  vol {}%  · {label}",
            pb.speed, pb.volume
        );
        let pad = area.width.saturating_sub(body.chars().count() as u16 + 1) as usize;
        let line = Line::from(vec![
            Span::styled(
                body,
                Style::new()
                    .fg(Theme::primary())
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    // If a transient status message is live (set within the last ~5 s, per the
    // app-level auto-dismiss tick), render it in place of the hotkey bar so
    // one-shot feedback actually reaches the user.
    if !app.status_message.is_empty() {
        let msg = app.status_message.clone();
        let pad = area.width.saturating_sub(msg.chars().count() as u16 + 1) as usize;
        let age = app
            .status_message_at
            .map(|at| at.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let line = Line::from(vec![
            Span::styled(" ", bar_style),
            Span::styled(
                msg,
                Style::new()
                    .fg(status_message_color(age))
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), bar_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(bar_style), area);
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", bar_style));

    // Mode/pane chip — zellij-style colored pill at the leftmost segment.
    // Shows the active pane so the user always knows which keymap layer
    // is in force. The chip color is the focused-border accent (primary)
    // so the pane border and the chip glow together when the pane gains
    // focus. Search/Wizard/Playback have their own dedicated bars above,
    // so by the time we hit this code path the mode is effectively
    // "browsing pane X."
    let (chip_label, chip_fg, chip_bg) = pane_chip(app);
    spans.push(Span::styled(
        format!(" {chip_label} "),
        Style::new()
            .fg(chip_fg)
            .bg(chip_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" ", bar_style));

    // Filter-active indicator — visible whenever a search filter is in force
    // but the input is not focused. Spells out what Esc will do.
    if !app.search_query.is_empty() {
        let (matched, total) = filter_counts(app);
        let label = format!(
            "[/{}] {}/{} · Esc clears ",
            app.search_query, matched, total
        );
        spans.push(Span::styled(
            label,
            Style::new()
                .fg(Theme::secondary())
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Pulsing REC dot when a recording is active (DESIGN.md signature motion).
    let active_recordings = app.active_recording_count();
    if active_recordings > 0 {
        spans.push(Span::styled(
            "● ",
            rec_pulse_style(app.clock.elapsed().as_secs_f32()),
        ));
        spans.push(Span::styled(
            format!("REC({active_recordings}) "),
            Style::new()
                .fg(Theme::red())
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let shimmer = hotkey_shimmer_char(app);

    // Context-sensitive buttons
    match app.active_pane {
        ActivePane::Detail => {
            push_button(
                &mut spans, "Record", "r", bar_style, key_style, shimmer, bar_bg,
            );
            push_button(
                &mut spans, "Watch", "w", bar_style, key_style, shimmer, bar_bg,
            );
        }
        ActivePane::RecordingList => {
            push_button(
                &mut spans, "Stop", "s", bar_style, key_style, shimmer, bar_bg,
            );
            push_button(
                &mut spans, "Play", "p", bar_style, key_style, shimmer, bar_bg,
            );
            push_button(
                &mut spans, "Info", "i", bar_style, key_style, shimmer, bar_bg,
            );
        }
        _ => {}
    }

    // Always-visible buttons
    push_button(
        &mut spans, "Search", "/", bar_style, key_style, shimmer, bar_bg,
    );
    push_button(
        &mut spans, "Intel", "I", bar_style, key_style, shimmer, bar_bg,
    );
    push_button(
        &mut spans, "Config", "C", bar_style, key_style, shimmer, bar_bg,
    );
    push_button(
        &mut spans, "Help", "?", bar_style, key_style, shimmer, bar_bg,
    );
    push_button(
        &mut spans,
        "Recordings",
        "L",
        bar_style,
        key_style,
        shimmer,
        bar_bg,
    );
    push_button(
        &mut spans, "Log", "F", bar_style, key_style, shimmer, bar_bg,
    );
    push_button(
        &mut spans, "Quit", "q", bar_style, key_style, shimmer, bar_bg,
    );

    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let total_width = area.width as usize;

    // Build platform indicators (hidden when unconfigured)
    let indicators = build_indicators(app, bar_bg, None);
    let ind_width: usize = indicators.iter().map(|s| s.content.chars().count()).sum();

    // Plugin status indicators
    let plugin_statuses = registry.status_lines(app);
    let plugin_width: usize = plugin_statuses.iter().map(|s| s.len() + 2).sum();

    let pad = total_width.saturating_sub(used + ind_width + plugin_width);
    spans.push(Span::styled(" ".repeat(pad), bar_style));

    for status in &plugin_statuses {
        spans.push(Span::styled(
            format!("[{status}] "),
            Style::new().fg(Theme::secondary()).bg(bar_bg),
        ));
    }
    spans.extend(indicators);

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(bar_style);
    frame.render_widget(bar, area);
}

/// Build the platform indicator spans. Only configured platforms are shown.
/// `highlight_idx` highlights one indicator when in StatusBar focus mode.
fn build_indicators<'a>(
    app: &AppState,
    bar_bg: ratatui::style::Color,
    highlight_idx: Option<usize>,
) -> Vec<Span<'a>> {
    let bar_style = Theme::hotkey_bar();
    let mut spans = Vec::new();
    let mut idx = 0usize;

    let configured = configured_platforms(app);

    for kind in &configured {
        let (connected, has_errors) = match kind {
            PlatformKind::Twitch => (
                app.twitch_connected,
                app.platform_errors.get(kind).is_some_and(|e| !e.is_empty()),
            ),
            PlatformKind::YouTube => (
                app.youtube_connected,
                app.platform_errors.get(kind).is_some_and(|e| !e.is_empty()),
            ),
            PlatformKind::Patreon => (
                app.patreon_connected,
                app.platform_errors.get(kind).is_some_and(|e| !e.is_empty()),
            ),
        };

        let is_highlighted = highlight_idx == Some(idx);

        let (bullet, color) = if connected && !has_errors {
            ("● ", Theme::green())
        } else if connected && has_errors {
            ("● ", Theme::secondary()) // amber: connected but errors
        } else {
            ("○ ", Theme::secondary()) // amber: configured but not connected
        };

        let label = match kind {
            PlatformKind::Twitch => "TW ",
            PlatformKind::YouTube => "YT ",
            PlatformKind::Patreon => "PA ",
        };

        let style = if is_highlighted {
            Style::new()
                .fg(color)
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new().fg(color).bg(bar_bg)
        };

        let label_style = if is_highlighted {
            Style::new()
                .fg(Theme::fg())
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            bar_style
        };

        spans.push(Span::styled(bullet, style));
        spans.push(Span::styled(label, label_style));
        idx += 1;
    }

    spans
}

/// Return the list of configured platforms in display order.
pub fn configured_platforms(app: &AppState) -> Vec<PlatformKind> {
    let mut platforms = Vec::new();
    if app.config.twitch.is_some() {
        platforms.push(PlatformKind::Twitch);
    }
    if app.config.youtube.is_some() {
        platforms.push(PlatformKind::YouTube);
    }
    if app.config.patreon.is_some() {
        platforms.push(PlatformKind::Patreon);
    }
    platforms
}

fn push_button<'a>(
    spans: &mut Vec<Span<'a>>,
    label: &'a str,
    key: &'a str,
    bar_style: Style,
    key_style: Style,
    shimmer: Option<(char, f32)>,
    bar_bg: Color,
) {
    // When a matching key was just pressed, ramp its fg secondary → primary
    // → secondary over 240 ms so the key visibly "fires".
    let ks = match shimmer {
        Some((c, t)) if key.chars().next() == Some(c) => {
            let tri = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
            let blended = Theme::blend_for(Theme::secondary(), Theme::primary(), tri);
            Style::new()
                .fg(blended)
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD)
        }
        _ => key_style,
    };
    spans.push(Span::styled(label, bar_style));
    spans.push(Span::styled(" [", bar_style));
    spans.push(Span::styled(key, ks));
    spans.push(Span::styled("]", bar_style));
    spans.push(Span::styled(" ", bar_style)); // 1-char gap
}

/// Returns `Some((char, t))` when a hotkey was pressed in the last 240 ms.
/// `t` is the normalized progress through the shimmer animation.
fn hotkey_shimmer_char(app: &AppState) -> Option<(char, f32)> {
    if reduce_motion() {
        return None;
    }
    let at = app.last_hotkey_at?;
    let c = app.last_hotkey?;
    let elapsed = at.elapsed().as_secs_f32();
    if elapsed > 0.24 {
        return None;
    }
    Some((c, (elapsed / 0.24).clamp(0.0, 1.0)))
}

/// Build the leftmost mode/pane chip. Format: `MODE · pane`, e.g. `NOR · detail`
/// or `VIS · recordings`. Visual mode flips the chip background to the
/// secondary (amber) accent so multi-select state is unambiguous; normal
/// mode tracks the focused-border ramp by using the primary accent.
fn pane_chip(app: &AppState) -> (String, Color, Color) {
    let mode = app.input_mode.label();
    let pane_label = match &app.active_pane {
        ActivePane::Sidebar => "sidebar",
        ActivePane::Detail => "detail",
        ActivePane::RecordingList => "recordings",
        ActivePane::Settings => "settings",
        ActivePane::Log => "log",
        ActivePane::Schedule => "schedule",
        ActivePane::StatusBar => "statusbar",
        ActivePane::Wizard => "wizard",
        ActivePane::Plugin(_) => "plugin",
    };
    let label = format!("{mode} · {pane_label}");
    let (fg, bg) = match app.input_mode {
        crate::app::InputMode::Visual => (Theme::bg(), Theme::secondary()),
        crate::app::InputMode::Insert => (Theme::bg(), Theme::blue()),
        crate::app::InputMode::Normal => (Theme::bg(), Theme::primary()),
    };
    (label, fg, bg)
}

/// Returns `(matched, total)` for the pane the filter currently applies to.
/// Used by the filter-active indicator so the user can see how many items
/// survived the query.
fn filter_counts(app: &AppState) -> (usize, usize) {
    match app.active_pane {
        ActivePane::RecordingList => {
            let total = app.recordings.len();
            let matched = if app.search_filtered_recordings.is_empty() {
                total
            } else {
                app.search_filtered_recordings.len()
            };
            (matched, total)
        }
        // Sidebar / Detail both filter the channel list
        _ => {
            let total = app.channels.len();
            let matched = if app.search_filtered_channels.is_empty() {
                total
            } else {
                app.search_filtered_channels.len()
            };
            (matched, total)
        }
    }
}
