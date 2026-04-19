use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::app::{ActivePane, AppState};
use crate::plugin::registry::PluginRegistry;
use crate::recording::job::RecordingState;
use crate::tui::theme::Theme;

pub fn render_help(
    frame: &mut Frame,
    area: Rect,
    registry: &PluginRegistry,
    active_pane: &ActivePane,
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
        .border_style(Theme::border_focused())
        .title(" Help ")
        .title_style(Theme::title());

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let global_keys: Vec<(&str, &str)> = vec![
        ("?", "Toggle this help"),
        ("q", "Quit"),
        ("/", "Search / filter"),
        ("F", "Log viewer"),
        ("F5", "Refresh channels now"),
        ("Ctrl+D", "Platform diagnostics"),
        ("Esc", "Clear filter / go back"),
    ];

    let sidebar_keys: Vec<(&str, &str)> = vec![
        ("j/k, ↑/↓", "Navigate channels"),
        ("g/G, Home/End", "Jump first / last"),
        ("Enter/l/→", "Select channel"),
        ("S", "Settings"),
        ("L", "Recording list"),
    ];

    let detail_keys: Vec<(&str, &str)> = vec![
        ("j/k, ↑/↓", "Cycle channels"),
        ("g/G, Home/End", "Jump first / last"),
        ("r", "Start recording"),
        ("R", "Record from start (YT)"),
        ("w", "Watch in mpv"),
        ("a", "Toggle monitor"),
        ("t", "Toggle transcode mode"),
        ("Esc/h/←", "Go back"),
    ];

    let recording_keys: Vec<(&str, &str)> = vec![
        ("j/k, ↑/↓", "Navigate recordings"),
        ("g/G, Home/End", "Jump first / last"),
        ("s", "Stop recording"),
        ("p", "Play recording"),
        ("i", "Recording info"),
        ("Esc/h/←", "Go back"),
    ];

    let log_keys: Vec<(&str, &str)> = vec![
        ("j/k, ↑/↓", "Scroll"),
        ("g/Home", "Jump to top"),
        ("G/End", "Jump to bottom"),
        ("PgUp/Dn", "Scroll page"),
        ("c", "Clear log"),
        ("Esc/h/←", "Go back"),
    ];

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
            Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD),
        ));
        for (_plugin_name, cmd) in &plugin_cmds {
            let key_label = format!("{:?}", cmd.key).replace("Char('", "").replace("')", "");
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{key_label:>10}"),
                    Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD),
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
            let done = matches!(
                j.state,
                RecordingState::Finished | RecordingState::Failed
            );
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Theme::red()))
        .title(" Stopping Recordings ")
        .title_style(
            Style::new()
                .fg(Theme::red())
                .add_modifier(Modifier::BOLD),
        );

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
                Style::new()
                    .fg(Theme::green())
                    .add_modifier(Modifier::BOLD),
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

pub fn render_confirm(frame: &mut Frame, area: Rect, message: &str) {
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Theme::secondary()))
        .title(" Confirm ")
        .title_style(Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD));

    let inner = block.inner(center);
    frame.render_widget(block, center);

    let lines = vec![
        Line::raw(""),
        Line::styled(
            format!("  {message}"),
            Style::new().fg(Theme::fg()),
        ),
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
