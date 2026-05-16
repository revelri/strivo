//! Bridges `tracing` events into the in-memory UI event ring buffer.
//!
//! A custom `tracing_subscriber::Layer` captures every event at or above
//! the configured floor (default `INFO`) emitted by code in the
//! `strivo` / `strivo_core` / `strivo_plugins` targets and forwards it
//! through a tokio `UnboundedSender`. The main loop drains the
//! corresponding receiver and pushes each entry onto
//! `AppState::event_ring`.
//!
//! This is the Live Log Tail half of M1.3.d. The Shift+E pop-over
//! (M1.3.e) renders the same ring. Distinct from `log_lines` (raw file
//! tail) and `platform_errors` (per-platform diagnostics).
//!
//! Events that arrive before the sender is wired (early daemon
//! bootstrap) are dropped — the file log retains them.

use std::sync::OnceLock;

use tokio::sync::mpsc::UnboundedSender;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::app::{AppEvent, UiEvent, UiEventLevel};

static SENDER: OnceLock<UnboundedSender<AppEvent>> = OnceLock::new();

/// Install the channel sender. Idempotent — only the first caller wins.
/// Events that arrive before this call are dropped (the file log still
/// catches them).
pub fn install_sender(tx: UnboundedSender<AppEvent>) {
    let _ = SENDER.set(tx);
}

/// The tracing-subscriber layer. Build at logging-init time:
/// `tracing_subscriber::registry().with(file_layer).with(LogBridgeLayer)`.
pub struct LogBridgeLayer;

impl<S: Subscriber> Layer<S> for LogBridgeLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let Some(tx) = SENDER.get() else {
            return;
        };

        let meta = event.metadata();
        // Only forward strivo-originated events to keep the user pop-over
        // noise-free. Third-party crates (hyper, reqwest, rusqlite) still
        // hit the file log.
        let target = meta.target();
        if !target.starts_with("strivo") {
            return;
        }

        let level = match *meta.level() {
            Level::TRACE => UiEventLevel::Trace,
            Level::DEBUG => UiEventLevel::Debug,
            Level::INFO => UiEventLevel::Info,
            Level::WARN => UiEventLevel::Warn,
            Level::ERROR => UiEventLevel::Error,
        };
        // The user pop-over is for actionable status — drop trace/debug.
        if matches!(level, UiEventLevel::Trace | UiEventLevel::Debug) {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            meta.name().to_string()
        } else {
            visitor.message
        };

        let source: &'static str = if target.contains("crunchr") {
            "crunchr"
        } else if target.contains("archiver") {
            "archiver"
        } else if target.contains("monitor") {
            "monitor"
        } else if target.contains("recording") {
            "recording"
        } else if target.contains("platform") {
            "platform"
        } else {
            "strivo"
        };

        let ev = UiEvent {
            at: chrono::Utc::now(),
            level,
            source,
            message,
        };
        let _ = tx.send(AppEvent::LogBridge(ev));
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}").trim_matches('"').to_string();
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}
