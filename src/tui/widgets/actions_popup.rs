//! Actions popup (`a`). spotify-player-style verb menu scoped to the
//! focused item — or the selection set if non-empty.
//!
//! M4 MVP scope:
//!   - Built-in verbs on RecordingList: Play, Properties, Copy path,
//!     Delete (Shift+D), Rename (Shift+R), Open in folder.
//!   - "Apply to selection set when non-empty" gating — the menu
//!     header shows "Acting on N recordings" vs "Acting on cursor".
//!
//! Plugin-contributed verbs are M5: the host needs a
//! `PluginCommand::Scope::Item` field for plugins to register their
//! verbs against item types, plus a dispatch hook. Both are tracked
//! against the plan's D5+X5 phase 2.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::AppState;
use crate::tui::keymap::KeyAction;
use crate::tui::theme::Theme;

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// One verb in the popup. Built-in entries route through a
/// [`KeyAction`] in `dispatch`; plugin-contributed entries carry the
/// plugin's name + verb name so the dispatcher can route them via the
/// plugin's `on_key` (or via a dedicated `PluginVerb` route the host
/// adds later).
#[derive(Debug, Clone)]
pub struct ActionEntry {
    pub label: String,
    pub desc: String,
    pub dispatch: ActionDispatch,
    /// Whether the verb is meaningful for multi-selection. Verbs that
    /// only make sense on a single cursor row (e.g. Rename) get a dim
    /// tag in the popup when the selection set is non-empty.
    pub multi: bool,
}

/// What pressing Enter on this row does. (D5+X5.)
#[derive(Debug, Clone)]
pub enum ActionDispatch {
    /// Built-in path — apply a [`KeyAction`] through
    /// [`crate::app::AppState::apply_key_action`].
    KeyAction(KeyAction),
    /// Plugin-contributed verb. The host surfaces it via the status
    /// message + sends the plugin a focused-pane activate so the
    /// plugin's own on_key can pick up the next interaction. A
    /// future commit adds a dedicated routing channel; for now the
    /// activation + status surface is the visible affordance.
    PluginVerb {
        plugin: &'static str,
        verb: &'static str,
    },
}

/// Built-in verbs for the RecordingList. Plugin-contributed verbs are
/// appended via [`entries_for_recording_list_with_plugins`].
pub fn entries_for_recording_list_builtins() -> Vec<ActionEntry> {
    vec![
        ActionEntry {
            label: "Play".into(),
            desc: "open in mpv".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::PlayRecording),
            multi: false,
        },
        ActionEntry {
            label: "Properties".into(),
            desc: "show metadata + plugin sections".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::ShowRecordingProperties),
            multi: false,
        },
        ActionEntry {
            label: "Copy path".into(),
            desc: "copy file path to system clipboard".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::CopyToClipboard),
            multi: false,
        },
        ActionEntry {
            label: "Open in folder".into(),
            desc: "reveal in file manager".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::OpenInFolder),
            multi: false,
        },
        ActionEntry {
            label: "Rename".into(),
            desc: "rename the recording file".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::RenameRecording),
            multi: false,
        },
        ActionEntry {
            label: "Delete (selection)".into(),
            desc: "move selected recordings to trash".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::TrashSelectedRecordings),
            multi: true,
        },
        ActionEntry {
            label: "Clear selections".into(),
            desc: "drop the multi-select set".into(),
            dispatch: ActionDispatch::KeyAction(KeyAction::ClearRecordingSelections),
            multi: true,
        },
    ]
}

/// Combined verb list: built-ins + plugin-contributed item-scoped
/// verbs registered against [`crate::plugin::ItemKind::Recording`]. (D5+X5.)
pub fn entries_for_recording_list_with_plugins(
    registry: &crate::plugin::registry::PluginRegistry,
) -> Vec<ActionEntry> {
    let mut entries = entries_for_recording_list_builtins();
    for (plugin_name, cmd) in registry.item_commands(crate::plugin::ItemKind::Recording) {
        entries.push(ActionEntry {
            label: format!("{}: {}", plugin_name, cmd.name),
            desc: cmd.description.to_string(),
            dispatch: ActionDispatch::PluginVerb {
                plugin: plugin_name,
                verb: cmd.name,
            },
            multi: false,
        });
    }
    entries
}

/// Back-compat shim — callers that don't have a registry handy get
/// just the built-in list.
#[deprecated(note = "use entries_for_recording_list_with_plugins for full surface")]
pub fn entries_for_recording_list() -> Vec<ActionEntry> {
    entries_for_recording_list_builtins()
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    enter_progress: f32,
) {
    let Some(state) = app.actions_popup.as_ref() else {
        return;
    };

    // 50% width × ~50% height; capped so even on tiny terminals we
    // don't render off the edge.
    let h = ((state.entries.len() + 4) as u16).min((area.height * 6) / 10).max(8);
    let w = (area.width.saturating_mul(5) / 10).clamp(50, 80);

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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_ramp(enter_progress))
        .padding(Padding::horizontal(1))
        .title(" Actions ")
        .title_style(Theme::title());
    let inner = block.inner(center);
    frame.render_widget(block, center);

    let [header_area, list_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Header — what the popup is acting on.
    let n_selected = app.recording_selections_set.len();
    let header_line = if n_selected > 0 {
        Line::from(vec![
            Span::styled(
                format!(" Acting on {n_selected} recordings "),
                Style::new()
                    .fg(Theme::bg())
                    .bg(Theme::secondary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "(multi-target verbs apply to the set)",
                Style::new().fg(Theme::muted()),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " Acting on cursor row ",
                Style::new()
                    .fg(Theme::bg())
                    .bg(Theme::primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "(press [v] in the list first to multi-select)",
                Style::new().fg(Theme::muted()),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(header_line), header_area);

    let lines: Vec<Line> = state
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let selected = i == state.selected;
            let marker = if selected { "▶ " } else { "  " };
            let dim = !e.multi && n_selected > 1;
            let label_style = if selected {
                Style::new()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD)
            } else if dim {
                Style::new().fg(Theme::dim())
            } else {
                Style::new().fg(Theme::fg())
            };
            Line::from(vec![
                Span::styled(marker.to_string(), Style::new().fg(Theme::secondary())),
                Span::styled(format!("{:<28}", truncate(&e.label, 28)), label_style),
                Span::raw("  "),
                Span::styled(e.desc.clone(), Style::new().fg(Theme::muted())),
                if dim {
                    Span::styled(
                        "  (single-target)".to_string(),
                        Style::new().fg(Theme::dim()),
                    )
                } else {
                    Span::raw("")
                },
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        list_area,
    );

    let hint = Line::from(vec![
        Span::styled("[↑/↓]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" nav  "),
        Span::styled("[Enter]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" run  "),
        Span::styled("[Esc]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" close"),
    ]);
    frame.render_widget(Paragraph::new(hint), hint_area);
}
