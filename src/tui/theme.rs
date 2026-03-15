use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

#[allow(dead_code)]
impl Theme {
    // Base colors (Dracula Neon)
    pub const BG: Color = Color::Rgb(40, 42, 54);
    pub const FG: Color = Color::Rgb(248, 248, 242);
    pub const SURFACE: Color = Color::Rgb(48, 50, 64);
    pub const OVERLAY: Color = Color::Rgb(68, 71, 90);

    // Accent colors
    pub const PURPLE: Color = Color::Rgb(189, 147, 249);
    pub const GREEN: Color = Color::Rgb(80, 250, 123);
    pub const RED: Color = Color::Rgb(255, 85, 85);
    pub const YELLOW: Color = Color::Rgb(241, 250, 140);
    pub const BLUE: Color = Color::Rgb(139, 233, 253);
    pub const GRAY: Color = Color::Rgb(98, 114, 164);
    pub const DIM: Color = Color::Rgb(68, 71, 90);
    pub const PINK: Color = Color::Rgb(255, 121, 198);

    // Platform colors
    pub const TWITCH: Color = Color::Rgb(145, 70, 255);
    pub const YOUTUBE: Color = Color::Rgb(255, 0, 0);

    // Hotkey bar background
    const HOTKEY_BG: Color = Color::Rgb(58, 60, 78);

    pub fn title() -> Style {
        Style::new().fg(Self::PURPLE).add_modifier(Modifier::BOLD)
    }

    pub fn selected() -> Style {
        Style::new().fg(Self::BG).bg(Self::PURPLE)
    }

    pub fn status_live() -> Style {
        Style::new().fg(Self::GREEN).add_modifier(Modifier::BOLD)
    }

    pub fn status_recording() -> Style {
        Style::new().fg(Self::RED).add_modifier(Modifier::BOLD)
    }

    pub fn status_offline() -> Style {
        Style::new().fg(Self::GRAY)
    }

    pub fn border() -> Style {
        Style::new().fg(Self::DIM)
    }

    pub fn border_focused() -> Style {
        Style::new().fg(Self::PURPLE)
    }

    pub fn status_bar() -> Style {
        Style::new().fg(Self::FG).bg(Self::OVERLAY)
    }

    pub fn key_hint() -> Style {
        Style::new().fg(Self::YELLOW)
    }

    pub fn error() -> Style {
        Style::new().fg(Self::RED)
    }

    pub fn hotkey_bar() -> Style {
        Style::new().fg(Self::FG).bg(Self::HOTKEY_BG)
    }

    pub fn hotkey_key() -> Style {
        Style::new()
            .fg(Self::YELLOW)
            .bg(Self::HOTKEY_BG)
            .add_modifier(Modifier::BOLD)
    }

    pub fn day_header() -> Style {
        Style::new()
            .fg(Self::PURPLE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn stream_subtitle() -> Style {
        Style::new()
            .fg(Self::GRAY)
            .add_modifier(Modifier::ITALIC)
    }
}
