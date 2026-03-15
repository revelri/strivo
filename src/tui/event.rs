use crate::app::AppEvent;
use anyhow::Result;
use crossterm::event::{self, Event};
use std::time::Duration;

/// Polls for crossterm events with a timeout, returning an AppEvent if available.
pub fn poll_event(timeout: Duration) -> Result<Option<AppEvent>> {
    if event::poll(timeout)? {
        match event::read()? {
            Event::Key(key) => Ok(Some(AppEvent::Key(key))),
            Event::Resize(w, h) => Ok(Some(AppEvent::Resize(w, h))),
            _ => Ok(None),
        }
    } else {
        Ok(Some(AppEvent::Tick))
    }
}
