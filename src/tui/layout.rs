use ratatui::{
    Frame,
    layout::{Constraint, Layout},
};

use crate::app::{ActivePane, AppState};
use crate::plugin::registry::PluginRegistry;
use crate::tui::widgets::{channel_detail, dialog, log_viewer, platform_debug, properties, recording_list, settings, sidebar, status_bar, wizard};

pub fn render(frame: &mut Frame, app: &mut AppState, registry: &PluginRegistry) {
    let [main_area, status_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let [sidebar_area, detail_area] = Layout::horizontal([
        Constraint::Length(30),
        Constraint::Fill(1),
    ])
    .areas(main_area);

    // Sidebar (always visible)
    sidebar::render(frame, sidebar_area, app);

    // Main panel depends on active pane
    match app.active_pane {
        ActivePane::Detail => channel_detail::render(frame, detail_area, app),
        ActivePane::Settings => settings::render(frame, detail_area, app),
        ActivePane::Log => {
            // Rate-limit log refresh (~every 30 ticks = ~1s at 30fps)
            if app.tick_counter % 30 == 0 {
                app.refresh_log();
            }
            log_viewer::render(frame, detail_area, app);
        }
        ActivePane::Plugin(_) => {
            registry.render_active_pane(frame, detail_area, app);
        }
        // Default: show recording list (Sidebar, RecordingList, or anything else)
        _ => recording_list::render(frame, detail_area, app),
    }

    // Status bar
    status_bar::render(frame, status_area, app, registry);

    // Overlays
    //
    // Wizard surfaces in two cases: the user is on ActivePane::Wizard (first
    // run with no credentials), or an active device-code flow is waiting for
    // the user to enter a code — in which case it's promoted to an overlay
    // regardless of the active pane, so the prompt never gets buried.
    let show_wizard = app.active_pane == ActivePane::Wizard || app.pending_auth.is_some();
    if show_wizard {
        wizard::render(frame, frame.area(), app);
    }

    if app.show_help {
        dialog::render_help(frame, frame.area(), registry, &app.active_pane);
    }

    if app.quit_confirm {
        dialog::render_confirm(frame, frame.area(), "Quit with active recordings?");
    }

    if let Some(kind) = app.show_platform_debug {
        platform_debug::render(frame, frame.area(), app, kind);
    }

    if app.show_properties.is_some() {
        properties::render(frame, frame.area(), app, registry);
    }

    if app.stop_all_deadline.is_some() {
        dialog::render_stopping(frame, frame.area(), app);
    }
}
