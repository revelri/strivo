use ratatui::{
    Frame,
    layout::{Constraint, Layout},
};

use crate::app::{ActivePane, AppState};
use crate::tui::widgets::{channel_detail, dialog, sidebar, status_bar, wizard};

pub fn render(frame: &mut Frame, app: &AppState) {
    let [main_area, status_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let [sidebar_area, detail_area] = Layout::horizontal([
        Constraint::Length(22),
        Constraint::Fill(1),
    ])
    .areas(main_area);

    // Sidebar
    sidebar::render(frame, sidebar_area, app);

    // Main panel
    channel_detail::render(frame, detail_area, app);

    // Status bar
    status_bar::render(frame, status_area, app);

    // Overlays
    if app.active_pane == ActivePane::Wizard {
        wizard::render(frame, frame.area());
    }

    if app.show_help {
        dialog::render_help(frame, frame.area());
    }
}
