pub mod event;
pub mod layout;
pub mod theme;
pub mod widgets;

use crate::app::{AppEvent, AppState};
use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;

const FRAME_DURATION: Duration = Duration::from_millis(33); // ~30fps

pub async fn run(mut app: AppState, mut rx: mpsc::UnboundedReceiver<AppEvent>) -> Result<()> {
    let mut terminal = ratatui::init();

    let result = run_loop(&mut terminal, &mut app, &mut rx).await;

    ratatui::restore();
    result
}

async fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut AppState,
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| layout::render(frame, app))?;

        // Poll crossterm events (non-blocking with short timeout)
        if let Some(event) = event::poll_event(FRAME_DURATION)? {
            app.handle_event(event);
        }

        // Drain backend events
        while let Ok(event) = rx.try_recv() {
            app.handle_event(event);
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
