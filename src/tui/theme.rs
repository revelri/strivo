use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

impl Theme {
    // Base colors
    pub const BG: Color = Color::Rgb(24, 24, 32);
    pub const FG: Color = Color::Rgb(205, 214, 244);
    pub const SURFACE: Color = Color::Rgb(30, 30, 46);
    pub const OVERLAY: Color = Color::Rgb(49, 50, 68);

    // Accent colors
    pub const PURPLE: Color = Color::Rgb(137, 180, 250);
    pub const GREEN: Color = Color::Rgb(166, 227, 161);
    pub const RED: Color = Color::Rgb(243, 139, 168);
    pub const YELLOW: Color = Color::Rgb(249, 226, 175);
    pub const BLUE: Color = Color::Rgb(116, 199, 236);
    pub const GRAY: Color = Color::Rgb(88, 91, 112);
    pub const DIM: Color = Color::Rgb(69, 71, 90);

    // Platform colors
    pub const TWITCH: Color = Color::Rgb(145, 70, 255);
    pub const YOUTUBE: Color = Color::Rgb(255, 0, 0);

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
}
