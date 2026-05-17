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
    EnterStatusBar,
    EnterLogPane,
    EnterSchedulePane,
    SearchStart,
    /// Plugin-layer activation commands are still routed via the
    /// registry; this variant exists so the table can document them
    /// even though dispatch happens elsewhere.
    PluginActivate,
}

impl KeyAction {
    pub fn desc(&self) -> &'static str {
        match self {
            Self::Quit => "quit (confirm if recording)",
            Self::HelpToggle => "toggle help overlay",
            Self::HelpClose => "close help overlay",
            Self::ThemePickerOpen => "theme picker",
            Self::EventLogToggle => "event log",
            Self::EnterStatusBar => "status-bar focus",
            Self::EnterLogPane => "log pane",
            Self::EnterSchedulePane => "schedule pane",
            Self::SearchStart => "search filter",
            Self::PluginActivate => "plugin command",
        }
    }

    /// Parse an action name from the user remap file. Matches the
    /// variant identifier as written in code so the TOML stays close
    /// to the source. Unknown names return `None` and the loader logs
    /// a warning.
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "Quit" => Self::Quit,
            "HelpToggle" => Self::HelpToggle,
            "HelpClose" => Self::HelpClose,
            "ThemePickerOpen" => Self::ThemePickerOpen,
            "EventLogToggle" => Self::EventLogToggle,
            "EnterStatusBar" => Self::EnterStatusBar,
            "EnterLogPane" => Self::EnterLogPane,
            "EnterSchedulePane" => Self::EnterSchedulePane,
            "SearchStart" => Self::SearchStart,
            "PluginActivate" => Self::PluginActivate,
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
            return Some(Self { code, modifiers: mods });
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
            desc: self.desc.clone().unwrap_or_else(|| action.desc().to_string()),
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
        Chord { layer, key, action, desc }
    }
    // Global. Per-pane keys (j/k navigation, etc.) still live in
    // their handler match arms and will migrate in M3 follow-ups.
    static T: &[Chord] = &[
        c(Layer::Global, KeyPattern::plain(Char('q')),      KeyAction::Quit,                "quit"),
        c(Layer::Global, KeyPattern::plain(Char('?')),      KeyAction::HelpToggle,          "toggle help"),
        c(Layer::Global, KeyPattern::ctrl('t'),             KeyAction::ThemePickerOpen,     "theme picker"),
        c(Layer::Global, KeyPattern::ctrl('d'),             KeyAction::EnterStatusBar,      "diagnostics focus"),
        c(Layer::Global, KeyPattern { code: Char('E'), modifiers: M::SHIFT }, KeyAction::EventLogToggle, "event log"),
        c(Layer::Global, KeyPattern { code: Char('F'), modifiers: M::SHIFT }, KeyAction::EnterLogPane,   "log pane"),
        c(Layer::Global, KeyPattern { code: Char('S'), modifiers: M::SHIFT }, KeyAction::EnterSchedulePane, "schedule pane"),
        c(Layer::Global, KeyPattern::plain(Char('/')),      KeyAction::SearchStart,         "search filter"),
    ];
    T
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
        assert_eq!(lookup(Layer::Global, &with_shift), Some(KeyAction::EventLogToggle));
        assert_eq!(lookup(Layer::Global, &without_shift), Some(KeyAction::EventLogToggle));
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
