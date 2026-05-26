//! Centralized keymap table — single source of truth for keybindings.
//!
//! Yazi-inspired (see YAZI-AUDIT.md §2). Each `Chord` carries the key
//! pattern, the typed [`KeyAction`] to fire, and a static `desc` string.
//! The help overlay reads from this table so `?` always reflects reality
//! (M3.3); a future TOML overlay lets users remap (M3.4).
//!
//! M3 Phase 1 only migrates the **global** key layer in this commit —
//! pane handlers still own their own match arms. They consult
//! [`lookup`] first via [`maybe_global`] and only fall back to their
//! native match for layer-local keys not yet migrated.

use std::sync::OnceLock;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

use crate::app::ActivePane;

/// Which keymap layer a chord belongs to. Layer precedence follows
/// `overlay > plugin > pane > global`. Overlays own their keys
/// outright (the global pre-dispatch shortcircuits while they're up);
/// pane layers consult global last so a pane-specific key wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Always-on keys: quit, help, theme picker, etc.
    Global,
    Sidebar,
    Detail,
    RecordingList,
    Schedule,
    Settings,
    Log,
    Wizard,
    StatusBar,
    /// Overlay layers. These short-circuit other layers while open.
    ThemePicker,
    EventLog,
    PlaybackOverlay,
    SearchInput,
    QuitConfirm,
    PropertiesModal,
    PlatformDebugModal,
}

impl Layer {
    /// Map an `ActivePane` to the matching pane layer. Caller asks the
    /// table for the active layer, falling back to `Global` if no
    /// pane-specific entry matches.
    pub fn for_pane(pane: &ActivePane) -> Option<Self> {
        Some(match pane {
            ActivePane::Sidebar => Self::Sidebar,
            ActivePane::Detail => Self::Detail,
            ActivePane::RecordingList => Self::RecordingList,
            ActivePane::Schedule => Self::Schedule,
            ActivePane::Settings => Self::Settings,
            ActivePane::Log => Self::Log,
            ActivePane::Wizard => Self::Wizard,
            ActivePane::StatusBar => Self::StatusBar,
            ActivePane::Plugin(_) => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Sidebar => "sidebar",
            Self::Detail => "detail",
            Self::RecordingList => "recordings",
            Self::Schedule => "schedule",
            Self::Settings => "settings",
            Self::Log => "log",
            Self::Wizard => "wizard",
            Self::StatusBar => "statusbar",
            Self::ThemePicker => "themepicker",
            Self::EventLog => "eventlog",
            Self::PlaybackOverlay => "playback",
            Self::SearchInput => "search",
            Self::QuitConfirm => "quit?",
            Self::PropertiesModal => "props",
            Self::PlatformDebugModal => "platdebug",
        }
    }
}

/// Typed actions a key can request. Mirrored over to AppState which
/// applies them. New keys add a variant; the help overlay's third
/// column is the `desc` field of the corresponding [`Chord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Quit,
    HelpToggle,
    HelpClose,
    ThemePickerOpen,
    EventLogToggle,
    PluginBrowserToggle,
    EnterStatusBar,
    EnterLogPane,
    EnterSchedulePane,
    EnterSettings,
    EnterRecordingList,
    SearchStart,
    /// Plugin-layer activation commands are still routed via the
    /// registry; this variant exists so the table can document them
    /// even though dispatch happens elsewhere.
    PluginActivate,

    // Universal navigation (pane-context-sensitive)
    NavDown,
    NavUp,
    NavTop,
    NavBottom,
    NavBack,
    NavActivate,

    // Sidebar / Detail
    ToggleAutoRecord,
    ToggleBulkDownload,
    ToggleBulkDownloadPlatform,
    PickBulkPlaylist,

    // Detail
    StartRecording,
    StartRecordingFromStart,
    WatchStream,
    ToggleTranscode,

    // RecordingList
    StopRecording,
    PlayRecording,
    ShowRecordingProperties,
    ToggleRecordingSelect,
    ClearRecordingSelections,
    TrashSelectedRecordings,
    RenameRecording,
    MoveRecording,

    // Playback overlay
    PlaybackTogglePause,
    PlaybackSeekForward,
    PlaybackSeekBack,
    PlaybackSpeedUp,
    PlaybackSpeedDown,
    PlaybackVolumeUp,
    PlaybackVolumeDown,

    /// Toggle Visual mode (yazi audit §1). On the RecordingList this
    /// makes j/k extend the multi-selection.
    VisualModeToggle,
    /// Open the command palette (yazi audit §3). Typed names hit
    /// KeyAction::from_name and dispatch through apply_key_action.
    CommandPaletteOpen,
    /// Open the actions popup for the focused item (D5). The handler
    /// reads the selection set first and falls back to the cursor item.
    ActionsPopupOpen,
    /// Toggle the host DAG overlay (X6 + C1 phase 2). Shows every
    /// Pipeline plugins have submitted, with per-stage glyphs +
    /// retry counts + costs.
    DagOverlayToggle,
    /// Channel marks (yazi audit §11). MarkSetPrompt opens the modal
    /// to bind the current row to a char; MarkJumpPrompt opens it to
    /// jump.
    MarkSetPrompt,
    MarkJumpPrompt,
    /// Clipboard / open helpers (M4.6).
    CopyToClipboard,
    OpenInFolder,
    /// Undo the last destructive action (M4.follow.a). Cleared on quit;
    /// limited to 5 entries.
    UndoLast,
    /// Toggle the RecordingList between List and Grid view (M5.4).
    ToggleRecordingListView,

    // Schedule
    ScheduleAdd,
    ScheduleEditCron,
    ScheduleEditDuration,
    ScheduleDelete,

    // Log
    LogScrollPageDown,
    LogScrollPageUp,
    LogClear,
}

impl KeyAction {
    pub fn desc(&self) -> &'static str {
        match self {
            Self::Quit => "quit (confirm if recording)",
            Self::HelpToggle => "toggle help overlay",
            Self::HelpClose => "close help overlay",
            Self::ThemePickerOpen => "theme picker",
            Self::EventLogToggle => "event log",
            Self::PluginBrowserToggle => "plugins",
            Self::EnterStatusBar => "status-bar focus",
            Self::EnterLogPane => "log pane",
            Self::EnterSchedulePane => "schedule pane",
            Self::EnterSettings => "settings",
            Self::EnterRecordingList => "recordings",
            Self::SearchStart => "search filter",
            Self::PluginActivate => "plugin command",
            Self::NavDown => "next",
            Self::NavUp => "previous",
            Self::NavTop => "first",
            Self::NavBottom => "last",
            Self::NavBack => "back",
            Self::NavActivate => "open / activate",
            Self::ToggleAutoRecord => "toggle auto-record",
            Self::ToggleBulkDownload => "start/stop bulk download",
            Self::ToggleBulkDownloadPlatform => "start/stop bulk download (whole platform)",
            Self::PickBulkPlaylist => "bulk download a YouTube playlist",
            Self::StartRecording => "start recording",
            Self::StartRecordingFromStart => "record from start (YouTube)",
            Self::WatchStream => "watch in mpv",
            Self::ToggleTranscode => "toggle transcode mode",
            Self::StopRecording => "stop recording",
            Self::PlayRecording => "play recording",
            Self::ShowRecordingProperties => "recording properties",
            Self::ToggleRecordingSelect => "toggle multi-select",
            Self::ClearRecordingSelections => "clear multi-select",
            Self::TrashSelectedRecordings => "delete to trash",
            Self::RenameRecording => "rename recording",
            Self::MoveRecording => "move recording",
            Self::VisualModeToggle => "visual mode (multi-select)",
            Self::CommandPaletteOpen => "command palette",
            Self::ActionsPopupOpen => "actions popup",
            Self::DagOverlayToggle => "pipelines DAG",
            Self::MarkSetPrompt => "set mark",
            Self::MarkJumpPrompt => "jump to mark",
            Self::CopyToClipboard => "copy to clipboard",
            Self::OpenInFolder => "open folder",
            Self::UndoLast => "undo last destructive action",
            Self::ToggleRecordingListView => "toggle list / grid view",
            Self::PlaybackTogglePause => "play/pause",
            Self::PlaybackSeekForward => "seek +10s",
            Self::PlaybackSeekBack => "seek -10s",
            Self::PlaybackSpeedUp => "speed +0.25x",
            Self::PlaybackSpeedDown => "speed -0.25x",
            Self::PlaybackVolumeUp => "volume +5",
            Self::PlaybackVolumeDown => "volume -5",
            Self::ScheduleAdd => "add schedule",
            Self::ScheduleEditCron => "edit cron",
            Self::ScheduleEditDuration => "edit duration",
            Self::ScheduleDelete => "delete schedule",
            Self::LogScrollPageDown => "page down",
            Self::LogScrollPageUp => "page up",
            Self::LogClear => "clear log",
        }
    }

    /// Inverse of [`from_name`]. Returns the variant identifier as a
    /// `&'static str` for the command palette + future TOML serialization.
    /// Kept in lock-step with `from_name` so the round-trip is total
    /// (verified by the `name_roundtrip` test below).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::HelpToggle => "HelpToggle",
            Self::HelpClose => "HelpClose",
            Self::ThemePickerOpen => "ThemePickerOpen",
            Self::EventLogToggle => "EventLogToggle",
            Self::PluginBrowserToggle => "PluginBrowserToggle",
            Self::EnterStatusBar => "EnterStatusBar",
            Self::EnterLogPane => "EnterLogPane",
            Self::EnterSchedulePane => "EnterSchedulePane",
            Self::EnterSettings => "EnterSettings",
            Self::EnterRecordingList => "EnterRecordingList",
            Self::SearchStart => "SearchStart",
            Self::PluginActivate => "PluginActivate",
            Self::NavDown => "NavDown",
            Self::NavUp => "NavUp",
            Self::NavTop => "NavTop",
            Self::NavBottom => "NavBottom",
            Self::NavBack => "NavBack",
            Self::NavActivate => "NavActivate",
            Self::ToggleAutoRecord => "ToggleAutoRecord",
            Self::ToggleBulkDownload => "ToggleBulkDownload",
            Self::ToggleBulkDownloadPlatform => "ToggleBulkDownloadPlatform",
            Self::PickBulkPlaylist => "PickBulkPlaylist",
            Self::StartRecording => "StartRecording",
            Self::StartRecordingFromStart => "StartRecordingFromStart",
            Self::WatchStream => "WatchStream",
            Self::ToggleTranscode => "ToggleTranscode",
            Self::StopRecording => "StopRecording",
            Self::PlayRecording => "PlayRecording",
            Self::ShowRecordingProperties => "ShowRecordingProperties",
            Self::ToggleRecordingSelect => "ToggleRecordingSelect",
            Self::ClearRecordingSelections => "ClearRecordingSelections",
            Self::TrashSelectedRecordings => "TrashSelectedRecordings",
            Self::RenameRecording => "RenameRecording",
            Self::MoveRecording => "MoveRecording",
            Self::VisualModeToggle => "VisualModeToggle",
            Self::CommandPaletteOpen => "CommandPaletteOpen",
            Self::ActionsPopupOpen => "ActionsPopupOpen",
            Self::DagOverlayToggle => "DagOverlayToggle",
            Self::MarkSetPrompt => "MarkSetPrompt",
            Self::MarkJumpPrompt => "MarkJumpPrompt",
            Self::CopyToClipboard => "CopyToClipboard",
            Self::OpenInFolder => "OpenInFolder",
            Self::UndoLast => "UndoLast",
            Self::ToggleRecordingListView => "ToggleRecordingListView",
            Self::PlaybackTogglePause => "PlaybackTogglePause",
            Self::PlaybackSeekForward => "PlaybackSeekForward",
            Self::PlaybackSeekBack => "PlaybackSeekBack",
            Self::PlaybackSpeedUp => "PlaybackSpeedUp",
            Self::PlaybackSpeedDown => "PlaybackSpeedDown",
            Self::PlaybackVolumeUp => "PlaybackVolumeUp",
            Self::PlaybackVolumeDown => "PlaybackVolumeDown",
            Self::ScheduleAdd => "ScheduleAdd",
            Self::ScheduleEditCron => "ScheduleEditCron",
            Self::ScheduleEditDuration => "ScheduleEditDuration",
            Self::ScheduleDelete => "ScheduleDelete",
            Self::LogScrollPageDown => "LogScrollPageDown",
            Self::LogScrollPageUp => "LogScrollPageUp",
            Self::LogClear => "LogClear",
        }
    }

    /// Parse an action name from the user remap file. Matches the
    /// variant identifier as written in code so the TOML stays close
    /// to the source. Unknown names return `None` and the loader logs
    /// a warning.
    pub fn from_name(s: &str) -> Option<Self> {
        // Variant name -> enum. New variants must be added here so the
        // user remap TOML can reach them; tests verify the roundtrip.
        Some(match s {
            "Quit" => Self::Quit,
            "HelpToggle" => Self::HelpToggle,
            "HelpClose" => Self::HelpClose,
            "ThemePickerOpen" => Self::ThemePickerOpen,
            "EventLogToggle" => Self::EventLogToggle,
            "PluginBrowserToggle" => Self::PluginBrowserToggle,
            "EnterStatusBar" => Self::EnterStatusBar,
            "EnterLogPane" => Self::EnterLogPane,
            "EnterSchedulePane" => Self::EnterSchedulePane,
            "EnterSettings" => Self::EnterSettings,
            "EnterRecordingList" => Self::EnterRecordingList,
            "SearchStart" => Self::SearchStart,
            "PluginActivate" => Self::PluginActivate,
            "NavDown" => Self::NavDown,
            "NavUp" => Self::NavUp,
            "NavTop" => Self::NavTop,
            "NavBottom" => Self::NavBottom,
            "NavBack" => Self::NavBack,
            "NavActivate" => Self::NavActivate,
            "ToggleAutoRecord" => Self::ToggleAutoRecord,
            "ToggleBulkDownload" => Self::ToggleBulkDownload,
            "ToggleBulkDownloadPlatform" => Self::ToggleBulkDownloadPlatform,
            "PickBulkPlaylist" => Self::PickBulkPlaylist,
            "StartRecording" => Self::StartRecording,
            "StartRecordingFromStart" => Self::StartRecordingFromStart,
            "WatchStream" => Self::WatchStream,
            "ToggleTranscode" => Self::ToggleTranscode,
            "StopRecording" => Self::StopRecording,
            "PlayRecording" => Self::PlayRecording,
            "ShowRecordingProperties" => Self::ShowRecordingProperties,
            "ToggleRecordingSelect" => Self::ToggleRecordingSelect,
            "ClearRecordingSelections" => Self::ClearRecordingSelections,
            "TrashSelectedRecordings" => Self::TrashSelectedRecordings,
            "RenameRecording" => Self::RenameRecording,
            "MoveRecording" => Self::MoveRecording,
            "VisualModeToggle" => Self::VisualModeToggle,
            "CommandPaletteOpen" => Self::CommandPaletteOpen,
            "ActionsPopupOpen" => Self::ActionsPopupOpen,
            "DagOverlayToggle" => Self::DagOverlayToggle,
            "MarkSetPrompt" => Self::MarkSetPrompt,
            "MarkJumpPrompt" => Self::MarkJumpPrompt,
            "CopyToClipboard" => Self::CopyToClipboard,
            "OpenInFolder" => Self::OpenInFolder,
            "UndoLast" => Self::UndoLast,
            "ToggleRecordingListView" => Self::ToggleRecordingListView,
            "PlaybackTogglePause" => Self::PlaybackTogglePause,
            "PlaybackSeekForward" => Self::PlaybackSeekForward,
            "PlaybackSeekBack" => Self::PlaybackSeekBack,
            "PlaybackSpeedUp" => Self::PlaybackSpeedUp,
            "PlaybackSpeedDown" => Self::PlaybackSpeedDown,
            "PlaybackVolumeUp" => Self::PlaybackVolumeUp,
            "PlaybackVolumeDown" => Self::PlaybackVolumeDown,
            "ScheduleAdd" => Self::ScheduleAdd,
            "ScheduleEditCron" => Self::ScheduleEditCron,
            "ScheduleEditDuration" => Self::ScheduleEditDuration,
            "ScheduleDelete" => Self::ScheduleDelete,
            "LogScrollPageDown" => Self::LogScrollPageDown,
            "LogScrollPageUp" => Self::LogScrollPageUp,
            "LogClear" => Self::LogClear,
            _ => return None,
        })
    }
}

/// One row in the binding table. `on` matches a `crossterm::KeyEvent`;
/// `desc` is the help-overlay third column.
#[derive(Debug, Clone, Copy)]
pub struct Chord {
    pub layer: Layer,
    pub key: KeyPattern,
    pub action: KeyAction,
    pub desc: &'static str,
}

/// What to match against a key event. Kept simple for now — single
/// key + modifier flags. Multi-key prefixes (yazi-style `gg`) can be
/// added later by extending this enum.
#[derive(Debug, Clone, Copy)]
pub struct KeyPattern {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyPattern {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub const fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    pub const fn shift_char(c: char) -> Self {
        // crossterm sets SHIFT for uppercase chars on most platforms;
        // the actual KeyCode::Char is the uppercase form. We match
        // either form by emitting both flag combinations in `matches`.
        Self {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::SHIFT,
        }
    }

    /// Parse the yazi-style `<C-s>` / `<S-Tab>` / single-char form
    /// the user types into `keybindings.toml`.
    pub fn parse(spec: &str) -> Option<Self> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(inner) = trimmed.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
            let mut mods = KeyModifiers::NONE;
            let mut rest = inner;
            // Parse modifier prefixes greedily: C- / A- / S- / M- (Super)
            loop {
                if let Some(r) = rest.strip_prefix("C-") {
                    mods |= KeyModifiers::CONTROL;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix("A-") {
                    mods |= KeyModifiers::ALT;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix("S-") {
                    mods |= KeyModifiers::SHIFT;
                    rest = r;
                } else {
                    break;
                }
            }
            let code = match rest {
                "Tab" => KeyCode::Tab,
                "Enter" => KeyCode::Enter,
                "Esc" => KeyCode::Esc,
                "Space" => KeyCode::Char(' '),
                "Up" => KeyCode::Up,
                "Down" => KeyCode::Down,
                "Left" => KeyCode::Left,
                "Right" => KeyCode::Right,
                "Home" => KeyCode::Home,
                "End" => KeyCode::End,
                "PageUp" => KeyCode::PageUp,
                "PageDown" => KeyCode::PageDown,
                "Backspace" => KeyCode::Backspace,
                "Delete" => KeyCode::Delete,
                s if s.starts_with('F') => {
                    let n: u8 = s[1..].parse().ok()?;
                    KeyCode::F(n)
                }
                s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
                _ => return None,
            };
            return Some(Self {
                code,
                modifiers: mods,
            });
        }
        if trimmed.chars().count() == 1 {
            return Some(Self::plain(KeyCode::Char(trimmed.chars().next().unwrap())));
        }
        None
    }

    pub fn matches(&self, ev: &KeyEvent) -> bool {
        if self.code != ev.code {
            return false;
        }
        // Crossterm reports SHIFT inconsistently for character keys
        // (Unix vs Windows). Treat the SHIFT bit as a soft match for
        // KeyCode::Char so platform drift doesn't break bindings.
        if matches!(self.code, KeyCode::Char(_)) {
            // Required modifiers minus SHIFT must be a subset of the
            // event modifiers; SHIFT is allowed to differ.
            let want = self.modifiers - KeyModifiers::SHIFT;
            let have = ev.modifiers - KeyModifiers::SHIFT;
            return want == have;
        }
        self.modifiers == ev.modifiers
    }
}

/// On-disk schema for `~/.config/strivo/keybindings.toml`. Mirrors yazi's
/// three-bucket model: `prepend_keymap` rows are matched before the
/// base table, `append_keymap` rows after. `keymap` replaces the base
/// for a layer (rare; for full takeovers).
#[derive(Debug, Default, Deserialize)]
pub struct RemapFile {
    #[serde(default)]
    pub prepend_keymap: Vec<RemapRow>,
    #[serde(default)]
    pub append_keymap: Vec<RemapRow>,
}

#[derive(Debug, Deserialize)]
pub struct RemapRow {
    pub layer: Option<String>,
    pub on: String,
    pub action: String,
    #[serde(default)]
    pub desc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedChord {
    pub layer: Layer,
    pub key: KeyPattern,
    pub action: KeyAction,
    pub desc: String,
}

impl RemapRow {
    pub fn parse(&self) -> Option<ParsedChord> {
        let layer = match self.layer.as_deref().unwrap_or("Global") {
            "Global" => Layer::Global,
            "Sidebar" => Layer::Sidebar,
            "Detail" => Layer::Detail,
            "RecordingList" => Layer::RecordingList,
            "Schedule" => Layer::Schedule,
            "Settings" => Layer::Settings,
            "Log" => Layer::Log,
            "Wizard" => Layer::Wizard,
            "StatusBar" => Layer::StatusBar,
            _ => return None,
        };
        let key = KeyPattern::parse(&self.on)?;
        let action = KeyAction::from_name(&self.action)?;
        Some(ParsedChord {
            layer,
            key,
            action,
            desc: self
                .desc
                .clone()
                .unwrap_or_else(|| action.desc().to_string()),
        })
    }
}

/// Loaded user overlay. Lookup checks `prepend` first, then the base
/// table, then `append`. Initialized at startup via [`load_remap`].
#[derive(Debug, Default)]
pub struct RemapOverlay {
    pub prepend: Vec<ParsedChord>,
    pub append: Vec<ParsedChord>,
}

static OVERLAY: OnceLock<RemapOverlay> = OnceLock::new();

/// Read `~/.config/strivo/keybindings.toml` (if present) into the
/// process-wide overlay. Idempotent: only the first call has effect.
/// Bad rows are logged and skipped — the base table still works.
pub fn load_remap() {
    if OVERLAY.get().is_some() {
        return;
    }
    let path = crate::config::AppConfig::config_dir().join("keybindings.toml");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        let _ = OVERLAY.set(RemapOverlay::default());
        return;
    };
    let parsed: Result<RemapFile, _> = toml::from_str(&contents);
    let overlay = match parsed {
        Ok(f) => {
            let prepend: Vec<ParsedChord> = f
                .prepend_keymap
                .iter()
                .filter_map(|r| {
                    let p = r.parse();
                    if p.is_none() {
                        tracing::warn!(
                            row = ?r,
                            "keybindings.toml: skipping unparseable prepend row"
                        );
                    }
                    p
                })
                .collect();
            let append: Vec<ParsedChord> = f
                .append_keymap
                .iter()
                .filter_map(|r| {
                    let p = r.parse();
                    if p.is_none() {
                        tracing::warn!(
                            row = ?r,
                            "keybindings.toml: skipping unparseable append row"
                        );
                    }
                    p
                })
                .collect();
            tracing::info!(
                prepend = prepend.len(),
                append = append.len(),
                "keybindings.toml loaded"
            );
            RemapOverlay { prepend, append }
        }
        Err(e) => {
            tracing::warn!("keybindings.toml parse failed: {e} — using base table");
            RemapOverlay::default()
        }
    };
    let _ = OVERLAY.set(overlay);
}

fn overlay() -> &'static RemapOverlay {
    OVERLAY.get_or_init(RemapOverlay::default)
}

/// The global keymap. New rows go here; per-layer lookup walks this
/// vector once. Layer precedence is enforced by [`lookup`], which
/// searches active-layer entries before falling back to `Global`.
fn table() -> &'static [Chord] {
    use KeyCode::*;
    use KeyModifiers as M;
    const fn c(layer: Layer, key: KeyPattern, action: KeyAction, desc: &'static str) -> Chord {
        Chord {
            layer,
            key,
            action,
            desc,
        }
    }
    // Global. Per-pane keys (j/k navigation, etc.) still live in
    // their handler match arms and will migrate in M3 follow-ups.
    const fn nav_rows(layer: Layer) -> [Chord; 12] {
        [
            c(
                layer,
                KeyPattern::plain(Char('j')),
                KeyAction::NavDown,
                "next",
            ),
            c(layer, KeyPattern::plain(Down), KeyAction::NavDown, "next"),
            c(
                layer,
                KeyPattern::plain(Char('k')),
                KeyAction::NavUp,
                "previous",
            ),
            c(layer, KeyPattern::plain(Up), KeyAction::NavUp, "previous"),
            c(
                layer,
                KeyPattern::plain(Char('g')),
                KeyAction::NavTop,
                "first",
            ),
            c(layer, KeyPattern::plain(Home), KeyAction::NavTop, "first"),
            c(
                layer,
                KeyPattern {
                    code: Char('G'),
                    modifiers: M::SHIFT,
                },
                KeyAction::NavBottom,
                "last",
            ),
            c(layer, KeyPattern::plain(End), KeyAction::NavBottom, "last"),
            c(
                layer,
                KeyPattern::plain(Char('h')),
                KeyAction::NavBack,
                "back",
            ),
            c(layer, KeyPattern::plain(Left), KeyAction::NavBack, "back"),
            c(layer, KeyPattern::plain(Esc), KeyAction::NavBack, "back"),
            c(
                layer,
                KeyPattern::plain(Enter),
                KeyAction::NavActivate,
                "open / activate",
            ),
        ]
    }

    // Per-pane rows are stored as separate static slices so they can be
    // const-initialized. Walking nested slices keeps lookup O(table_size)
    // but the table is tiny.
    static GLOBAL: &[Chord] = &[
        c(
            Layer::Global,
            KeyPattern::plain(Char('q')),
            KeyAction::Quit,
            "quit",
        ),
        c(
            Layer::Global,
            KeyPattern::plain(Char('?')),
            KeyAction::HelpToggle,
            "toggle help",
        ),
        c(
            Layer::Global,
            KeyPattern::ctrl('t'),
            KeyAction::ThemePickerOpen,
            "theme picker",
        ),
        c(
            Layer::Global,
            KeyPattern::ctrl('d'),
            KeyAction::EnterStatusBar,
            "diagnostics focus",
        ),
        c(
            Layer::Global,
            KeyPattern {
                code: Char('E'),
                modifiers: M::SHIFT,
            },
            KeyAction::EventLogToggle,
            "event log",
        ),
        c(
            Layer::Global,
            KeyPattern {
                code: Char('P'),
                modifiers: M::SHIFT,
            },
            KeyAction::PluginBrowserToggle,
            "plugins",
        ),
        c(
            Layer::Global,
            KeyPattern::ctrl('g'),
            KeyAction::DagOverlayToggle,
            "pipelines DAG",
        ),
        c(
            Layer::Global,
            KeyPattern {
                code: Char('F'),
                modifiers: M::SHIFT,
            },
            KeyAction::EnterLogPane,
            "log pane",
        ),
        c(
            Layer::Global,
            KeyPattern {
                code: Char('S'),
                modifiers: M::SHIFT,
            },
            KeyAction::EnterSchedulePane,
            "schedule pane",
        ),
        c(
            Layer::Global,
            KeyPattern::plain(Char('/')),
            KeyAction::SearchStart,
            "search filter",
        ),
        c(
            Layer::Global,
            KeyPattern::plain(Char(':')),
            KeyAction::CommandPaletteOpen,
            "command palette",
        ),
        c(
            Layer::Global,
            KeyPattern::plain(Char('u')),
            KeyAction::UndoLast,
            "undo last",
        ),
    ];

    static SIDEBAR_NAV: [Chord; 12] = nav_rows(Layer::Sidebar);
    static SIDEBAR_LOCAL: &[Chord] = &[
        c(
            Layer::Sidebar,
            KeyPattern::plain(Char('l')),
            KeyAction::NavActivate,
            "open detail",
        ),
        c(
            Layer::Sidebar,
            KeyPattern::plain(Right),
            KeyAction::NavActivate,
            "open detail",
        ),
        c(
            Layer::Sidebar,
            KeyPattern {
                code: Char('L'),
                modifiers: M::SHIFT,
            },
            KeyAction::EnterRecordingList,
            "recordings",
        ),
        c(
            Layer::Sidebar,
            KeyPattern::plain(Char('s')),
            KeyAction::EnterSettings,
            "settings",
        ),
        c(
            Layer::Sidebar,
            KeyPattern {
                code: Char('C'),
                modifiers: M::SHIFT,
            },
            KeyAction::EnterSettings,
            "settings",
        ),
        c(
            Layer::Sidebar,
            KeyPattern::plain(Char('a')),
            KeyAction::ToggleAutoRecord,
            "toggle auto-record",
        ),
        c(
            Layer::Sidebar,
            KeyPattern::plain(Char('b')),
            KeyAction::ToggleBulkDownload,
            "start/stop bulk download",
        ),
        c(
            Layer::Sidebar,
            KeyPattern {
                code: Char('B'),
                modifiers: M::SHIFT,
            },
            KeyAction::ToggleBulkDownloadPlatform,
            "bulk download whole platform",
        ),
        c(
            Layer::Sidebar,
            KeyPattern::plain(Char('m')),
            KeyAction::MarkSetPrompt,
            "set mark on current",
        ),
        c(
            Layer::Sidebar,
            KeyPattern::plain(Char('\'')),
            KeyAction::MarkJumpPrompt,
            "jump to mark",
        ),
    ];

    static DETAIL_NAV: [Chord; 12] = nav_rows(Layer::Detail);
    static DETAIL_LOCAL: &[Chord] = &[
        c(
            Layer::Detail,
            KeyPattern::plain(Char('r')),
            KeyAction::StartRecording,
            "start recording",
        ),
        c(
            Layer::Detail,
            KeyPattern {
                code: Char('R'),
                modifiers: M::SHIFT,
            },
            KeyAction::StartRecordingFromStart,
            "record from start (YT)",
        ),
        c(
            Layer::Detail,
            KeyPattern::plain(Char('w')),
            KeyAction::WatchStream,
            "watch in mpv",
        ),
        c(
            Layer::Detail,
            KeyPattern::plain(Char('a')),
            KeyAction::ToggleAutoRecord,
            "toggle auto-record",
        ),
        c(
            Layer::Detail,
            KeyPattern::plain(Char('b')),
            KeyAction::ToggleBulkDownload,
            "start/stop bulk download",
        ),
        c(
            Layer::Detail,
            KeyPattern {
                code: Char('B'),
                modifiers: M::SHIFT,
            },
            KeyAction::ToggleBulkDownloadPlatform,
            "bulk download whole platform",
        ),
        c(
            Layer::Detail,
            KeyPattern {
                code: Char('P'),
                modifiers: M::SHIFT,
            },
            KeyAction::PickBulkPlaylist,
            "bulk download a YouTube playlist",
        ),
        c(
            Layer::Detail,
            KeyPattern::plain(Char('t')),
            KeyAction::ToggleTranscode,
            "toggle transcode mode",
        ),
        c(
            Layer::Detail,
            KeyPattern::plain(Char('y')),
            KeyAction::CopyToClipboard,
            "copy channel URL",
        ),
    ];

    static REC_NAV: [Chord; 12] = nav_rows(Layer::RecordingList);
    static REC_LOCAL: &[Chord] = &[
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('s')),
            KeyAction::StopRecording,
            "stop recording",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('p')),
            KeyAction::PlayRecording,
            "play",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('i')),
            KeyAction::ShowRecordingProperties,
            "properties",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('v')),
            KeyAction::ToggleRecordingSelect,
            "toggle select",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('a')),
            KeyAction::ActionsPopupOpen,
            "actions popup",
        ),
        c(
            Layer::RecordingList,
            KeyPattern {
                code: Char('V'),
                modifiers: M::SHIFT,
            },
            KeyAction::VisualModeToggle,
            "visual mode",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::ctrl('v'),
            KeyAction::ClearRecordingSelections,
            "clear selections",
        ),
        c(
            Layer::RecordingList,
            KeyPattern {
                code: Char('D'),
                modifiers: M::SHIFT,
            },
            KeyAction::TrashSelectedRecordings,
            "delete to trash",
        ),
        c(
            Layer::RecordingList,
            KeyPattern {
                code: Char('R'),
                modifiers: M::SHIFT,
            },
            KeyAction::RenameRecording,
            "rename",
        ),
        c(
            Layer::RecordingList,
            KeyPattern {
                code: Char('M'),
                modifiers: M::SHIFT,
            },
            KeyAction::MoveRecording,
            "move",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('y')),
            KeyAction::CopyToClipboard,
            "copy path",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Tab),
            KeyAction::ToggleRecordingListView,
            "toggle list/grid",
        ),
        c(
            Layer::RecordingList,
            KeyPattern {
                code: Char('O'),
                modifiers: M::SHIFT,
            },
            KeyAction::OpenInFolder,
            "open folder",
        ),
        // Playback overlay keys (active only while playback.is_some()).
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char(' ')),
            KeyAction::PlaybackTogglePause,
            "play/pause",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char(']')),
            KeyAction::PlaybackSeekForward,
            "seek +10s",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('[')),
            KeyAction::PlaybackSeekBack,
            "seek -10s",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('.')),
            KeyAction::PlaybackSpeedUp,
            "speed +0.25x",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char(',')),
            KeyAction::PlaybackSpeedDown,
            "speed -0.25x",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('+')),
            KeyAction::PlaybackVolumeUp,
            "volume +5",
        ),
        c(
            Layer::RecordingList,
            KeyPattern::plain(Char('-')),
            KeyAction::PlaybackVolumeDown,
            "volume -5",
        ),
    ];

    static SCHEDULE_NAV: [Chord; 12] = nav_rows(Layer::Schedule);
    static SCHEDULE_LOCAL: &[Chord] = &[
        c(
            Layer::Schedule,
            KeyPattern::plain(Char('a')),
            KeyAction::ScheduleAdd,
            "add schedule",
        ),
        c(
            Layer::Schedule,
            KeyPattern::plain(Char('e')),
            KeyAction::ScheduleEditCron,
            "edit cron",
        ),
        c(
            Layer::Schedule,
            KeyPattern::plain(Char('d')),
            KeyAction::ScheduleEditDuration,
            "edit duration",
        ),
        c(
            Layer::Schedule,
            KeyPattern {
                code: Char('D'),
                modifiers: M::SHIFT,
            },
            KeyAction::ScheduleDelete,
            "delete row",
        ),
    ];

    static LOG_NAV: [Chord; 12] = nav_rows(Layer::Log);
    static LOG_LOCAL: &[Chord] = &[
        c(
            Layer::Log,
            KeyPattern::plain(PageDown),
            KeyAction::LogScrollPageDown,
            "page down",
        ),
        c(
            Layer::Log,
            KeyPattern::plain(PageUp),
            KeyAction::LogScrollPageUp,
            "page up",
        ),
        c(
            Layer::Log,
            KeyPattern::plain(Char('c')),
            KeyAction::LogClear,
            "clear log",
        ),
    ];

    static SETTINGS_NAV: [Chord; 12] = nav_rows(Layer::Settings);

    // Concatenated table — assembled lazily on first access. The slices
    // above are const, so the runtime cost is one Vec::extend per slice.
    static FULL: OnceLock<Vec<Chord>> = OnceLock::new();
    FULL.get_or_init(|| {
        let mut all: Vec<Chord> = Vec::new();
        all.extend_from_slice(GLOBAL);
        all.extend_from_slice(&SIDEBAR_NAV);
        all.extend_from_slice(SIDEBAR_LOCAL);
        all.extend_from_slice(&DETAIL_NAV);
        all.extend_from_slice(DETAIL_LOCAL);
        all.extend_from_slice(&REC_NAV);
        all.extend_from_slice(REC_LOCAL);
        all.extend_from_slice(&SCHEDULE_NAV);
        all.extend_from_slice(SCHEDULE_LOCAL);
        all.extend_from_slice(&LOG_NAV);
        all.extend_from_slice(LOG_LOCAL);
        all.extend_from_slice(&SETTINGS_NAV);
        all
    })
    .as_slice()
}

/// Look up a `KeyAction` for `key` in `layer`. Layer order:
/// 1. User `prepend_keymap` rows for this layer.
/// 2. Base table rows for this layer.
/// 3. User `prepend_keymap` rows for `Global` (when `layer != Global`).
/// 4. Base table rows for `Global` (same).
/// 5. User `append_keymap` rows for this layer, then `Global`.
pub fn lookup(layer: Layer, key: &KeyEvent) -> Option<KeyAction> {
    let overlay = overlay();
    // Prepend (user-supplied wins over base) — current layer.
    if let Some(c) = overlay
        .prepend
        .iter()
        .find(|c| c.layer == layer && c.key.matches(key))
    {
        return Some(c.action);
    }
    // Base table — current layer.
    let t = table();
    if let Some(chord) = t.iter().find(|c| c.layer == layer && c.key.matches(key)) {
        return Some(chord.action);
    }
    // Global fallback.
    if layer != Layer::Global {
        if let Some(c) = overlay
            .prepend
            .iter()
            .find(|c| c.layer == Layer::Global && c.key.matches(key))
        {
            return Some(c.action);
        }
        if let Some(chord) = t
            .iter()
            .find(|c| c.layer == Layer::Global && c.key.matches(key))
        {
            return Some(chord.action);
        }
    }
    // Append (last-chance user-supplied).
    if let Some(c) = overlay
        .append
        .iter()
        .find(|c| c.layer == layer && c.key.matches(key))
    {
        return Some(c.action);
    }
    if layer != Layer::Global {
        if let Some(c) = overlay
            .append
            .iter()
            .find(|c| c.layer == Layer::Global && c.key.matches(key))
        {
            return Some(c.action);
        }
    }
    None
}

/// Iterator over chords in a given layer plus the global layer. Used
/// by the auto-generated help overlay (M3.3).
pub fn chords_for(layer: Layer) -> Vec<&'static Chord> {
    let t = table();
    t.iter()
        .filter(|c| c.layer == layer || c.layer == Layer::Global)
        .collect()
}

/// All chords (every layer). Used by the conflict-detection assert
/// and any "show me everything" rendering paths.
pub fn all_chords() -> &'static [Chord] {
    table()
}

/// Sanity check at startup: assert no two chords in the same layer
/// share the same `(code, modifiers)`. Called from `AppState::new` so
/// any duplicate is a panic on first run rather than silent shadowing.
pub fn assert_no_conflicts() {
    let t = table();
    let mut seen: Vec<(Layer, KeyCode, KeyModifiers)> = Vec::new();
    for chord in t {
        let key = (chord.layer, chord.key.code, chord.key.modifiers);
        if seen.contains(&key) {
            panic!(
                "keymap conflict in layer {:?}: {:?} (mods {:?}) bound twice",
                chord.layer, chord.key.code, chord.key.modifiers
            );
        }
        seen.push(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_roundtrip_through_from_name() {
        // Every chord's action must round-trip through name → from_name.
        // Guards against drift between the two tables when new variants
        // get added in one without the other.
        for chord in all_chords() {
            let n = chord.action.name();
            assert_eq!(
                KeyAction::from_name(n),
                Some(chord.action),
                "from_name({n:?}) did not yield {:?}",
                chord.action
            );
        }
    }

    #[test]
    fn lookup_global_quit() {
        let ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(lookup(Layer::Sidebar, &ev), Some(KeyAction::Quit));
    }

    #[test]
    fn ctrl_t_opens_theme_picker() {
        let ev = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Layer::Global, &ev), Some(KeyAction::ThemePickerOpen));
    }

    #[test]
    fn shift_e_opens_event_log_regardless_of_shift_drift() {
        // Some platforms send SHIFT for uppercase char; others don't.
        let with_shift = KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT);
        let without_shift = KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE);
        assert_eq!(
            lookup(Layer::Global, &with_shift),
            Some(KeyAction::EventLogToggle)
        );
        assert_eq!(
            lookup(Layer::Global, &without_shift),
            Some(KeyAction::EventLogToggle)
        );
    }

    #[test]
    fn no_conflicts_in_table() {
        assert_no_conflicts();
    }

    #[test]
    fn parse_yazi_style_chords() {
        let p = KeyPattern::parse("<C-s>").unwrap();
        assert_eq!(p.code, KeyCode::Char('s'));
        assert!(p.modifiers.contains(KeyModifiers::CONTROL));

        let p = KeyPattern::parse("<S-Tab>").unwrap();
        assert_eq!(p.code, KeyCode::Tab);
        assert!(p.modifiers.contains(KeyModifiers::SHIFT));

        let p = KeyPattern::parse("q").unwrap();
        assert_eq!(p.code, KeyCode::Char('q'));
        assert!(p.modifiers.is_empty());

        assert!(KeyPattern::parse("").is_none());
        assert!(KeyPattern::parse("<not-a-key>").is_none());
    }

    #[test]
    fn action_name_roundtrip() {
        for action in [
            KeyAction::Quit,
            KeyAction::ThemePickerOpen,
            KeyAction::EventLogToggle,
        ] {
            let name = format!("{action:?}");
            // Variant names are stable identifiers — same as from_name().
            assert_eq!(KeyAction::from_name(&name), Some(action));
        }
    }
}
