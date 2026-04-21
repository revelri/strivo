pub mod kitty_import;

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

/// Global theme storage — initialized once, switchable at runtime.
static THEME: OnceLock<RwLock<ThemeData>> = OnceLock::new();
/// Generation counter — incremented on every theme switch.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// Per-frame cache — avoids 100+ RwLock reads + ThemeData clones per render.
thread_local! {
    static CACHED: RefCell<(u64, ThemeData)> = RefCell::new((u64::MAX, ThemeData::neon()));
}

/// Platform colors — fixed, NOT themeable per DESIGN.md.
pub const TWITCH_COLOR: Color = Color::Rgb(145, 70, 255);
pub const YOUTUBE_COLOR: Color = Color::Rgb(255, 0, 0);
pub const PATREON_COLOR: Color = Color::Rgb(255, 66, 77);

/// 16 semantic color slots per DESIGN.md's Ghostty-style theming spec.
///
/// Slots: bg, fg, surface, overlay, primary, secondary, dim, muted,
///        + 8 ANSI colors (black, red, green, yellow, blue, magenta, cyan, white).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeData {
    pub name: String,

    /// Optional author attribution (surfaced in the picker's preview pane).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Optional one-line description (e.g. "cool pastels, balanced contrast").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    // Base surfaces
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub bg: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub fg: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub surface: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub overlay: Color,

    // Semantic accents
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub primary: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub secondary: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub dim: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub muted: Color,

    // ANSI color slots
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_black: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_red: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_green: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_yellow: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_blue: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_magenta: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_cyan: Color,
    #[serde(deserialize_with = "de_color", serialize_with = "ser_color")]
    pub ansi_white: Color,
}

fn ser_color<S: serde::Serializer>(color: &Color, s: S) -> Result<S::Ok, S::Error> {
    match color {
        Color::Rgb(r, g, b) => s.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}")),
        _ => s.serialize_str("#000000"),
    }
}

fn de_color<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
    let s = String::deserialize(d)?;
    parse_hex_color(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid hex color: {s}")))
}

pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

// ── Built-in themes (per DESIGN.md) ─────────────────────────────────────

impl ThemeData {
    /// Neon (default) — Cyan + Amber on deep blue-black. The signature StriVo look.
    pub fn neon() -> Self {
        Self {
            name: "neon".to_string(),
            author: Some("StriVo".into()),
            description: Some("Cyan + amber on deep blue-black — the signature StriVo look".into()),
            bg: Color::Rgb(26, 27, 38),         // #1A1B26
            fg: Color::Rgb(232, 232, 226),       // #E8E8E2
            surface: Color::Rgb(36, 37, 58),     // #24253A
            overlay: Color::Rgb(59, 61, 86),     // #3B3D56
            primary: Color::Rgb(0, 229, 255),    // #00E5FF (cyan)
            secondary: Color::Rgb(255, 176, 32), // #FFB020 (amber)
            dim: Color::Rgb(86, 91, 126),        // #565B7E
            muted: Color::Rgb(169, 174, 207),    // #A9AECF
            ansi_black: Color::Rgb(26, 27, 38),
            ansi_red: Color::Rgb(255, 68, 68),   // #FF4444
            ansi_green: Color::Rgb(57, 255, 127), // #39FF7F
            ansi_yellow: Color::Rgb(255, 176, 32),
            ansi_blue: Color::Rgb(0, 180, 216),  // #00B4D8
            ansi_magenta: Color::Rgb(255, 121, 198), // #FF79C6
            ansi_cyan: Color::Rgb(0, 229, 255),
            ansi_white: Color::Rgb(232, 232, 226),
        }
    }

    /// Monochrome — Grayscale with red/green semantic colors only.
    pub fn monochrome() -> Self {
        Self {
            name: "monochrome".to_string(),
            author: Some("StriVo".into()),
            description: Some("Grayscale with red/green semantic accents only".into()),
            bg: Color::Rgb(24, 24, 24),
            fg: Color::Rgb(220, 220, 220),
            surface: Color::Rgb(36, 36, 36),
            overlay: Color::Rgb(56, 56, 56),
            primary: Color::Rgb(200, 200, 200),   // white-ish accent
            secondary: Color::Rgb(160, 160, 160),
            dim: Color::Rgb(80, 80, 80),
            muted: Color::Rgb(140, 140, 140),
            ansi_black: Color::Rgb(24, 24, 24),
            ansi_red: Color::Rgb(220, 80, 80),
            ansi_green: Color::Rgb(80, 200, 80),
            ansi_yellow: Color::Rgb(200, 200, 80),
            ansi_blue: Color::Rgb(120, 120, 200),
            ansi_magenta: Color::Rgb(180, 120, 180),
            ansi_cyan: Color::Rgb(120, 200, 200),
            ansi_white: Color::Rgb(220, 220, 220),
        }
    }

    /// Catppuccin Mocha — Soothing pastels.
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "catppuccin-mocha".to_string(),
            author: Some("Catppuccin".into()),
            description: Some("Soothing pastels — the Mocha variant".into()),
            bg: Color::Rgb(30, 30, 46),          // #1E1E2E
            fg: Color::Rgb(205, 214, 244),        // #CDD6F4
            surface: Color::Rgb(49, 50, 68),      // #313244
            overlay: Color::Rgb(69, 71, 90),      // #45475A
            primary: Color::Rgb(137, 180, 250),   // #89B4FA (blue)
            secondary: Color::Rgb(249, 226, 175), // #F9E2AF (yellow)
            dim: Color::Rgb(69, 71, 90),
            muted: Color::Rgb(108, 112, 134),     // #6C7086
            ansi_black: Color::Rgb(30, 30, 46),
            ansi_red: Color::Rgb(243, 139, 168),  // #F38BA8
            ansi_green: Color::Rgb(166, 227, 161), // #A6E3A1
            ansi_yellow: Color::Rgb(249, 226, 175),
            ansi_blue: Color::Rgb(137, 180, 250),
            ansi_magenta: Color::Rgb(245, 194, 231), // #F5C2E7
            ansi_cyan: Color::Rgb(148, 226, 213),  // #94E2D5
            ansi_white: Color::Rgb(205, 214, 244),
        }
    }

    /// Tokyo Night — Cool blues and muted tones.
    pub fn tokyo_night() -> Self {
        Self {
            name: "tokyo-night".to_string(),
            author: Some("enkia".into()),
            description: Some("Cool blues and muted tones, inspired by Tokyo at night".into()),
            bg: Color::Rgb(26, 27, 38),
            fg: Color::Rgb(192, 202, 245),        // #C0CAF5
            surface: Color::Rgb(36, 40, 59),      // #24283B
            overlay: Color::Rgb(52, 59, 88),      // #343B58
            primary: Color::Rgb(125, 207, 255),   // #7DCFFF (cyan)
            secondary: Color::Rgb(224, 175, 104), // #E0AF68 (amber)
            dim: Color::Rgb(52, 59, 88),
            muted: Color::Rgb(86, 95, 137),       // #565F89
            ansi_black: Color::Rgb(26, 27, 38),
            ansi_red: Color::Rgb(247, 118, 142),  // #F7768E
            ansi_green: Color::Rgb(158, 206, 106), // #9ECE6A
            ansi_yellow: Color::Rgb(224, 175, 104),
            ansi_blue: Color::Rgb(125, 207, 255),
            ansi_magenta: Color::Rgb(187, 154, 247), // #BB9AF7
            ansi_cyan: Color::Rgb(125, 207, 255),
            ansi_white: Color::Rgb(192, 202, 245),
        }
    }

    /// Solarized Dark — Ethan Schoonover's precision palette.
    pub fn solarized_dark() -> Self {
        Self {
            name: "solarized-dark".to_string(),
            author: Some("Ethan Schoonover".into()),
            description: Some("Precision palette tuned for readability".into()),
            bg: Color::Rgb(0, 43, 54),            // #002B36
            fg: Color::Rgb(131, 148, 150),         // #839496
            surface: Color::Rgb(7, 54, 66),        // #073642
            overlay: Color::Rgb(88, 110, 117),     // #586E75
            primary: Color::Rgb(38, 139, 210),    // #268BD2 (blue)
            secondary: Color::Rgb(181, 137, 0),   // #B58900 (yellow)
            dim: Color::Rgb(88, 110, 117),
            muted: Color::Rgb(101, 123, 131),      // #657B83
            ansi_black: Color::Rgb(0, 43, 54),
            ansi_red: Color::Rgb(220, 50, 47),    // #DC322F
            ansi_green: Color::Rgb(133, 153, 0),  // #859900
            ansi_yellow: Color::Rgb(181, 137, 0),
            ansi_blue: Color::Rgb(38, 139, 210),
            ansi_magenta: Color::Rgb(211, 54, 130), // #D33682
            ansi_cyan: Color::Rgb(42, 161, 152),   // #2AA198
            ansi_white: Color::Rgb(238, 232, 213),  // #EEE8D5
        }
    }
}

impl ThemeData {
    /// Neon Light — daytime companion to the signature dark palette.
    pub fn neon_light() -> Self {
        Self {
            name: "neon-light".to_string(),
            author: Some("StriVo".into()),
            description: Some("Daylight variant — dark accents on a light paper".into()),
            bg: Color::Rgb(246, 246, 244),
            fg: Color::Rgb(30, 30, 40),
            surface: Color::Rgb(232, 232, 228),
            overlay: Color::Rgb(210, 210, 206),
            primary: Color::Rgb(0, 120, 160),
            secondary: Color::Rgb(180, 90, 0),
            dim: Color::Rgb(170, 170, 166),
            muted: Color::Rgb(100, 100, 110),
            ansi_black: Color::Rgb(50, 50, 60),
            ansi_red: Color::Rgb(200, 60, 60),
            ansi_green: Color::Rgb(0, 150, 80),
            ansi_yellow: Color::Rgb(180, 90, 0),
            ansi_blue: Color::Rgb(0, 120, 160),
            ansi_magenta: Color::Rgb(160, 60, 180),
            ansi_cyan: Color::Rgb(0, 140, 160),
            ansi_white: Color::Rgb(30, 30, 40),
        }
    }

    /// High-contrast dark variant for low-vision users (F.3).
    pub fn neon_hc() -> Self {
        Self {
            name: "neon-hc".to_string(),
            author: Some("StriVo".into()),
            description: Some("High-contrast dark variant — maximum legibility".into()),
            bg: Color::Rgb(0, 0, 0),
            fg: Color::Rgb(255, 255, 255),
            surface: Color::Rgb(16, 16, 16),
            overlay: Color::Rgb(40, 40, 40),
            primary: Color::Rgb(0, 255, 255),
            secondary: Color::Rgb(255, 200, 0),
            dim: Color::Rgb(120, 120, 120),
            muted: Color::Rgb(200, 200, 200),
            ansi_black: Color::Rgb(0, 0, 0),
            ansi_red: Color::Rgb(255, 80, 80),
            ansi_green: Color::Rgb(80, 255, 120),
            ansi_yellow: Color::Rgb(255, 230, 0),
            ansi_blue: Color::Rgb(100, 180, 255),
            ansi_magenta: Color::Rgb(255, 120, 220),
            ansi_cyan: Color::Rgb(0, 255, 255),
            ansi_white: Color::Rgb(255, 255, 255),
        }
    }

    pub fn gruvbox_dark() -> Self {
        Self {
            name: "gruvbox-dark".to_string(),
            author: Some("morhetz".into()),
            description: Some("Retro groove — warm earth tones on dark".into()),
            bg: Color::Rgb(40, 40, 40),
            fg: Color::Rgb(235, 219, 178),
            surface: Color::Rgb(60, 56, 54),
            overlay: Color::Rgb(80, 73, 69),
            primary: Color::Rgb(131, 165, 152),  // aqua
            secondary: Color::Rgb(250, 189, 47), // yellow
            dim: Color::Rgb(102, 92, 84),
            muted: Color::Rgb(168, 153, 132),
            ansi_black: Color::Rgb(40, 40, 40),
            ansi_red: Color::Rgb(251, 73, 52),
            ansi_green: Color::Rgb(184, 187, 38),
            ansi_yellow: Color::Rgb(250, 189, 47),
            ansi_blue: Color::Rgb(131, 165, 152),
            ansi_magenta: Color::Rgb(211, 134, 155),
            ansi_cyan: Color::Rgb(142, 192, 124),
            ansi_white: Color::Rgb(235, 219, 178),
        }
    }

    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            author: Some("Arctic Ice Studio".into()),
            description: Some("Arctic, north-bluish clean and elegant".into()),
            bg: Color::Rgb(46, 52, 64),
            fg: Color::Rgb(216, 222, 233),
            surface: Color::Rgb(59, 66, 82),
            overlay: Color::Rgb(67, 76, 94),
            primary: Color::Rgb(136, 192, 208),
            secondary: Color::Rgb(235, 203, 139),
            dim: Color::Rgb(76, 86, 106),
            muted: Color::Rgb(155, 166, 182),
            ansi_black: Color::Rgb(46, 52, 64),
            ansi_red: Color::Rgb(191, 97, 106),
            ansi_green: Color::Rgb(163, 190, 140),
            ansi_yellow: Color::Rgb(235, 203, 139),
            ansi_blue: Color::Rgb(129, 161, 193),
            ansi_magenta: Color::Rgb(180, 142, 173),
            ansi_cyan: Color::Rgb(136, 192, 208),
            ansi_white: Color::Rgb(229, 233, 240),
        }
    }

    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            author: Some("Zeno Rocha".into()),
            description: Some("Dark theme for night owls — vivid neons on black".into()),
            bg: Color::Rgb(40, 42, 54),
            fg: Color::Rgb(248, 248, 242),
            surface: Color::Rgb(68, 71, 90),
            overlay: Color::Rgb(98, 114, 164),
            primary: Color::Rgb(139, 233, 253),
            secondary: Color::Rgb(241, 250, 140),
            dim: Color::Rgb(98, 114, 164),
            muted: Color::Rgb(189, 147, 249),
            ansi_black: Color::Rgb(40, 42, 54),
            ansi_red: Color::Rgb(255, 85, 85),
            ansi_green: Color::Rgb(80, 250, 123),
            ansi_yellow: Color::Rgb(241, 250, 140),
            ansi_blue: Color::Rgb(139, 233, 253),
            ansi_magenta: Color::Rgb(255, 121, 198),
            ansi_cyan: Color::Rgb(139, 233, 253),
            ansi_white: Color::Rgb(248, 248, 242),
        }
    }

    pub fn rose_pine_moon() -> Self {
        Self {
            name: "rose-pine-moon".to_string(),
            author: Some("Emilia".into()),
            description: Some("All natural pine, faux fur and a bit of soho vibes".into()),
            bg: Color::Rgb(35, 33, 54),
            fg: Color::Rgb(224, 222, 244),
            surface: Color::Rgb(57, 53, 82),
            overlay: Color::Rgb(68, 65, 90),
            primary: Color::Rgb(156, 207, 216),
            secondary: Color::Rgb(246, 193, 119),
            dim: Color::Rgb(110, 106, 134),
            muted: Color::Rgb(144, 140, 170),
            ansi_black: Color::Rgb(35, 33, 54),
            ansi_red: Color::Rgb(235, 111, 146),
            ansi_green: Color::Rgb(62, 143, 176),
            ansi_yellow: Color::Rgb(246, 193, 119),
            ansi_blue: Color::Rgb(156, 207, 216),
            ansi_magenta: Color::Rgb(196, 167, 231),
            ansi_cyan: Color::Rgb(234, 154, 151),
            ansi_white: Color::Rgb(224, 222, 244),
        }
    }

    pub fn kanagawa() -> Self {
        Self {
            name: "kanagawa".to_string(),
            author: Some("rebelot".into()),
            description: Some("Inspired by The Great Wave off Kanagawa, Hokusai".into()),
            bg: Color::Rgb(31, 31, 40),
            fg: Color::Rgb(220, 215, 186),
            surface: Color::Rgb(42, 42, 55),
            overlay: Color::Rgb(54, 54, 70),
            primary: Color::Rgb(122, 168, 159),
            secondary: Color::Rgb(220, 165, 97),
            dim: Color::Rgb(84, 84, 100),
            muted: Color::Rgb(150, 150, 170),
            ansi_black: Color::Rgb(31, 31, 40),
            ansi_red: Color::Rgb(195, 64, 67),
            ansi_green: Color::Rgb(118, 148, 106),
            ansi_yellow: Color::Rgb(192, 163, 110),
            ansi_blue: Color::Rgb(122, 168, 159),
            ansi_magenta: Color::Rgb(149, 127, 184),
            ansi_cyan: Color::Rgb(106, 150, 166),
            ansi_white: Color::Rgb(220, 215, 186),
        }
    }

    pub fn everforest_dark() -> Self {
        Self {
            name: "everforest-dark".to_string(),
            author: Some("sainnhe".into()),
            description: Some("Green-based dark theme, comfortable for the eyes".into()),
            bg: Color::Rgb(45, 53, 59),
            fg: Color::Rgb(211, 198, 170),
            surface: Color::Rgb(60, 71, 78),
            overlay: Color::Rgb(74, 85, 93),
            primary: Color::Rgb(127, 187, 179),
            secondary: Color::Rgb(219, 188, 127),
            dim: Color::Rgb(86, 99, 110),
            muted: Color::Rgb(157, 169, 160),
            ansi_black: Color::Rgb(45, 53, 59),
            ansi_red: Color::Rgb(230, 126, 128),
            ansi_green: Color::Rgb(167, 192, 128),
            ansi_yellow: Color::Rgb(219, 188, 127),
            ansi_blue: Color::Rgb(127, 187, 179),
            ansi_magenta: Color::Rgb(214, 153, 182),
            ansi_cyan: Color::Rgb(131, 192, 146),
            ansi_white: Color::Rgb(211, 198, 170),
        }
    }
}

/// Returns all built-in themes.
pub fn builtin_themes() -> Vec<ThemeData> {
    vec![
        ThemeData::neon(),
        ThemeData::neon_light(),
        ThemeData::neon_hc(),
        ThemeData::monochrome(),
        ThemeData::catppuccin_mocha(),
        ThemeData::tokyo_night(),
        ThemeData::solarized_dark(),
        ThemeData::gruvbox_dark(),
        ThemeData::nord(),
        ThemeData::dracula(),
        ThemeData::rose_pine_moon(),
        ThemeData::kanagawa(),
        ThemeData::everforest_dark(),
    ]
}

/// Scan user theme directory for `.toml` and `.conf` (Kitty/Ghostty) theme files.
pub fn scan_user_themes() -> Vec<ThemeData> {
    let themes_dir = crate::config::AppConfig::config_dir().join("themes");
    if !themes_dir.exists() {
        return Vec::new();
    }

    let mut themes = Vec::new();
    let entries = match std::fs::read_dir(&themes_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to read themes directory: {e}");
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        let parsed = match ext {
            Some("toml") => std::fs::read_to_string(&path)
                .map_err(|e| format!("read: {e}"))
                .and_then(|c| toml::from_str::<ThemeData>(&c).map_err(|e| format!("parse: {e}"))),
            Some("conf") => std::fs::read_to_string(&path)
                .map_err(|e| format!("read: {e}"))
                .and_then(|c| {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("imported")
                        .to_string();
                    kitty_import::parse(&name, &c).map_err(|e| format!("parse: {e}"))
                }),
            _ => continue,
        };
        match parsed {
            Ok(theme) => {
                tracing::info!("Loaded user theme '{}' from {}", theme.name, path.display());
                themes.push(theme);
            }
            Err(e) => tracing::warn!("Failed to load theme {}: {e}", path.display()),
        }
    }

    themes
}

/// Apply TOML override maps onto a resolved theme. Unknown keys are ignored
/// with a trace log. Invalid hex values are silently dropped (the theme keeps
/// its original slot so bad input degrades gracefully instead of crashing).
pub fn apply_overrides(
    mut theme: ThemeData,
    colors: &BTreeMap<String, String>,
    ansi: &BTreeMap<String, String>,
) -> ThemeData {
    for (slot, value) in colors {
        let Some(color) = parse_hex_color(value) else {
            tracing::warn!("theme override: invalid hex '{value}' for colors.{slot}");
            continue;
        };
        match slot.as_str() {
            "bg" => theme.bg = color,
            "fg" => theme.fg = color,
            "surface" => theme.surface = color,
            "overlay" => theme.overlay = color,
            "primary" => theme.primary = color,
            "secondary" => theme.secondary = color,
            "dim" => theme.dim = color,
            "muted" => theme.muted = color,
            _ => tracing::trace!("theme override: unknown colors.{slot}"),
        }
    }
    for (slot, value) in ansi {
        let Some(color) = parse_hex_color(value) else {
            tracing::warn!("theme override: invalid hex '{value}' for ansi.{slot}");
            continue;
        };
        match slot.as_str() {
            "black" => theme.ansi_black = color,
            "red" => theme.ansi_red = color,
            "green" => theme.ansi_green = color,
            "yellow" => theme.ansi_yellow = color,
            "blue" => theme.ansi_blue = color,
            "magenta" => theme.ansi_magenta = color,
            "cyan" => theme.ansi_cyan = color,
            "white" => theme.ansi_white = color,
            _ => tracing::trace!("theme override: unknown ansi.{slot}"),
        }
    }
    theme
}

/// List all available theme names (user themes override built-ins).
pub fn available_themes() -> Vec<String> {
    let user = scan_user_themes();
    let builtins = builtin_themes();

    let mut names: Vec<String> = user.iter().map(|t| t.name.clone()).collect();
    for b in &builtins {
        if !names.contains(&b.name) {
            names.push(b.name.clone());
        }
    }
    names
}

/// Resolve a theme by name: user files > built-ins > default.
pub fn resolve_theme(name: &str) -> ThemeData {
    for theme in scan_user_themes() {
        if theme.name == name {
            return theme;
        }
    }
    for theme in builtin_themes() {
        if theme.name == name {
            return theme;
        }
    }
    ThemeData::neon()
}

// ── Theme accessor (per-frame cached) ───────────────────────────────────

pub struct Theme;

#[allow(dead_code)]
impl Theme {
    /// Initialize the global theme. Call once at startup.
    pub fn init(theme_name: &str) {
        Self::init_with_overrides(theme_name, &BTreeMap::new(), &BTreeMap::new());
    }

    /// Initialize with override maps applied on top of the named theme.
    pub fn init_with_overrides(
        theme_name: &str,
        colors: &BTreeMap<String, String>,
        ansi: &BTreeMap<String, String>,
    ) {
        let data = apply_overrides(resolve_theme(theme_name), colors, ansi);
        let _ = THEME.set(RwLock::new(data));
    }

    /// Switch the global theme at runtime (no overrides).
    pub fn set(theme_name: &str) {
        Self::set_with_overrides(theme_name, &BTreeMap::new(), &BTreeMap::new());
    }

    /// Switch and apply overrides atomically.
    pub fn set_with_overrides(
        theme_name: &str,
        colors: &BTreeMap<String, String>,
        ansi: &BTreeMap<String, String>,
    ) {
        let data = apply_overrides(resolve_theme(theme_name), colors, ansi);
        if let Some(lock) = THEME.get() {
            if let Ok(mut guard) = lock.write() {
                *guard = data;
                GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Get the current theme name.
    pub fn current_name() -> String {
        Self::cached().name.clone()
    }

    /// Snapshot the current theme. Used by the theme picker so `Esc` can
    /// revert to whatever was live before the preview session started.
    pub fn snapshot() -> ThemeData {
        Self::cached()
    }

    /// Replace the current theme with a previously captured snapshot. Does
    /// not touch config — this is for transient preview/revert flows.
    pub fn restore(data: ThemeData) {
        if let Some(lock) = THEME.get() {
            if let Ok(mut guard) = lock.write() {
                *guard = data;
                GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Get a cached copy of the current theme data. Reads the RwLock at most
    /// once per generation (i.e., once per theme switch), not once per call.
    fn cached() -> ThemeData {
        let gen = GENERATION.load(std::sync::atomic::Ordering::Relaxed);
        CACHED.with(|cell| {
            let cached = cell.borrow();
            if cached.0 == gen {
                return cached.1.clone();
            }
            drop(cached);

            let data = THEME
                .get()
                .and_then(|lock| lock.read().ok())
                .map(|t| t.clone())
                .unwrap_or_else(ThemeData::neon);
            *cell.borrow_mut() = (gen, data.clone());
            data
        })
    }

    // ── Base surfaces ───────────────────────────────────────────────────
    pub fn bg() -> Color { Self::cached().bg }
    pub fn fg() -> Color { Self::cached().fg }
    pub fn surface() -> Color { Self::cached().surface }
    pub fn overlay() -> Color { Self::cached().overlay }

    // ── Semantic accents ────────────────────────────────────────────────
    pub fn primary() -> Color { Self::cached().primary }
    pub fn secondary() -> Color { Self::cached().secondary }
    pub fn dim() -> Color { Self::cached().dim }
    pub fn muted() -> Color { Self::cached().muted }

    // ── ANSI slots ──────────────────────────────────────────────────────
    pub fn red() -> Color { Self::cached().ansi_red }
    pub fn green() -> Color { Self::cached().ansi_green }
    pub fn yellow() -> Color { Self::cached().ansi_yellow }
    pub fn blue() -> Color { Self::cached().ansi_blue }
    pub fn magenta() -> Color { Self::cached().ansi_magenta }
    pub fn cyan() -> Color { Self::cached().ansi_cyan }

    // ── Platform colors (fixed constants, NOT themeable) ────────────────
    pub fn twitch() -> Color { TWITCH_COLOR }
    pub fn youtube() -> Color { YOUTUBE_COLOR }
    pub fn patreon() -> Color { PATREON_COLOR }

    // ── Derived: hotkey bar background ──────────────────────────────────
    fn hotkey_bg() -> Color {
        match Self::surface() {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(10),
                g.saturating_add(10),
                b.saturating_add(14),
            ),
            other => other,
        }
    }

    // ── Style helpers ───────────────────────────────────────────────────
    pub fn title() -> Style {
        Style::new().fg(Self::primary()).add_modifier(Modifier::BOLD)
    }

    pub fn selected() -> Style {
        Style::new().fg(Self::bg()).bg(Self::primary())
    }

    pub fn status_live() -> Style {
        Style::new().fg(Self::green()).add_modifier(Modifier::BOLD)
    }

    pub fn status_recording() -> Style {
        Style::new().fg(Self::red()).add_modifier(Modifier::BOLD)
    }

    pub fn status_offline() -> Style {
        Style::new().fg(Self::muted())
    }

    pub fn border() -> Style {
        Style::new().fg(Self::dim())
    }

    pub fn border_focused() -> Style {
        Style::new().fg(Self::primary())
    }

    /// Border style for a freshly-focused pane — ramps from dim → primary
    /// over 180 ms (DESIGN.md §Motion). After the ramp completes it's
    /// indistinguishable from [`border_focused`]. Honors `STRIVO_REDUCE_MOTION`.
    pub fn border_focused_ramp(elapsed_secs: f32) -> Style {
        use crate::tui::anim::{easing::Ease, reduce_motion};
        if reduce_motion() {
            return Self::border_focused();
        }
        const DURATION: f32 = 0.18;
        let t = (elapsed_secs / DURATION).clamp(0.0, 1.0);
        let eased = Ease::Standard.apply(t);
        Style::new().fg(Self::blend_for(Self::dim(), Self::primary(), eased))
    }

    /// Border style at a supplied eased progress. Use for overlays driven by
    /// `AppState::overlay_enter(...)` which already applies the ease curve.
    pub fn border_ramp(progress: f32) -> Style {
        Style::new().fg(Self::blend_for(Self::dim(), Self::primary(), progress))
    }

    /// Unfocused-border fade: when a pane was previously focused and just
    /// lost focus, eases primary → dim over `DURATION` so the change reads as
    /// a transition rather than an instantaneous swap. Elsewhere (long
    /// unfocused), `border()` is sufficient.
    pub fn border_unfocused_ramp(elapsed_secs: f32) -> Style {
        use crate::tui::anim::{easing::Ease, reduce_motion};
        if reduce_motion() {
            return Self::border();
        }
        const DURATION: f32 = 0.12;
        let t = (elapsed_secs / DURATION).clamp(0.0, 1.0);
        let eased = Ease::InCubic.apply(t);
        Style::new().fg(Self::blend_for(Self::primary(), Self::dim(), eased))
    }

    /// Semantic helpers per DESIGN-TODOS D-section so widgets stop reaching
    /// for raw `Color::` names.
    pub fn log_error() -> Style { Style::new().fg(Self::red()) }
    pub fn log_warn() -> Style { Style::new().fg(Self::secondary()) }
    pub fn log_info() -> Style { Style::new().fg(Self::fg()) }
    pub fn log_debug() -> Style { Style::new().fg(Self::muted()) }
    pub fn log_trace() -> Style { Style::new().fg(Self::dim()) }
    pub fn scrollbar_thumb() -> Style { Style::new().fg(Self::primary()) }
    pub fn scrollbar_track() -> Style { Style::new().fg(Self::dim()) }
    pub fn indicator_active() -> Style { Style::new().fg(Self::green()) }
    pub fn indicator_idle() -> Style { Style::new().fg(Self::muted()) }
    pub fn status_paused() -> Style { Style::new().fg(Self::secondary()) }

    /// Linear RGB blend of two theme colors by `t ∈ [0, 1]`. Non-RGB colors
    /// snap at the halfway point — they don't interpolate meaningfully.
    pub fn blend_for(a: Color, b: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        match (a, b) {
            (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
                ((ar as f32) + (br as f32 - ar as f32) * t).round() as u8,
                ((ag as f32) + (bg as f32 - ag as f32) * t).round() as u8,
                ((ab as f32) + (bb as f32 - ab as f32) * t).round() as u8,
            ),
            _ => {
                if t < 0.5 {
                    a
                } else {
                    b
                }
            }
        }
    }

    pub fn status_bar() -> Style {
        Style::new().fg(Self::fg()).bg(Self::overlay())
    }

    pub fn key_hint() -> Style {
        Style::new().fg(Self::secondary())
    }

    pub fn error() -> Style {
        Style::new().fg(Self::red())
    }

    pub fn hotkey_bar() -> Style {
        Style::new().fg(Self::fg()).bg(Self::hotkey_bg())
    }

    pub fn hotkey_key() -> Style {
        Style::new()
            .fg(Self::secondary())
            .bg(Self::hotkey_bg())
            .add_modifier(Modifier::BOLD)
    }

    pub fn day_header() -> Style {
        Style::new()
            .fg(Self::primary())
            .add_modifier(Modifier::BOLD)
    }

    pub fn stream_subtitle() -> Style {
        Style::new()
            .fg(Self::muted())
            .add_modifier(Modifier::ITALIC)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color_valid() {
        assert_eq!(parse_hex_color("#ff5555"), Some(Color::Rgb(255, 85, 85)));
        assert_eq!(parse_hex_color("00e5ff"), Some(Color::Rgb(0, 229, 255)));
        assert_eq!(parse_hex_color("#000000"), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_hex_color("#FFFFFF"), Some(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("#fff"), None); // too short
        assert_eq!(parse_hex_color("#gggggg"), None); // invalid chars
        assert_eq!(parse_hex_color("#1234567"), None); // too long
    }

    #[test]
    fn test_all_builtin_themes_have_unique_names() {
        let themes = builtin_themes();
        let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "duplicate theme names found");
    }

    #[test]
    fn test_resolve_theme_fallback() {
        let t = resolve_theme("nonexistent-theme");
        assert_eq!(t.name, "neon");
    }

    #[test]
    fn test_all_builtins_toml_roundtrip() {
        for original in builtin_themes() {
            let serialized = toml::to_string_pretty(&original).expect("serialize");
            let decoded: ThemeData = toml::from_str(&serialized).expect("deserialize");
            assert_eq!(decoded.name, original.name);
            assert_eq!(decoded.bg, original.bg);
            assert_eq!(decoded.fg, original.fg);
            assert_eq!(decoded.primary, original.primary);
            assert_eq!(decoded.ansi_red, original.ansi_red);
        }
    }

    #[test]
    fn test_overrides_apply() {
        let mut colors = BTreeMap::new();
        colors.insert("primary".to_string(), "#FF00FF".to_string());
        let mut ansi = BTreeMap::new();
        ansi.insert("red".to_string(), "#00FF00".to_string());
        let t = apply_overrides(ThemeData::neon(), &colors, &ansi);
        assert_eq!(t.primary, Color::Rgb(0xff, 0x00, 0xff));
        assert_eq!(t.ansi_red, Color::Rgb(0x00, 0xff, 0x00));
    }
}
