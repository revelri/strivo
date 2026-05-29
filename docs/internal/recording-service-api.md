# RecordingService API design

Companion to [`recording-dispatch-inventory.md`](./recording-dispatch-inventory.md).
The inventory enumerated 4 Start sites + 5 DownloadVod sites with three
divergences (output-path policy, cookies resolution, thumbnail
plumbing). This doc defines the API that collapses them and the
migration plan that gets every site onto it.

Implementation is task #6.

## Module layout

New module `src/intents/` (sibling to `src/recording/`, `src/platform/`).
Keeps the recording engine free of the "where did this intent come from
and which config keys does it need" logic, which is the actual lift.

```
src/intents/
├── mod.rs           // re-exports + module doc
├── start.rs         // start_recording(spec, &AppConfig) -> RecordingCommand
├── download_vod.rs  // download_vod(spec, &AppConfig)    -> RecordingCommand
├── spec.rs          // StartSpec, DownloadVodSpec, OutputPathPolicy
└── cookies.rs       // CookieSource enum + resolver
```

Public surface: a single `pub use intents::{start_recording,
download_vod, StartSpec, DownloadVodSpec, OutputPathPolicy};` from
`lib.rs`.

## Spec types

```rust
// src/intents/spec.rs

/// Everything a caller wants to *say* about a new live capture. The
/// service translates this into a fully-populated `RecordingCommand`,
/// applying config-derived defaults (cookies, transcode policy,
/// output-path slugging).
#[derive(Debug, Clone)]
pub struct StartSpec {
    pub channel_id: String,
    pub channel_name: String,
    pub display_name: Option<String>,
    pub platform: PlatformKind,
    pub stream_title: Option<String>,
    pub thumbnail_url: Option<String>,

    /// `true` = ask the platform driver to record from t=0 (Twitch
    /// Rewind, YouTube "live from start"). Honoured when the platform
    /// supports it; ignored otherwise.
    pub from_start: bool,

    /// Pre-generated UUID. `None` lets the recording manager pick one.
    /// The schedule path uses `Some` so it can correlate the timed
    /// `Stop` with the eventual `RecordingStarted` event.
    pub job_id: Option<Uuid>,

    /// Override the per-platform transcode default from config.
    /// `None` = use `config.effective_transcode(platform, channel_id)`.
    pub transcode_override: Option<bool>,

    /// Where cookies come from for gated streams. See `CookieSource`.
    pub cookies: CookieSource,
}

/// Everything a caller wants to *say* about pulling a VOD or gated
/// post. The service applies the same config-derived defaults as
/// `StartSpec` plus the output-path policy.
#[derive(Debug, Clone)]
pub struct DownloadVodSpec {
    pub url: String,
    pub channel_name: String,
    pub platform: PlatformKind,
    pub post_title: Option<String>,
    pub cookies: CookieSource,
    pub output_policy: OutputPathPolicy,
}

/// Where the downloaded file ends up on disk.
#[derive(Debug, Clone)]
pub enum OutputPathPolicy {
    /// Build from `config.recording_dir` + slug. Used by Patreon
    /// monitor, daemon translator for `PatreonPull`/`DownloadVod`, the
    /// catalog-pull bulk download path. The default.
    Fresh,

    /// Co-locate with an existing live capture's file under a `_vod`
    /// suffix (`<base>.<ext>` → `<base>_vod.<ext>`). Used only by
    /// `vod_backfill` after a live record finishes.
    AdjacentTo(PathBuf),
}
```

## Cookies — resolver, not a path

```rust
// src/intents/cookies.rs

/// Where the engine should source cookies. Three call patterns:
/// - `Inherit`: this caller already holds a live HTTP session (e.g.
///   the Patreon monitor). No cookies file needed; the engine skips
///   `--cookies` and relies on the caller's adapter.
/// - `FromConfig`: pull the per-platform cookies path from `AppConfig`
///   (the common case for daemon-translator and webui-initiated
///   pulls).
/// - `Explicit`: an exact `PathBuf` the caller already resolved.
#[derive(Debug, Clone)]
pub enum CookieSource {
    Inherit,
    FromConfig,
    Explicit(PathBuf),
}

impl CookieSource {
    pub fn resolve(&self, config: &AppConfig, platform: PlatformKind)
        -> Option<PathBuf>
    {
        match self {
            Self::Inherit => None,
            Self::FromConfig => match platform {
                PlatformKind::YouTube => config.youtube.as_ref()
                    .and_then(|y| y.cookies_path.clone()),
                PlatformKind::Patreon => config.patreon.as_ref()
                    .and_then(|p| p.cookies_path.clone()),
                _ => None,
            },
            Self::Explicit(p) => Some(p.clone()),
        }
    }
}
```

This replaces the three-different-ways-to-resolve-cookies problem
(monitor `None`, daemon-translator `config.x.cookies_path`, TUI
hand-rolled match) with one enum the caller picks once.

## Service signatures

```rust
// src/intents/start.rs
pub fn start_recording(spec: StartSpec, config: &AppConfig) -> RecordingCommand;

// src/intents/download_vod.rs
pub fn download_vod(spec: DownloadVodSpec, config: &AppConfig) -> RecordingCommand;
```

Both are pure functions: no IO, no channels, no async. They return a
`RecordingCommand` the caller hands to its existing `recording_tx`. This
keeps the boundary trivial — sites that today call `recording_tx.send(...)`
become `recording_tx.send(intents::start_recording(spec, &config))`.

A future iteration can move the `recording_tx.send` itself into the
intents module behind a `RecordingService` struct holding the channel,
but that's a refactor over a refactor — punt to follow-up.

## IPC wire-shape decision

The dispatch inventory found two competing envelopes for "download a
VOD":

```rust
// (1) Used by legacy TUI
ClientMessage::Recording(RecordingCommand::DownloadVod { ... full struct ... })

// (2) Used by webui
ClientMessage::DownloadVod { url, channel_name, platform, post_title }
```

(2) wins. The webui-shaped envelope is minimal (no client-side path
or cookie resolution), the daemon translates it through the new
`intents::download_vod`, and the result lands in the same recording
engine queue. (1) is removed as part of TUI deletion (task #13);
nothing else uses it.

`ClientMessage::PatreonPull` stays — it's a higher-level intent
("pull this specific Patreon post we saw in the SPA"), not a duplicate.
Its daemon-side handler will switch to `intents::download_vod` with
`OutputPathPolicy::Fresh` and `CookieSource::FromConfig`.

## Migration plan (task #6 order)

Land the module first, then migrate sites in order of risk (lowest →
highest). Build green at each step.

| Order | Site | Spec construction | Notes |
|---|---|---|---|
| 1 | `intents/` module itself | n/a | Add module + types + tests, no callers yet |
| 2 | `daemon.rs:887` `ClientMessage::DownloadVod` | `DownloadVodSpec { ..., CookieSource::FromConfig, OutputPathPolicy::Fresh }` | Replaces the hand-rolled match. Smallest blast radius — one daemon arm. |
| 3 | `daemon.rs:865` `ClientMessage::PatreonPull` | `DownloadVodSpec { platform: Patreon, ..., CookieSource::FromConfig, OutputPathPolicy::Fresh }` | Same shape as (2); now both daemon translators share one engine call. |
| 4 | `monitor/patreon.rs:173` auto-pull | `DownloadVodSpec { ..., CookieSource::Inherit, OutputPathPolicy::Fresh }` | First non-daemon caller; verifies `Inherit` works end-to-end. |
| 5 | `recording/vod_backfill.rs:100` | `DownloadVodSpec { ..., CookieSource::Inherit, OutputPathPolicy::AdjacentTo(req.live_output_path) }` | Replaces the bespoke `vod_output_path` helper. Delete that helper once green. |
| 6 | `monitor/mod.rs:316` auto-record-on-live | `StartSpec { ..., CookieSource::FromConfig }` | First `start_recording` caller. Bumps the webui-API parity ahead of time. |
| 7 | `recording/schedule.rs:265` cron-fired Start | `StartSpec { ..., CookieSource::FromConfig, job_id: Some(pre-generated) }` | Schedule already has the job_id wiring; spec just makes it explicit. |
| 8 | `crates/strivo-web/src/routes/api.rs:701` `POST /api/v1/recordings` | `StartSpec { ..., CookieSource::FromConfig }` | **Fixes the current bug**: gated YouTube starts via webui currently pass `cookies_path: None`. |
| 9 | `crates/strivo-web/src/routes/api.rs:2148` `DownloadVod` endpoint | (handled by daemon arm at step 2; no per-site change here) | The route just builds the `ClientMessage::DownloadVod` envelope; daemon translates. |
| 10 | TUI sites `src/tui/mod.rs:551, 711` | n/a | **Do NOT migrate.** Both die with task #13 TUI deletion. Migrating them is wasted work. |

Each step is one commit, runs `cargo check --workspace --offline` +
`cargo test -p strivo-core --lib` clean before the next.

## What the service does NOT own (yet)

- **The `recording_tx` channel itself.** Sites still call
  `recording_tx.send(intents::start_recording(spec, &config))`. Owning
  the channel means owning the lifetime, which means the service
  becomes a struct passed into every caller — that's a separate
  refactor and not required for correctness.
- **Stop / StopAll.** Two sites total, both pass-through, no
  divergence. Touching them buys nothing.
- **BulkDownload.** Routed through `recording/bulk.rs` which has its
  own queueing semantics. Not in the duplicate-translator bucket the
  audit flagged. Out of scope.
- **Recording-engine internals.** `run_manager`, `build_output_path`,
  `episode_dir`, `sanitize_path_component`, the ffmpeg/streamlink/yt-dlp
  drivers — all unchanged. The service is a translator layer
  *above* the engine, not a rewrite of it.

## Tests

`src/intents/start.rs` and `download_vod.rs` both ship with unit
tests covering:

- `CookieSource::FromConfig` resolves YouTube/Patreon paths from config.
- `CookieSource::Inherit` returns `None`.
- `CookieSource::Explicit(p)` returns `Some(p)` regardless of platform.
- `OutputPathPolicy::Fresh` calls `build_output_path` with the right slug.
- `OutputPathPolicy::AdjacentTo(p)` produces `<base>_vod.<ext>` and rejects
  paths without an extension (existing `vod_output_path` behaviour).
- `start_recording` honours `transcode_override`; `None` defers to config.
- `start_recording` propagates `from_start`, `thumbnail_url`, `job_id`,
  `display_name` verbatim.

Integration: after step (8) of the migration, a webui-initiated start
of a configured-with-cookies YouTube channel succeeds in `cargo test
-p strivo-web --test routes`. (Webui-bug fix verification.)

That is the design. Task #6 lands the module + walks the migration table top to bottom.
