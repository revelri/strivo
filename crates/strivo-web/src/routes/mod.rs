pub mod api;
pub mod assets;
pub mod events;
pub mod login;
// Retained but unmounted: the sole recording file-serving path (download/
// play) plus the path-containment guard + tests from roadmap item 2. The
// legacy htmx page routers (channels/dashboard/logs/schedule/settings/
// system) were retired in item 10 — the SPA + /api/v1 supersede them.
pub mod recordings;
pub mod websub;
