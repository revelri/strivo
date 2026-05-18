use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{ActivePane, AppState};
use crate::plugin::registry::PluginRegistry;
use crate::recording::job::RecordingState;
use crate::tui::theme::Theme;

/// Render a `KeyPattern` as a help-overlay-friendly label.
/// `<C-s>` / `<S-Tab>` / single-letter forms.
fn format_pattern(p: &crate::tui::keymap::KeyPattern) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut out = String::new();
    if p.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl+");
    }
    if p.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("Alt+");
    }
    let shift_implicit = matches!(p.code, KeyCode::Char(c) if c.is_uppercase());
    if p.modifiers.contains(KeyModifiers::SHIFT) && !shift_implicit {
        out.push_str("Shift+");
    }
    match p.code {
        KeyCode::Char(' ') => out.push_str("Space"),
        KeyCode::Char(c) => out.push(c),
        KeyCode::Tab => out.push_str("Tab"),
        KeyCode::Enter => out.push_str("Enter"),
        KeyCode::Esc => out.push_str("Esc"),
        KeyCode::Up => out.push('↑'),
        KeyCode::Down => out.push('↓'),
        KeyCode::Left => out.push('←'),
        KeyCode::Right => out.push('→'),
        KeyCode::Home => out.push_str("Home"),
        KeyCode::End => out.push_str("End"),
        KeyCode::PageUp => out.push_str("PgUp"),
        KeyCode::PageDown => out.push_str("PgDn"),
        KeyCode::F(n) => out.push_str(&format!("F{n}")),
        _ => out.push_str(&format!("{:?}", p.code)),
    }
    out
}

/// Collect (key-label, desc) pairs for every chord in `layer` from the
/// keymap table. Used to auto-build pane sections of the help overlay.
fn layer_rows(layer: crate::tui::keymap::Layer) -> Vec<(String, &'static str)> {
    crate::tui::keymap::all_chords()
        .iter()
        .filter(|c| c.layer == layer)
        .map(|c| (format_pattern(&c.key), c.desc))
        .collect()
}

pub fn render_help(
    frame: &mut Frame,
    area: Rect,
    registry: &PluginRegistry,
    active_pane: &ActivePane,
    enter_progress: f32,
) {
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(15),
        Constraint::Min(18),
        Constraint::Percentage(15),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Min(44),
        Constraint::Percentage(20),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_ramp(enter_progress))
        .title(" Help ")
        .title_style(Theme::title());

    let inner = block.inner(center);
    frame.render_widget(block, center);

    // Global keys are auto-generated from the keymap table (M3.3). Any
    // chord added in src/tui/keymap.rs shows up here automatically.
    // Pane sections below stay hardcoded until M3.2 migrates each
    // per-pane match arm into the table.
    let table_global: Vec<(String, &'static str)> =
        crate::tui::keymap::chords_for(crate::tui::keymap::Layer::Global)
            .into_iter()
            .filter(|c| c.layer == crate::tui::keymap::Layer::Global)
            .map(|c| (format_pattern(&c.key), c.desc))
            .collect();
    let mut global_keys: Vec<(&str, &str)> = Vec::new();
    let global_owned: Vec<(String, &'static str)> = table_global;
    for (k, d) in &global_owned {
        global_keys.push((k.as_str(), d));
    }
    // Hand-maintained extras that aren't (yet) in the table: F5 still
    // lives in per-pane handlers, and Esc has overlay-dismiss semantics
    // that are clearer as a separate line.
    global_keys.extend([
        ("F5", "Refresh channels now"),
        ("Esc", "Clear filter / go back"),
    ]);

    // Pane sections auto-generated from the keymap table (M3.followup.e).
    // The base table is the single source of truth; help text falls out
    // of each Chord's `.desc`.
    let sidebar_owned = layer_rows(crate::tui::keymap::Layer::Sidebar);
    let detail_owned = layer_rows(crate::tui::keymap::Layer::Detail);
    let recording_owned = layer_rows(crate::tui::keymap::Layer::RecordingList);
    let schedule_owned = layer_rows(crate::tui::keymap::Layer::Schedule);
    let settings_owned = layer_rows(crate::tui::keymap::Layer::Settings);
    let log_owned = layer_rows(crate::tui::keymap::Layer::Log);

    let sidebar_keys: Vec<(&str, &str)> = sidebar_owned
        .iter()
        .map(|(k, d)| (k.as_str(), *d))
        .collect();
    let detail_keys: Vec<(&str, &str)> =
        detail_owned.iter().map(|(k, d)| (k.as_str(), *d)).collect();
    let recording_keys: Vec<(&str, &str)> = recording_owned
        .iter()
        .map(|(k, d)| (k.as_str(), *d))
        .collect();
    let schedule_keys: Vec<(&str, &str)> = schedule_owned
        .iter()
        .map(|(k, d)| (k.as_str(), *d))
        .collect();
    let settings_keys: Vec<(&str, &str)> = settings_owned
        .iter()
        .map(|(k, d)| (k.as_str(), *d))
        .collect();
    let log_keys: Vec<(&str, &str)> = log_owned.iter().map(|(k, d)| (k.as_str(), *d)).collect();

    // Wizard stays hardcoded — its keys aren't in the table because they
    // intentionally diverge from universal navigation (tab-style auth
    // platform switcher).
    let wizard_keys: Vec<(&str, &str)> = vec![
        ("o", "Open auth URL in browser"),
        ("Esc", "Dismiss wizard"),
        ("q", "Quit"),
    ];

    let mut lines: Vec<Line> = vec![Line::raw("")];

    let sections: Vec<(&str, &[(&str, &str)], bool)> = vec![
        ("Global", &global_keys, true),
        (
            "Sidebar",
            &sidebar_keys,
            matches!(active_pane, ActivePane::Sidebar),
        ),
        (
            "Channel Detail",
            &detail_keys,
            matches!(active_pane, ActivePane::Detail),
        ),
        (
            "Recordings",
            &recording_keys,
            matches!(active_pane, ActivePane::RecordingList),
        ),
        (
            "Schedule",
            &schedule_keys,
            matches!(active_pane, ActivePane::Schedule),
        ),
        (
            "Settings",
            &settings_keys,
            matches!(active_pane, ActivePane::Settings),
        ),
        (
            "Log Viewer",
            &log_keys,
            matches!(active_pane, ActivePane::Log),
        ),
        (
            "Setup Wizard",
            &wizard_keys,
            matches!(active_pane, ActivePane::Wizard),
        ),
    ];

    for (title, keys, is_active) in &sections {
        let title_style = if *is_active {
            Style::new()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Theme::muted()).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::styled(format!("  {title}"), title_style));
        for (key, desc) in *keys {
            let (key_style, desc_style) = if *is_active {
                (
                    Style::new()
                        .fg(Theme::secondary())
                        .add_modifier(Modifier::BOLD),
                    Style::new().fg(Theme::fg()),
                )
            } else {
                (
                    Style::new().fg(Theme::muted()),
                    Style::new().fg(Theme::muted()),
                )
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{key:>10}"), key_style),
                Span::raw("  "),
                Span::styled(*desc, desc_style),
            ]));
        }
        lines.push(Line::raw(""));
    }

    // Plugin commands
    let plugin_cmds = registry.all_commands();
    if !plugin_cmds.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "  Plugins",
            Style::new()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        ));
        for (_plugin_name, cmd) in &plugin_cmds {
            let key_label = format!("{:?}", cmd.key)
                .replace("Char('", "")
                .replace("')", "");
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{key_label:>10}"),
                    Style::new()
                        .fg(Theme::secondary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(cmd.description, Style::new().fg(Theme::fg())),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Modal shown while `AllRecordingsStopped` is pending. Counts down the
/// deadline and lists each job with a checkmark as it finishes, so the user
/// can see progress instead of staring at a frozen screen.
pub fn render_stopping(frame: &mut Frame, area: Rect, app: &AppState) {
    let Some(deadline) = app.stop_all_deadline else {
        return;
    };

    let now = std::time::Instant::now();
    let remaining = if deadline > now {
        (deadline - now).as_secs()
    } else {
        0
    };

    // Collect jobs that were active when StopAll was sent — we use
    // "currently active" as a proxy. Once each transitions to Finished /
    // Stopped / Failed, we render a ✓ next to it.
    let jobs: Vec<&crate::recording::job::RecordingJob> = app.recordings.values().collect();

    // Pick the jobs we care about (anything except Pending) so finished
    // ones can still be shown with their green check.
    let mut rows: Vec<(bool, String)> = jobs
        .iter()
        .filter(|j| {
            matches!(
                j.state,
                RecordingState::Recording
                    | RecordingState::ResolvingUrl
                    | RecordingState::Stopping
                    | RecordingState::Finished
                    | RecordingState::Failed
            )
        })
        .map(|j| {
            let done = matches!(j.state, RecordingState::Finished | RecordingState::Failed);
            (done, j.channel_name.clone())
        })
        .collect();
    rows.sort_by_key(|(done, name)| (*done, name.clone()));

    let still = rows.iter().filter(|(done, _)| !*done).count();
    let height = (8 + rows.len().min(8) as u16).min(area.height.saturating_sub(2));

    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(25),
        Constraint::Length(height),
        Constraint::Percentage(25),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Min(44),
        Constraint::Percentage(25),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let enter_progress = app.overlay_enter(crate::app::OverlayKey::Stopping, 0.18);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Theme::blend_for(Theme::dim(), Theme::red(), enter_progress)))
        .title(" Stopping Recordings ")
        .title_style(Style::new().fg(Theme::red()).add_modifier(Modifier::BOLD));

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{} still stopping", still),
            Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  ·  "),
        Span::styled(
            format!("{}s remaining", remaining),
            Style::new().fg(Theme::secondary()),
        ),
    ]));
    lines.push(Line::raw(""));

    for (done, name) in rows.iter().take(8) {
        let (glyph, style) = if *done {
            (
                "✓",
                Style::new().fg(Theme::green()).add_modifier(Modifier::BOLD),
            )
        } else {
            ("●", Style::new().fg(Theme::red()))
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(glyph, style),
            Span::raw("  "),
            Span::styled(name.clone(), Style::new().fg(Theme::fg())),
        ]));
    }

    if rows.len() > 8 {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("  …and {} more", rows.len() - 8),
            Style::new().fg(Theme::muted()),
        ));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn render_confirm(frame: &mut Frame, area: Rect, message: &str, enter_progress: f32) {
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(35),
        Constraint::Length(7),
        Constraint::Percentage(35),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Min(40),
        Constraint::Percentage(25),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let border_color = Theme::blend_for(Theme::dim(), Theme::secondary(), enter_progress);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border_color))
        .title(" Confirm ")
        .title_style(
            Style::new()
                .fg(Theme::secondary())
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let lines = vec![
        Line::raw(""),
        Line::styled(format!("  {message}"), Style::new().fg(Theme::fg())),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("[y]", Theme::key_hint()),
            Span::raw(" Yes  "),
            Span::styled("[n]", Theme::key_hint()),
            Span::raw(" No"),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
