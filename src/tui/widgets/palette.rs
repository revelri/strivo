//! Command palette overlay (`:`). Fuzzy-filtered modal listing every
//! [`KeyAction`] in the keymap table plus the active plugin commands.
//!
//! The widget is render-only — typing / cursor / dispatch live in
//! `app.rs` and `tui/mod.rs`. We rebuild the filtered list on every
//! render from the keymap table; the cost is negligible (< 200 actions)
//! and keeps the state model trivially small.
//!
//! Layout (60% × 70%, capped):
//!
//! ```text
//! ┌─ Command palette ──────────────────────────────────┐
//! │ :search query                                      │
//! │ ─────────────────────────────────────────────────  │
//! │ ▶ Quit                  quit (confirm if recording)│
//! │   ThemePickerOpen       theme picker               │
//! │   EventLogToggle        event log                  │
//! │ …                                                  │
//! │ ─────────────────────────────────────────────────  │
//! │ [↑/↓] nav  [Enter] run  [Esc] close  [Tab] scope   │
//! └────────────────────────────────────────────────────┘
//! ```
//!
//! Resource scopes (X4) prefix the query: typing `:` followed by
//! `presets`, `edls`, `batches`, `transcripts`, or `clips` and pressing
//! Tab routes the palette to that resource's list instead of the
//! global action set.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::{AppState, PaletteScope};
use crate::plugin::registry::PluginRegistry;
use crate::tui::theme::Theme;

/// One row in the palette result list.
#[derive(Debug, Clone)]
pub struct PaletteRow {
    /// Display name (e.g. "Quit", "EventLogToggle", or for plugin
    /// commands the action label).
    pub label: String,
    /// One-line description shown in the right column.
    pub desc: String,
    /// What dispatcher should fire when Enter is pressed.
    pub dispatch: PaletteDispatch,
    /// Fuzzy-match score (higher is better). Drives result order.
    pub score: i32,
}

/// What gets executed on Enter.
#[derive(Debug, Clone)]
pub enum PaletteDispatch {
    /// Apply a built-in [`crate::tui::keymap::KeyAction`] via
    /// [`AppState::apply_key_action`].
    KeyAction(crate::tui::keymap::KeyAction),
    /// Activate a plugin pane by `PaneId`.
    PluginPane(crate::plugin::PaneId),
    /// Resource scopes (X4) — switch the palette into a scoped list.
    SwitchScope(PaletteScope),
    /// Palette-local action — recorder helpers (save-preset, clear-log)
    /// and similar metacommands that don't fit the KeyAction enum.
    LocalAction(PaletteLocal),
}

/// Palette-local meta-commands. (X3 + UX polish.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteLocal {
    /// Prompt for a name, then snapshot the current command_log to
    /// `~/.local/share/strivo/command-logs/<name>.json`.
    SavePreset,
    /// Clear the in-memory command log without saving.
    ClearLog,
    /// Open the preset list (read-only for now — replay is M5).
    ListPresets,
}

impl PaletteLocal {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SavePreset => "palette:save-preset",
            Self::ClearLog => "palette:clear-log",
            Self::ListPresets => "palette:list-presets",
        }
    }

    pub fn desc(&self) -> &'static str {
        match self {
            Self::SavePreset => "snapshot session command log to a preset file",
            Self::ClearLog => "clear the in-memory command log",
            Self::ListPresets => "list saved command-log presets",
        }
    }
}

/// Build the filtered, ranked row list from the current query.
/// Pulled out so dispatch ("Enter on selected row") and render share
/// one source of truth.
pub fn build_rows(
    app: &AppState,
    registry: &PluginRegistry,
) -> Vec<PaletteRow> {
    let Some(state) = app.palette.as_ref() else {
        return Vec::new();
    };

    // X4 — scoped resource listings. When a scope is active the
    // palette stops looking at the global action list and instead
    // renders rows drawn from that scope's resource registry.
    if let Some(scope) = state.scope {
        return build_scoped_rows(scope, state.query.trim(), app);
    }

    // Distinguish resource-switch queries (typed as `:scope`) from
    // ordinary action search.
    let q = state.query.trim();
    if q.starts_with(':') && q.len() > 1 {
        // Show scope switchers matching the remainder.
        let needle = &q[1..];
        let mut out: Vec<PaletteRow> = Vec::new();
        for s in [
            PaletteScope::Presets,
            PaletteScope::Edls,
            PaletteScope::Batches,
            PaletteScope::Transcripts,
            PaletteScope::Clips,
        ] {
            if let Some(m) = crate::search::fuzzy_match(needle, s.label()) {
                out.push(PaletteRow {
                    label: format!(":{}", s.label()),
                    desc: scope_desc(&s).into(),
                    dispatch: PaletteDispatch::SwitchScope(s),
                    score: m.score + 50, // prefer scope routes
                });
            }
        }
        out.sort_by(|a, b| b.score.cmp(&a.score));
        return out;
    }

    // Otherwise: rank everything by fuzzy match against `q`.
    let needle = q;
    let mut out: Vec<PaletteRow> = Vec::new();

    // Built-in KeyActions.
    let mut seen: std::collections::HashSet<&'static str> =
        std::collections::HashSet::new();
    for chord in crate::tui::keymap::all_chords() {
        let name = action_name(chord.action);
        if !seen.insert(name) {
            continue;
        }
        let row = score_row(
            needle,
            name,
            chord.desc,
            PaletteDispatch::KeyAction(chord.action),
        );
        if let Some(r) = row {
            out.push(r);
        }
    }

    // Palette-local meta-commands (X3). Always offered alongside the
    // keymap actions so the user can fuzzy-find them like anything else.
    for local in [
        PaletteLocal::SavePreset,
        PaletteLocal::ClearLog,
        PaletteLocal::ListPresets,
    ] {
        if let Some(r) = score_row(
            needle,
            local.label(),
            local.desc(),
            PaletteDispatch::LocalAction(local),
        ) {
            out.push(r);
        }
    }

    // Plugin commands. PluginCommand.name is the action label;
    // PluginCommand.description is the help line. Plugin commands
    // currently route by activating the plugin's first pane — the
    // plugin's own on_key handles the specifics.
    for (plugin_name, cmd) in registry.all_commands() {
        let label = format!("{plugin_name}.{}", cmd.name);
        let dispatch = if let Some(pane) = plugin_pane_for(registry, plugin_name) {
            PaletteDispatch::PluginPane(pane)
        } else {
            continue;
        };
        let row = score_row(needle, &label, cmd.description, dispatch);
        if let Some(r) = row {
            out.push(r);
        }
    }

    out.sort_by(|a, b| b.score.cmp(&a.score));
    out.truncate(50);
    out
}

fn plugin_pane_for(
    registry: &PluginRegistry,
    plugin_name: &str,
) -> Option<crate::plugin::PaneId> {
    let plugin = registry.plugin_ref(plugin_name)?;
    plugin.panes().into_iter().next()
}

/// X4 — per-scope row builders. Each scope returns the rows for its
/// resource family; selection dispatches a KeyAction (when one applies)
/// or [`PaletteDispatch::LocalAction`] for descriptive entries.
fn build_scoped_rows(
    scope: PaletteScope,
    query: &str,
    _app: &AppState,
) -> Vec<PaletteRow> {
    let mut out: Vec<PaletteRow> = Vec::new();
    match scope {
        PaletteScope::Presets => {
            // Crunchr preset names lookup. We don't have access to the
            // plugin from here without registering a list-fn upfront,
            // so we walk both built-in names + on-disk files. Selecting
            // a preset clears the scope and surfaces the name in the
            // status bar — the user then activates the Crunchr pane
            // and runs the preset there. Full plumbing waits on C1
            // phase 2.
            let mut presets: Vec<String> = vec![
                "fast-cheap".into(),
                "quality-local".into(),
                "quality-api".into(),
            ];
            // On-disk presets via the well-known directory pattern.
            if let Some(base) = directories::BaseDirs::new() {
                let dir = base
                    .config_dir()
                    .join("strivo")
                    .join("crunchr")
                    .join("presets");
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.path().file_stem().and_then(|s| s.to_str()) {
                            if !presets.iter().any(|p| p == name) {
                                presets.push(name.to_string());
                            }
                        }
                    }
                }
            }
            for name in presets {
                if let Some(r) = score_row_meta(
                    query,
                    &name,
                    "crunchr preset (select to copy name to status)",
                    PaletteDispatch::LocalAction(PaletteLocal::ListPresets),
                ) {
                    out.push(r);
                }
            }
        }
        PaletteScope::Edls => {
            // List EDL JSON files in the host's edls/ dir.
            let dir = directories::BaseDirs::new()
                .map(|b| b.data_local_dir().join("strivo").join("edls"))
                .unwrap_or_default();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|s| s.to_str()) != Some("json") {
                        continue;
                    }
                    if let Some(name) = p.file_stem().and_then(|s| s.to_str()) {
                        if let Some(r) = score_row_meta(
                            query,
                            name,
                            "edl on disk",
                            PaletteDispatch::LocalAction(PaletteLocal::ListPresets),
                        ) {
                            out.push(r);
                        }
                    }
                }
            }
            if out.is_empty() {
                out.push(PaletteRow {
                    label: "(no EDLs)".into(),
                    desc: "Editor saves clip lists here; none yet.".into(),
                    dispatch: PaletteDispatch::LocalAction(PaletteLocal::ListPresets),
                    score: 0,
                });
            }
        }
        PaletteScope::Batches => {
            out.push(PaletteRow {
                label: "(host pipeline registry)".into(),
                desc: "submitted pipelines — DAG overlay shows full state".into(),
                dispatch: PaletteDispatch::LocalAction(PaletteLocal::ListPresets),
                score: 0,
            });
        }
        PaletteScope::Transcripts => {
            out.push(PaletteRow {
                label: "(see Crunchr Search)".into(),
                desc: "Crunchr pane lists finished transcripts; use its own search".into(),
                dispatch: PaletteDispatch::LocalAction(PaletteLocal::ListPresets),
                score: 0,
            });
        }
        PaletteScope::Clips => {
            // Clip-EDLs live as `<recording>.edl.json` sidecars; the
            // canonical list lives with the Editor pane. Surfacing
            // them here pre-Editor-rewrite would mean walking the
            // recordings directory; that's a future commit.
            out.push(PaletteRow {
                label: "(see Editor pane)".into(),
                desc: "clip EDLs live as recording sidecars".into(),
                dispatch: PaletteDispatch::LocalAction(PaletteLocal::ListPresets),
                score: 0,
            });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score));
    out
}

/// Variant of score_row that takes ownership of `label`+`desc` as
/// `&str` — used by scoped builders where the strings already live in
/// the caller.
fn score_row_meta(
    needle: &str,
    label: &str,
    desc: &str,
    dispatch: PaletteDispatch,
) -> Option<PaletteRow> {
    score_row(needle, label, desc, dispatch)
}

fn score_row(
    needle: &str,
    label: &str,
    desc: &str,
    dispatch: PaletteDispatch,
) -> Option<PaletteRow> {
    if needle.is_empty() {
        return Some(PaletteRow {
            label: label.to_string(),
            desc: desc.to_string(),
            dispatch,
            score: 0,
        });
    }
    // Score against both the label and the description; take the max.
    let m_label = crate::search::fuzzy_match(needle, label);
    let m_desc = crate::search::fuzzy_match(needle, desc);
    let score = match (m_label, m_desc) {
        (None, None) => return None,
        (Some(a), None) => a.score,
        (None, Some(b)) => b.score - 5, // mild penalty for desc-only matches
        (Some(a), Some(b)) => a.score.max(b.score),
    };
    Some(PaletteRow {
        label: label.to_string(),
        desc: desc.to_string(),
        dispatch,
        score,
    })
}

fn action_name(action: crate::tui::keymap::KeyAction) -> &'static str {
    crate::tui::keymap::KeyAction::name(&action)
}

fn scope_desc(scope: &PaletteScope) -> &'static str {
    match scope {
        PaletteScope::Presets => "wiring presets · Crunchr",
        PaletteScope::Edls => "edit decision lists · Editor",
        PaletteScope::Batches => "submitted pipelines",
        PaletteScope::Transcripts => "finished transcripts · Crunchr",
        PaletteScope::Clips => "Editor clip lists",
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    registry: &PluginRegistry,
    enter_progress: f32,
) {
    let Some(state) = app.palette.as_ref() else {
        return;
    };

    // 60% width × 70% height, capped.
    let h = (area.height.saturating_mul(7) / 10).clamp(16, 28);
    let w = (area.width.saturating_mul(6) / 10).clamp(60, 100);

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

    let title = if let Some(scope) = state.scope {
        format!(" Palette · :{} ", scope.label())
    } else {
        " Palette ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_ramp(enter_progress))
        .padding(Padding::horizontal(1))
        .title(title)
        .title_style(Theme::title());
    let inner = block.inner(center);
    frame.render_widget(block, center);

    let [query_area, _rule, list_area, _rule2, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Query line with a leading ":" prompt and a soft block cursor.
    let chars: Vec<char> = state.query.chars().collect();
    let cur = state.cursor.min(chars.len());
    let left: String = chars[..cur].iter().collect();
    let right: String = chars[cur..].iter().collect();
    let prompt_color = if state.scope.is_some() {
        Theme::secondary()
    } else {
        Theme::primary()
    };
    let query_line = Line::from(vec![
        Span::styled(": ", Style::new().fg(prompt_color).add_modifier(Modifier::BOLD)),
        Span::styled(left, Style::new().fg(Theme::fg())),
        Span::styled("▌", Style::new().fg(prompt_color).add_modifier(Modifier::BOLD)),
        Span::styled(right, Style::new().fg(Theme::fg())),
    ]);
    frame.render_widget(Paragraph::new(query_line), query_area);

    // Build the filtered rows here, on render; cheap enough to skip
    // memoization at ≤200 actions.
    let rows = build_rows(app, registry);
    let visible = list_area.height as usize;
    let mut start = 0usize;
    if state.selected >= visible {
        start = state.selected.saturating_sub(visible.saturating_sub(1));
    }
    let label_width = (inner.width as usize).min(28).max(16);

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, r)| {
            let selected = i == state.selected;
            let marker = if selected { "▶ " } else { "  " };
            let label_truncated = truncate(&r.label, label_width);
            let label_style = if selected {
                Style::new()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Theme::fg())
            };
            Line::from(vec![
                Span::styled(marker.to_string(), Style::new().fg(prompt_color)),
                Span::styled(
                    format!("{label_truncated:<width$}", width = label_width),
                    label_style,
                ),
                Span::raw("  "),
                Span::styled(r.desc.clone(), Style::new().fg(Theme::muted())),
            ])
        })
        .collect();

    let body = if rows.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "  (no matches — Esc to close)",
            Style::new().fg(Theme::muted()),
        )))
    } else {
        Paragraph::new(lines)
    };
    frame.render_widget(body.wrap(Wrap { trim: false }), list_area);

    let hint = Line::from(vec![
        Span::styled("[↑/↓]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" nav  "),
        Span::styled("[Enter]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" run  "),
        Span::styled("[Tab]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" scope  "),
        Span::styled("[Esc]", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" close"),
    ]);
    frame.render_widget(Paragraph::new(hint), hint_area);
}

fn truncate(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count > n {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}
