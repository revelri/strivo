use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border())
        .title(" Recordings ")
        .title_style(Theme::title());

    let placeholder = Paragraph::new("No active recordings")
        .style(Style::new().fg(Theme::GRAY))
        .block(block);
    frame.render_widget(placeholder, area);
}
