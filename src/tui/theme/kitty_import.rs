//! Kitty / Ghostty `.conf` theme importer.
//!
//! Both tools use a line-oriented keyword format. We recognize the standard
//! keyword → hex lines shown below; anything else is ignored with a trace log.
//! Comments start with `#`, blank lines are skipped, leading/trailing
//! whitespace is trimmed.
//!
//! | Kitty keyword          | StriVo slot   |
//! |------------------------|---------------|
//! | `background`           | `bg`          |
//! | `foreground`           | `fg`          |
//! | `selection_background` | `overlay`     |
//! | `selection_foreground` | *(unused)*    |
//! | `cursor`               | `primary`     |
//! | `cursor_text_color`    | *(unused)*    |
//! | `color0`               | `ansi_black`  |
//! | `color1`               | `ansi_red`    |
//! | `color2`               | `ansi_green`  |
//! | `color3`               | `ansi_yellow` |
//! | `color4`               | `ansi_blue`   |
//! | `color5`               | `ansi_magenta`|
//! | `color6`               | `ansi_cyan`   |
//! | `color7`               | `ansi_white`  |
//! | `color8..15`           | bright variants overwrite the base ANSI slot |
//!
//! `surface`, `secondary`, `dim`, `muted` are derived from the palette when
//! not explicitly set: `surface` = lerp(bg → fg, 0.08), `dim` = lerp(bg → fg,
//! 0.32), `muted` = lerp(bg → fg, 0.66), `secondary` defaults to `ansi_yellow`.

use super::{parse_hex_color, ThemeData};
use ratatui::style::Color;

/// Parse a Kitty/Ghostty `.conf` theme. `name` is used as the returned
/// theme's display name (typically the file stem).
pub fn parse(name: &str, contents: &str) -> Result<ThemeData, String> {
    // Start from Neon and overwrite slots we find. That way unrecognized files
    // still produce a usable theme.
    let mut theme = ThemeData::neon();
    theme.name = name.to_string();

    let mut saw_bg = false;
    let mut saw_fg = false;
    let mut saw_secondary = false;
    let mut saw_overlay = false;
    let mut saw_surface = false;

    for (lineno, raw) in contents.lines().enumerate() {
        let line = raw.trim();
        // Full-line comment (Kitty: `#` at start of line, after whitespace strip).
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        // Key, then value (the rest of the line after value is comment / extra).
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        if key.is_empty() || value.is_empty() {
            continue;
        }
        let Some(color) = parse_hex_color(value) else {
            tracing::trace!("kitty_import {name}:{}: unparseable value '{value}' for '{key}'", lineno + 1);
            continue;
        };
        match key {
            "background" => {
                theme.bg = color;
                saw_bg = true;
            }
            "foreground" => {
                theme.fg = color;
                saw_fg = true;
            }
            "selection_background" => {
                theme.overlay = color;
                saw_overlay = true;
            }
            "selection_foreground" | "cursor_text_color" | "url_color" => {}
            "cursor" => theme.primary = color,
            "color0" => theme.ansi_black = color,
            "color1" => theme.ansi_red = color,
            "color2" => theme.ansi_green = color,
            "color3" => {
                theme.ansi_yellow = color;
                if !saw_secondary {
                    theme.secondary = color;
                }
            }
            "color4" => theme.ansi_blue = color,
            "color5" => theme.ansi_magenta = color,
            "color6" => theme.ansi_cyan = color,
            "color7" => theme.ansi_white = color,
            // Bright ANSI: overwrite — most themes look better with the vivid
            // variants for our accent-heavy UI.
            "color8" => theme.ansi_black = color,
            "color9" => theme.ansi_red = color,
            "color10" => theme.ansi_green = color,
            "color11" => {
                theme.ansi_yellow = color;
                if !saw_secondary {
                    theme.secondary = color;
                }
            }
            "color12" => theme.ansi_blue = color,
            "color13" => theme.ansi_magenta = color,
            "color14" => theme.ansi_cyan = color,
            "color15" => theme.ansi_white = color,
            // Explicit StriVo-specific slot overrides — allow a `.conf` to
            // target our semantic names directly alongside the Kitty keys.
            "surface" => {
                theme.surface = color;
                saw_surface = true;
            }
            "overlay" => {
                theme.overlay = color;
                saw_overlay = true;
            }
            "dim" => theme.dim = color,
            "muted" => theme.muted = color,
            "primary" => theme.primary = color,
            "secondary" => {
                theme.secondary = color;
                saw_secondary = true;
            }
            _ => tracing::trace!("kitty_import {name}: unknown key '{key}'"),
        }
    }

    if !saw_bg && !saw_fg {
        return Err("no recognized keys in theme file".into());
    }
    // Derive surface/dim/muted/overlay from bg/fg when the file didn't set them.
    if !saw_surface {
        theme.surface = lerp_rgb(theme.bg, theme.fg, 0.08);
    }
    if !saw_overlay {
        theme.overlay = lerp_rgb(theme.bg, theme.fg, 0.20);
    }
    theme.dim = lerp_rgb(theme.bg, theme.fg, 0.32);
    theme.muted = lerp_rgb(theme.bg, theme.fg, 0.66);

    Ok(theme)
}

fn lerp_rgb(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f32 + (br as f32 - ar as f32) * t).round() as u8,
            (ag as f32 + (bg as f32 - ag as f32) * t).round() as u8,
            (ab as f32 + (bb as f32 - ab as f32) * t).round() as u8,
        ),
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_kitty_theme() {
        let conf = r#"
# Catppuccin Mocha (fragment)
foreground              #CDD6F4
background              #1E1E2E
selection_background    #585B70
cursor                  #F5E0DC

color0  #45475A
color1  #F38BA8
color2  #A6E3A1
color3  #F9E2AF
color4  #89B4FA
color5  #F5C2E7
color6  #94E2D5
color7  #BAC2DE
"#;
        let t = parse("test", conf).expect("parse ok");
        assert_eq!(t.name, "test");
        assert_eq!(t.bg, Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(t.fg, Color::Rgb(0xcd, 0xd6, 0xf4));
        assert_eq!(t.primary, Color::Rgb(0xf5, 0xe0, 0xdc));
        assert_eq!(t.ansi_red, Color::Rgb(0xf3, 0x8b, 0xa8));
        assert_eq!(t.secondary, Color::Rgb(0xf9, 0xe2, 0xaf)); // from color3
        assert_eq!(t.overlay, Color::Rgb(0x58, 0x5b, 0x70));
        // surface derived from bg->fg lerp
        match t.surface {
            Color::Rgb(_, _, _) => {}
            other => panic!("surface not RGB: {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_conf() {
        assert!(parse("x", "# just comments\n").is_err());
        assert!(parse("x", "").is_err());
    }

    #[test]
    fn ignores_unknown_keys_without_failing() {
        let conf = "foreground #ffffff\nbackground #000000\nwindow_padding_width 10\n";
        let t = parse("x", conf).expect("parse ok");
        assert_eq!(t.bg, Color::Rgb(0, 0, 0));
        assert_eq!(t.fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn handles_inline_comments() {
        let conf = "foreground #aaaaaa  # light grey\nbackground #111111\n";
        let t = parse("x", conf).unwrap();
        assert_eq!(t.fg, Color::Rgb(0xaa, 0xaa, 0xaa));
        assert_eq!(t.bg, Color::Rgb(0x11, 0x11, 0x11));
    }
}
