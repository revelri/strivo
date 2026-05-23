//! Plugin browser overlay (Shift+P).
//!
//! Shows every plugin the host knows about — both *loaded* (registered with
//! the `PluginRegistry`) and *installed* (manifests scanned from
//! `~/.config/strivo/plugins/`). Replaces the bare list in Settings with a
//! richer view: per-plugin status chip, description, activation key, plus
//! manifests that are present on disk but not (yet) loaded.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::AppState;
use crate::plugin::registry::{PluginRegistry, PluginStatus};
use crate::tui::theme::Theme;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    registry: &PluginRegistry,
    enter_progress: f32,
) {
    // 70% width, 70% height — same proportions as the theme picker so
    // the two overlays feel like siblings.
    let h = area.height.saturating_mul(7) / 10;
    let h = h.min(28).max(14);
    let w = area.width.saturating_mul(7) / 10;
    let w = w.min(80).max(56);

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
        .title(" Plugins ")
        .title_style(Theme::title());
    let inner = block.inner(center);
    frame.render_widget(block, center);

    let loaded = registry.plugin_statuses();
    let loaded_names: std::collections::HashSet<String> =
        loaded.iter().map(|(n, _, _)| n.to_string()).collect();

    let mut lines: Vec<Line> = Vec::new();

    // ── Loaded plugins ────────────────────────────────────────────────
    lines.push(Line::from(Span::styled(
        "Loaded",
        Style::new()
            .fg(Theme::secondary())
            .add_modifier(Modifier::BOLD),
    )));
    if loaded.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::new().fg(Theme::muted()),
        )));
    } else {
        for (name, display_name, status) in loaded.iter() {
            let (chip, chip_fg, chip_bg) = status_chip(status);
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!(" {chip} "),
                    Style::new().fg(chip_fg).bg(chip_bg).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    (*display_name).to_string(),
                    Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ({name})"),
                    Style::new().fg(Theme::muted()),
                ),
            ]));
            if let PluginStatus::Error(msg) = status {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(
                        format!("error: {}", truncate(msg, inner.width as usize)),
                        Style::new().fg(Theme::red()),
                    ),
                ]));
            }
        }
    }

    // ── Installed (manifests not loaded) ──────────────────────────────
    let installed_only: Vec<_> = app
        .user_plugin_manifests
        .iter()
        .filter(|m| !loaded_names.contains(&m.name))
        .collect();

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Installed (manifests not loaded)",
        Style::new()
            .fg(Theme::secondary())
            .add_modifier(Modifier::BOLD),
    )));
    if installed_only.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::new().fg(Theme::muted()),
        )));
    } else {
        for m in &installed_only {
            let version = m.version.as_deref().unwrap_or("?");
            let key = m.activation_key.as_deref().unwrap_or("—");
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    " manifest ",
                    Style::new()
                        .fg(Theme::bg())
                        .bg(Theme::muted())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    m.name.clone(),
                    Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  v{version}"), Style::new().fg(Theme::muted())),
                Span::styled(format!("  key: {key}"), Style::new().fg(Theme::dim())),
            ]));
            if let Some(ref desc) = m.description {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(
                        truncate(desc, inner.width as usize),
                        Style::new().fg(Theme::muted()),
                    ),
                ]));
            }
        }
    }

    // ── Footer ────────────────────────────────────────────────────────
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("[Esc]", Theme::key_hint()),
        Span::raw(" Close  "),
        Span::styled("[Shift+P]", Theme::key_hint()),
        Span::raw(" Toggle"),
    ]));

    let scroll = if lines.len() > inner.height as usize {
        lines.len().saturating_sub(inner.height as usize)
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        inner,
    );
}

fn status_chip(status: &PluginStatus) -> (&'static str, ratatui::style::Color, ratatui::style::Color) {
    match status {
        PluginStatus::Ready => ("ready", Theme::bg(), Theme::green()),
        PluginStatus::Initializing => ("init", Theme::bg(), Theme::primary()),
        PluginStatus::Error(_) => ("error", Theme::bg(), Theme::red()),
        PluginStatus::Disabled => ("off", Theme::bg(), Theme::dim()),
    }
}

fn truncate(s: &str, width: usize) -> String {
    let cap = width.saturating_sub(6);
    if s.chars().count() > cap {
        let cut: String = s.chars().take(cap.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}
