use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};

use crate::app::AppState;
use crate::tui::theme::Theme;

use super::ArchiverPlugin;
use super::types::{ArchiveState, ArchiverView};

pub fn render(plugin: &ArchiverPlugin, frame: &mut Frame, area: Rect, _app: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_focused())
        .title(" Archiver ")
        .title_style(Theme::title());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [header_area, content_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    render_header(plugin, frame, header_area);

    match plugin.view {
        ArchiverView::ChannelList => render_channel_list(plugin, frame, content_area),
        ArchiverView::ArchiveQueue => render_queue(plugin, frame, content_area),
    }

    render_footer(plugin, frame, footer_area);
}

fn render_header(plugin: &ArchiverPlugin, frame: &mut Frame, area: Rect) {
    let channel_style = if plugin.view == ArchiverView::ChannelList {
        Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Theme::muted())
    };
    let queue_style = if plugin.view == ArchiverView::ArchiveQueue {
        Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Theme::muted())
    };

    let line = Line::from(vec![
        Span::styled(" Channels", channel_style),
        Span::styled("  |  ", Style::new().fg(Theme::dim())),
        Span::styled("Queue", queue_style),
        Span::styled("  [Tab]", Style::new().fg(Theme::dim())),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_channel_list(plugin: &ArchiverPlugin, frame: &mut Frame, area: Rect) {
    if plugin.channels.is_empty() {
        let lines = vec![
            Line::raw(""),
            Line::styled(
                "  No channels available",
                Style::new().fg(Theme::secondary()).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::styled(
                "  Connect Twitch or YouTube in StriVo to see channels here.",
                Style::new().fg(Theme::muted()),
            ),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let items: Vec<ListItem> = plugin
        .channels
        .iter()
        .map(|ch| {
            let platform_label = match ch.platform.to_string().as_str() {
                "Twitch" => "TW",
                "YouTube" => "YT",
                _ => "??",
            };
            let name_display: String = ch.display_name.chars().take(30).collect();
            let pad = 32usize.saturating_sub(name_display.len() + platform_label.len() + 3);

            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(name_display, Style::new().fg(Theme::fg())),
                Span::raw(" ".repeat(pad)),
                Span::styled(
                    format!("({platform_label})"),
                    Style::new().fg(Theme::dim()),
                ),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(plugin.selected_channel));

    let list = List::new(items)
        .highlight_style(Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_queue(plugin: &ArchiverPlugin, frame: &mut Frame, area: Rect) {
    if plugin.jobs.is_empty() {
        let lines = vec![
            Line::raw(""),
            Line::styled(
                "  No archive jobs",
                Style::new().fg(Theme::muted()),
            ),
            Line::raw(""),
            Line::styled(
                "  Select a channel and press Enter to start archiving.",
                Style::new().fg(Theme::dim()),
            ),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let items: Vec<ListItem> = plugin
        .jobs
        .iter()
        .map(|job| {
            let (indicator, ind_style) = match job.state {
                ArchiveState::Pending => ("○ ", Style::new().fg(Theme::dim())),
                ArchiveState::Scanning => ("⟳ ", Style::new().fg(Theme::secondary())),
                ArchiveState::Downloading => ("⟳ ", Style::new().fg(Theme::secondary())),
                ArchiveState::Paused => ("◼ ", Style::new().fg(Theme::secondary())),
                ArchiveState::Complete => ("✓ ", Style::new().fg(Theme::green())),
                ArchiveState::Failed => ("✗ ", Style::new().fg(Theme::red())),
            };

            let progress = if job.total_videos > 0 {
                let pct = (job.completed_videos as f64 / job.total_videos as f64 * 100.0) as u32;
                format!(" {}/{} ({}%)", job.completed_videos, job.total_videos, pct)
            } else {
                String::new()
            };

            let detail = match job.state {
                ArchiveState::Scanning => " Scanning...".to_string(),
                ArchiveState::Downloading => {
                    let current = job.current_video.as_deref().unwrap_or("");
                    let current_display: String = current.chars().take(25).collect();
                    format!("{progress} {current_display}")
                }
                ArchiveState::Complete => format!(" Complete ({} videos)", job.total_videos),
                ArchiveState::Failed => {
                    let err = job.error.as_deref().unwrap_or("unknown");
                    format!(" {}", err.chars().take(40).collect::<String>())
                }
                _ => String::new(),
            };

            let name_display: String = job.channel_name.chars().take(20).collect();

            ListItem::new(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(indicator, ind_style),
                Span::styled(name_display, Style::new().fg(Theme::fg())),
                Span::styled(detail, Style::new().fg(Theme::muted())),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(plugin.selected_job));

    let list = List::new(items)
        .highlight_style(Style::new().fg(Theme::primary()).add_modifier(Modifier::BOLD));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_footer(plugin: &ArchiverPlugin, frame: &mut Frame, area: Rect) {
    if let Some(ref error) = plugin.last_error {
        let error_display: String = error.chars().take(area.width.saturating_sub(4) as usize).collect();
        let line = Line::from(vec![
            Span::styled(" ⚠ ", Style::new().fg(Theme::red())),
            Span::styled(error_display, Style::new().fg(Theme::red())),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let active = plugin.jobs.iter().filter(|j| {
        j.state == ArchiveState::Downloading || j.state == ArchiveState::Scanning
    }).count();

    let archive_dir = plugin.config.as_ref()
        .map(|c| c.archive_dir.display().to_string())
        .unwrap_or_else(|| "~/Videos/StriVo/Archives".to_string());

    let dir_display: String = archive_dir.chars().take(area.width.saturating_sub(20) as usize).collect();

    let line = if active > 0 {
        Line::from(vec![
            Span::styled(format!(" [AR:{active}]"), Style::new().fg(Theme::secondary())),
            Span::styled(format!("  {dir_display}"), Style::new().fg(Theme::dim())),
        ])
    } else {
        Line::from(vec![
            Span::styled(format!(" {dir_display}"), Style::new().fg(Theme::dim())),
        ])
    };

    frame.render_widget(Paragraph::new(line), area);
}
