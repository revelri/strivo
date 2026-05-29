# Recording-dispatch inventory

Inputs for task #5 (RecordingService API design). Captures every site that
constructs `RecordingCommand::*` or the IPC `ClientMessage` variants that
the daemon translates into one. Cookies / output-path / title logic
documented per-site so the service extraction can collapse the
divergences explicitly.

Generated 2026-05-29 against the post-fold-in workspace. Excludes test
fixtures.

## Type surface

```rust
// src/recording/mod.rs:48
pub enum RecordingCommand {
    Start  { channel_id, channel_name, display_name: Option<String>,
             platform, transcode, cookies_path: Option<PathBuf>,
             stream_title: Option<String>, from_start,
             job_id: Option<Uuid>, thumbnail_url: Option<String> },
    Stop   { job_id },
    StopAll,
    DownloadVod { url, channel_name, platform,
                  output_path: PathBuf, cookies_path: Option<PathBuf>,
                  post_title: Option<String> },
}

// src/recording/mod.rs:794
pub fn build_output_path(config: &AppConfig, channel_name: &str,
                         platform: PlatformKind,
                         stream_title: Option<&str>) -> PathBuf;
```

## Start sites (4 — soon 3 after TUI deletion)

| # | Site | Trigger | cookies_path | display_name | from_start | job_id | thumbnail_url |
|---|---|---|---|---|---|---|---|
| 1 | `src/monitor/mod.rs:316` | Channel goes live + `auto_record` true | `self.get_cookies_path(ch.platform)` | `Some(ch.display_name)` | `true` | `None` | `ch.thumbnail_url` |
| 2 | `src/tui/mod.rs:551` | TUI Detail-pane `r`/`R` key (DYING) | per-platform `match { YT/Patreon → config, _ → None }` | from `AppAction` | from key (`r`/`R`) | `None` | `None` |
| 3 | `src/recording/schedule.rs:265` | Cron schedule fired | `None` | `None` | `false` | `Some(pre-generated)` | `None` |
| 4 | `crates/strivo-web/src/routes/api.rs:701` | `POST /api/v1/recordings` | **`None` (!)** | from request body | from request body | `None` | from request body |

**Divergences:**
- **Cookies:** the monitor pulls them via a platform-aware helper; the
  TUI hand-rolls a `match` per call site; the webui API passes `None`
  and trusts the daemon to *not* fill them in (the daemon currently
  doesn't on `Recording(Start)` — only on the high-level
  `DownloadVod` / `PatreonPull` envelopes). Net: a webui-initiated
  start of a gated YouTube stream will fail where a TUI start
  succeeds.
- **thumbnail_url:** monitor and webui pass it through; TUI / schedule
  pass `None`. The recording pipeline snapshots the URL to the local
  cache at start, so omitted thumbnails leave the SPA card blank.
- **display_name:** schedule path passes `None` and falls back to
  `channel_name` for slugs. Cosmetic, not load-bearing.

## DownloadVod sites (5 — soon 4 after TUI deletion)

| # | Site | Trigger | url | output_path | cookies_path |
|---|---|---|---|---|---|
| 1 | `src/monitor/patreon.rs:173` | Patreon poll finds a new gated post | `embed_url` from post | `build_output_path(creator, Patreon, Some(post_title))` | `None` (relies on Patreon client session) |
| 2 | `src/daemon.rs:876` (PatreonPull translator) | `ClientMessage::PatreonPull` from webui | passed in | `build_output_path(creator_name, Patreon, Some(post_title))` | `config.patreon.cookies_path` |
| 3 | `src/daemon.rs:912` (DownloadVod translator) | `ClientMessage::DownloadVod` from webui | passed in | `build_output_path(channel_name, platform, post_title.as_deref())` | per-platform `match { YT → yt cookies, Patreon → pat cookies, _ → None }` |
| 4 | `src/recording/vod_backfill.rs:100` | Twitch VOD backfill task after a live capture | from Twitch GQL VOD | **`vod_output_path(&req.live_output_path)` — bespoke, NOT `build_output_path`** | `None` |
| 5 | `src/tui/mod.rs:711` | TUI Patreon post pull (DYING) | `embed_url` from post | `build_output_path(...)` | **`None`** (vs daemon's PatreonPull which fills from config) |

**Divergences:**
- **Two output-path generators.** `build_output_path` for 4 of 5 sites;
  `vod_output_path` (a `<base>_vod.<ext>` rename of the live file path)
  for VOD backfill only. Backfill's intent is "co-locate with the live
  capture under a `_vod` suffix" — the others build from
  `config.recording_dir` + slug. This is a different naming policy,
  not a bug; the service API should surface it as an explicit
  `OutputPathPolicy { ::Fresh | ::AdjacentTo(live_path) }` rather than
  letting each call site pick.
- **Cookies for the same Patreon post** differ depending on which
  process originated the pull:
  - Monitor (in-daemon polling): `None`
  - Daemon (translating webui `PatreonPull`): `config.patreon.cookies_path`
  - TUI (legacy): `None`
  The monitor relies on the Patreon HTTP client's in-memory session;
  the daemon-translator path doesn't share that session and needs the
  cookie file. A service API can paper over this by always handing the
  recording engine the cookie *resolver* (closure) instead of a path.

## IPC ClientMessage variants that fork to DownloadVod

```rust
// src/ipc.rs (excerpts)
BulkDownload   { channel_id, channel_name, platform, action, playlist_id }   // routed to bulk.rs
PatreonPull    { embed_url, creator_name, post_title }                       // daemon.rs:865
DownloadVod    { url, channel_name, platform, post_title }                   // daemon.rs:887
Recording(RecordingCommand)                                                  // direct passthrough
```

Plus `ClientMessage::Recording(RecordingCommand::DownloadVod { … full struct … })` (the duplicate wire shape flagged in the adversarial review).

**Same intent, two envelopes:**
- `ClientMessage::DownloadVod` — client sends minimal fields; daemon
  computes `output_path` and `cookies_path`. Used by webui (`api.rs:2148`).
- `ClientMessage::Recording(RecordingCommand::DownloadVod)` — client
  sends a fully-populated `RecordingCommand`. Used by the legacy TUI.

After TUI deletion the duplicate wire shape can be removed — but the
daemon-side translation in `daemon.rs:912` (currently called
*only* for `ClientMessage::DownloadVod`) is the canonical path and
should be the spine of the service.

## Stop / StopAll (no divergence — informational)

| Site | Caller | Notes |
|---|---|---|
| `src/recording/schedule.rs:223` | Schedule timer expires | Targets the pre-generated `job_id` stored at Start time |
| `src/app.rs:2546, 3305` | TUI (DYING) | StopAll via `q` confirmation, Stop via row action |
| `crates/strivo-web/src/routes/api.rs:854, 874` | `DELETE /api/v1/recordings/{id}` and the all-stop endpoint | Pass-through |

No service redesign needed here; the surface is already minimal.

## Implications for the RecordingService API (task #5)

1. **One canonical translator** for `(url | channel_id, platform, …)
   → RecordingCommand`. The daemon's `ClientMessage::DownloadVod`
   handler at `daemon.rs:887-919` already has the per-platform cookie
   match and the `build_output_path` call; promote that to a free
   function `intents::download_vod(spec, &AppConfig) -> RecordingCommand`
   and call it from every site.
2. **Output-path policy is a parameter**, not a per-site choice. Make
   `OutputPathPolicy::Fresh { date_slug } | ::AdjacentTo(PathBuf)`
   explicit; VOD backfill picks the latter, everyone else the former.
3. **Cookies are a resolver closure**, not a path. The Patreon monitor
   and the daemon-translator paths can both pass
   `|cfg| -> Option<PathBuf>` that does the right thing for their
   process context; the engine doesn't know the difference.
4. **`thumbnail_url` policy** lives in the service too: monitor and
   webui always pass it; schedule has no source for it (live capture
   only — backfill snapshot happens after the fact); webui-API
   `start_recording` already plumbs it from the request body.
5. **Drop `ClientMessage::Recording(RecordingCommand::DownloadVod)`**
   as part of the TUI deletion. Keep only `ClientMessage::DownloadVod`
   and its sibling `PatreonPull` envelopes; both call into the
   canonical translator.

That is the input the next task (#5) needs to design the service signatures.
