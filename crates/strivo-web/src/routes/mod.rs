pub mod api;
pub mod assets;
pub mod events;
pub mod licence;
pub mod login;
// First-party plugin routes (Crunchr/Archiver/Insights/Viewguard + the
// recording captions VTT endpoint). Gated behind the `pro` feature so
// public/free clones of this repo can compile without the private
// strivo-plugins submodule. The licence runtime gate still applies on
// top of this: even when compiled in, locked Pro plugins are filtered
// out for non-entitled clients.
#[cfg(feature = "pro")]
pub mod plugins;
// Retained but unmounted: the sole recording file-serving path (download/
// play) plus the path-containment guard + tests from roadmap item 2. The
// legacy htmx page routers (channels/dashboard/logs/schedule/settings/
// system) were retired in item 10 — the SPA + /api/v1 supersede them.
pub mod recordings;
pub mod websub;
