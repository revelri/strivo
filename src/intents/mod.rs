//! Recording-intent translators.
//!
//! Pure functions that translate a caller's intent (a `StartSpec` or
//! `DownloadVodSpec`) into a fully-populated
//! [`crate::recording::RecordingCommand`]. The recording engine takes
//! that command verbatim — no further config lookups, no cookie
//! resolution, no path computation downstream.
//!
//! Background: before this module, four sites independently built
//! `RecordingCommand::Start` and five sites independently built
//! `RecordingCommand::DownloadVod`, each with subtly different
//! cookies / output-path / title logic. The webui's `start_recording`
//! passed `cookies_path: None` while the TUI hand-rolled a per-platform
//! match, so a gated YouTube stream started from the webui silently
//! failed where the TUI succeeded. See
//! `docs/internal/recording-dispatch-inventory.md` for the full audit.

pub mod cookies;
pub mod download_vod;
pub mod spec;
pub mod start;

pub use cookies::CookieSource;
pub use download_vod::download_vod;
pub use spec::{DownloadVodSpec, OutputPathPolicy, StartSpec};
pub use start::start_recording;
