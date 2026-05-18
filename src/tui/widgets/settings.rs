//! Settings tab — M2.2 redesign.
//!
//! Hierarchical groups (Recording / Output / Theme / Connections /
//! Plugins / Reset) rendered as headers + indented rows. Per-row inline
//! editors:
//! - bool   → toggle on Enter / Space
//! - enum   → cycle on Enter / Space
//! - int    → opens the text-input modal with `SettingsInt`
//! - path   → opens the modal with `SettingsPath` (tilde-expanded)
//! - string → opens the modal with `SettingsString`
//! - status → read-only; Enter opens the wizard or plugin modal
//!
//! `settings_rows()` is the single source of truth for both render
//! and key dispatch — the navigation cursor walks the selectable
//! indices it returns. Settings rows are computed every frame (cheap)
//! so the UI stays in sync with config + connection state.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState},
    Frame,
};

use crate::app::{ActivePane, AppState};
use crate::tui::theme::Theme;

/// One row in the settings list. `kind = Header` is decorative and not
/// selectable.
#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub kind: SettingsKind,
    pub label: String,
    pub value: String,
    /// Optional hint shown in the muted color trailing the value.
    pub hint: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub enum SettingsKind {
    Header,
    Bool {
        key: &'static str,
    },
    /// Cycle through a fixed set of options on Enter / Space.
    Cycle {
        key: &'static str,
    },
    Int {
        key: &'static str,
    },
    Path {
        key: &'static str,
    },
    String {
        key: &'static str,
    },
    /// Read-only status row. `Enter` may trigger a side action (open
    /// wizard, open plugin modal) handled by name.
    Status {
        action: &'static str,
    },
}

pub fn settings_rows(app: &AppState) -> Vec<SettingsRow> {
    use SettingsKind as K;
    let mut rows: Vec<SettingsRow> = Vec::new();

    let h = |label: &str| SettingsRow {
        kind: K::Header,
        label: label.to_string(),
        value: String::new(),
        hint: None,
    };

    rows.push(h("Recording"));
    rows.push(SettingsRow {
        kind: K::Path {
            key: "recording_dir",
        },
        label: "Output directory".into(),
        value: app.config.recording_dir.to_string_lossy().into_owned(),
        hint: Some("~ expands"),
    });
    rows.push(SettingsRow {
        kind: K::String {
            key: "filename_template",
        },
        label: "Filename template".into(),
        value: app.config.recording.filename_template.clone(),
        hint: Some("{channel} {date} {title}"),
    });
    rows.push(SettingsRow {
        kind: K::Bool { key: "transcode" },
        label: "Transcode mode".into(),
        value: if app.config.recording.transcode {
            "on (NVENC)".into()
        } else {
            "off (passthrough)".into()
        },
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::String {
            key: "recording.format.format",
        },
        label: "yt-dlp format selector".into(),
        value: app
            .config
            .recording
            .format
            .format
            .clone()
            .unwrap_or_else(|| "best".into()),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::Cycle {
            key: "recording.format.container",
        },
        label: "Container".into(),
        value: app
            .config
            .recording
            .format
            .container
            .clone()
            .unwrap_or_else(|| "mkv".into()),
        hint: Some("mkv / mp4"),
    });
    rows.push(SettingsRow {
        kind: K::Cycle {
            key: "recording.format.video_codec",
        },
        label: "Video codec".into(),
        value: app
            .config
            .recording
            .format
            .video_codec
            .clone()
            .unwrap_or_else(|| "copy".into()),
        hint: Some("copy / h264_nvenc / libx264"),
    });
    rows.push(SettingsRow {
        kind: K::Cycle {
            key: "recording.format.audio_codec",
        },
        label: "Audio codec".into(),
        value: app
            .config
            .recording
            .format
            .audio_codec
            .clone()
            .unwrap_or_else(|| "copy".into()),
        hint: Some("copy / aac"),
    });
    rows.push(SettingsRow {
        kind: K::Int {
            key: "recording.format.bitrate_kbps",
        },
        label: "Bitrate".into(),
        value: app
            .config
            .recording
            .format
            .bitrate_kbps
            .map(|n| format!("{n} kbps"))
            .unwrap_or_else(|| "unset".into()),
        hint: Some("0 unsets"),
    });

    rows.push(h("Archiver"));
    rows.push(SettingsRow {
        kind: K::Bool {
            key: "archiver.enabled",
        },
        label: "Enabled".into(),
        value: if app.config.archiver.enabled {
            "on"
        } else {
            "off"
        }
        .into(),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::Path {
            key: "archiver.archive_dir",
        },
        label: "Archive directory".into(),
        value: app
            .config
            .archiver
            .archive_dir
            .to_string_lossy()
            .into_owned(),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::String {
            key: "archiver.format",
        },
        label: "yt-dlp format".into(),
        value: app.config.archiver.format.clone(),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::Int {
            key: "archiver.concurrent_fragments",
        },
        label: "Concurrent fragments".into(),
        value: app.config.archiver.concurrent_fragments.to_string(),
        hint: Some("1..=16"),
    });
    rows.push(SettingsRow {
        kind: K::String {
            key: "archiver.rate_limit",
        },
        label: "Rate limit".into(),
        value: if app.config.archiver.rate_limit.is_empty() {
            "none".into()
        } else {
            app.config.archiver.rate_limit.clone()
        },
        hint: Some("e.g. 5M"),
    });

    rows.push(h("Crunchr"));
    rows.push(SettingsRow {
        kind: K::Bool {
            key: "crunchr.enabled",
        },
        label: "Enabled".into(),
        value: if app.config.crunchr.enabled {
            "on"
        } else {
            "off"
        }
        .into(),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::Status {
            action: "crunchr_modal",
        },
        label: "Backend".into(),
        value: app.config.crunchr.backend.clone(),
        hint: Some("Enter: plugin modal"),
    });
    rows.push(SettingsRow {
        kind: K::String {
            key: "crunchr.whisper_model",
        },
        label: "Whisper model".into(),
        value: app
            .config
            .crunchr
            .whisper_model
            .clone()
            .unwrap_or_else(|| "auto".into()),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::Int {
            key: "crunchr.whisper_timeout_secs",
        },
        label: "Whisper timeout".into(),
        value: format!("{}s", app.config.crunchr.whisper_timeout_secs),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::Bool {
            key: "crunchr.analysis.enabled",
        },
        label: "Analysis enabled".into(),
        value: if app.config.crunchr.analysis.enabled {
            "on"
        } else {
            "off"
        }
        .into(),
        hint: None,
    });
    rows.push(SettingsRow {
        kind: K::String {
            key: "crunchr.analysis.model",
        },
        label: "Analysis model".into(),
        value: app.config.crunchr.analysis.model.clone(),
        hint: None,
    });

    rows.push(h("Output"));
    rows.push(SettingsRow {
        kind: K::Int {
            key: "poll_interval_secs",
        },
        label: "Poll interval".into(),
        value: format!("{}s", app.config.poll_interval_secs),
        hint: Some("≥15"),
    });
    rows.push(SettingsRow {
        kind: K::Bool {
            key: "ui.reduce_motion",
        },
        label: "Reduce motion".into(),
        value: if app.config.ui.reduce_motion {
            "on"
        } else {
            "off"
        }
        .into(),
        hint: Some("snap animations to end"),
    });
    rows.push(SettingsRow {
        kind: K::Bool {
            key: "ui.verbose_status",
        },
        label: "Verbose status".into(),
        value: if app.config.ui.verbose_status {
            "on"
        } else {
            "off"
        }
        .into(),
        hint: Some("longer status labels"),
    });

    rows.push(h("Theme"));
    rows.push(SettingsRow {
        kind: K::Cycle { key: "theme" },
        label: "Active theme".into(),
        value: Theme::current_name(),
        hint: Some("Enter cycles · Ctrl+T picker"),
    });

    rows.push(h("Connections"));
    rows.push(SettingsRow {
        kind: K::Status {
            action: "wizard_twitch",
        },
        label: "Twitch".into(),
        value: connection_label(app.config.twitch.is_some(), app.twitch_connected),
        hint: Some("Enter: wizard"),
    });
    rows.push(SettingsRow {
        kind: K::Status {
            action: "wizard_youtube",
        },
        label: "YouTube".into(),
        value: connection_label(app.config.youtube.is_some(), app.youtube_connected),
        hint: Some("Enter: wizard"),
    });
    rows.push(SettingsRow {
        kind: K::Status {
            action: "wizard_patreon",
        },
        label: "Patreon".into(),
        value: connection_label(app.config.patreon.is_some(), app.patreon_connected),
        hint: Some("Enter: wizard"),
    });

    rows.push(h("Plugins"));
    if app.user_plugin_manifests.is_empty() {
        rows.push(SettingsRow {
            kind: K::Status {
                action: "plugin_dir_hint",
            },
            label: "User plugins".into(),
            value: "none discovered".into(),
            hint: Some("drop a TOML in ~/.config/strivo/plugins/"),
        });
    } else {
        for m in &app.user_plugin_manifests {
            let value = match (&m.version, &m.activation_key) {
                (Some(v), Some(k)) => format!("v{v} · {k}"),
                (Some(v), None) => format!("v{v}"),
                (None, Some(k)) => k.clone(),
                _ => "discovered".into(),
            };
            rows.push(SettingsRow {
                kind: K::Status {
                    action: "plugin_manifest",
                },
                label: m.name.clone(),
                value,
                hint: m
                    .description
                    .as_deref()
                    .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str),
            });
        }
    }

    rows.push(h("Maintenance"));
    rows.push(SettingsRow {
        kind: K::Status {
            action: "reset_defaults",
        },
        label: "Reset to defaults".into(),
        value: "preserves credentials".into(),
        hint: Some("Enter to confirm"),
    });

    rows
}

fn connection_label(configured: bool, connected: bool) -> String {
    match (configured, connected) {
        (true, true) => "connected".into(),
        (true, false) => "configured (not connected)".into(),
        (false, _) => "not configured".into(),
    }
}

/// Selectable-row indices into the full `settings_rows()` list. The
/// settings cursor (`AppState.settings_selected`) is an index into this
/// vector, not into the full list.
pub fn selectable_indices(rows: &[SettingsRow]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| !matches!(r.kind, SettingsKind::Header))
        .map(|(i, _)| i)
        .collect()
}

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let border_style = app.pane_border(&ActivePane::Settings);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(" Settings ")
        .title_style(Theme::title());

    let rows = settings_rows(app);
    let selectable = selectable_indices(&rows);
    let cursor = app
        .settings_selected
        .min(selectable.len().saturating_sub(1));
    let selected_full_idx = selectable.get(cursor).copied();

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| match row.kind {
            SettingsKind::Header => ListItem::new(Line::from(vec![
                Span::raw(""),
                Span::styled(
                    format!(" {}", row.label),
                    Style::new()
                        .fg(Theme::secondary())
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            _ => {
                let cursor_glyph = if Some(i) == selected_full_idx {
                    Span::styled(
                        " ▌ ",
                        Style::new()
                            .fg(Theme::primary())
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("   ")
                };
                let mut spans = vec![
                    cursor_glyph,
                    Span::styled(format!("{:<22}", row.label), Style::new().fg(Theme::blue())),
                    Span::raw(" "),
                    Span::styled(row.value.clone(), Style::new().fg(Theme::fg())),
                ];
                if let Some(hint) = row.hint {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(hint, Style::new().fg(Theme::muted())));
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();

    let mut state = ListState::default();
    if let Some(full_idx) = selected_full_idx {
        state.select(Some(full_idx));
    }

    let config_path_hint = Line::from(vec![
        Span::raw(" Config: "),
        Span::styled(
            crate::config::AppConfig::config_path()
                .to_string_lossy()
                .to_string(),
            Style::new().fg(Theme::muted()),
        ),
    ]);

    let list = List::new(items).block(block.title_bottom(config_path_hint));
    frame.render_stateful_widget(list, area, &mut state);
}
