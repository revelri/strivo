# StriVo Roadmap

Single source of truth for what is shipped, what is next, and what is
explicitly out of scope. Absorbs the previous `TODOS.md` and `DESIGN-TODOS.md`
planning state. The user-facing companion is
[CHANGELOG.md](./CHANGELOG.md) (semver release notes only).

References to `DESIGN.md`, `YAZI-AUDIT.md`, and `REVIEW.md` below point to
internal design notes (visual spec, TUI best-practice audit, adversarial
framework review respectively) that are kept out of the public tree.

## Quick roadmap

- **Shipped (0.1 → 0.3):** Twitch / YouTube / Patreon monitoring and
  recording, ratatui TUI with sidebar / channel detail / recording browser /
  settings / wizard / themes, daemon mode over Unix sockets, first-party
  plugins (Crunchr transcription, Archiver gallery), dynamic cdylib plugin
  loader.
- **Next (0.4 → 0.5):** Recording durability (journal + crash-recovery for
  in-flight ffmpeg processes), Crunchr retry / cancellation, full settings UI
  coverage, command palette and unified keymap dispatcher, plugin ABI
  versioning handshake.
- **Vision:** A mature, composable terminal DVR for live streams with a
  small, well-defined plugin surface and a complementary *arr-style web UI
  that talks to the same daemon socket.

**Status legend:** ✅ shipped · 🟡 in progress · ⬜ planned · ⏸ deferred (with reason)

---

## Shipped (0.1.0 → 0.3.0)

### 0.1.0 (2026-03-14) — initial release
- TUI dashboard: sidebar, channel detail, recording list, settings, log, status bar
- First-run setup wizard
- Twitch (OAuth device flow), YouTube (Data API v3 + cookies), Patreon (membership API)
- FFmpeg-based recording, MKV output, optional transcode
- Filename templates, auto-record per channel, cron schedules (backend)
- mpv playback (pipe streaming), streamlink + yt-dlp resolution
- Desktop notifications on go-live
- TOML config with XDG paths, OS keyring credentials
- Daemon/standalone modes, Unix-socket IPC, systemd unit generation
- CLI: `config {list,get,set,path,reset}`, `log {tail,path,clear}`, `daemon {start,stop,status,install}`

### 0.2.0–0.3.0 — Tier 1 UI/UX + P0/P1 quality (2026-04-19)
- Home/End nav across all panes; help overlay (F5, `t`, `R`, `g/G/Home/End`, Esc semantics)
- Esc precedence: clear filter → navigate back; `[/query] N/M · Esc clears` indicator
- Cursor-editable search input; `status_message` actually renders in hotkey bar with 5s auto-dismiss
- Quit-during-recording modal with live seconds + per-job ✓ checklist
- Daemon disconnect banner + auto-reconnect supervisor (1/2/5/10/30 s backoff)
- In-TUI device-code wizard; `AppAction::OpenUrl` cross-platform (xdg-open/open/start)
- Credential leak fix (`config get` refuses `*_secret`/`*_token`/etc.)
- Keyring SPOF: `STRIVO_*` env fallback with once-warned log
- Filename collision: `_N` (1..999) then UUID fallback
- CI on self-hosted runner; 10 integration tests (config roundtrip, filename collision, IPC handshake); 72 tests total green
- OAuth refresh-on-401 for Twitch/YouTube/Patreon
- Rate-limit backoff via shared `parse_retry_after` honoring `Retry-After` + `Ratelimit-Reset`
- Pre-record disk-space gate (≥5 GB via statvfs)
- Retry-exhaustion error surface (`rec.job.error` + `RecordingFinished`)
- Daemon socket hygiene: `sweep_stale_files`, pid+socket unlink on shutdown
- Standalone `PollNow` via `Arc<Notify>` from `ChannelMonitor::poll_notify()`
- Stale-PID detection: `kill(pid,0)` + actual `connect(2)` cross-check
- Config corruption recovery: `.backup` fallback, quarantine, defaults
- Transcode-mode persistence through Settings + `t` hotkey

### Theming pipeline & animation (2026-04-20 closing sprint)
- User-authored `~/.config/strivo/themes/*.{toml,conf}` (Kitty/Ghostty `.conf` parser + `strivo theme import`)
- `ThemeRef` enum: legacy string + rich-table forms (`#[serde(untagged)]`)
- `[theme.colors]` / `[theme.ansi]` overlay overrides
- Runtime theme switching: `Ctrl+T` picker overlay, live preview, Enter commits, Esc reverts via `Theme::snapshot`/`restore`, `R` rescans
- 13 built-in themes (Neon, Neon-HC, Neon-Light, Gruvbox Dark, Rose Pine Moon, Nord, Dracula, Kanagawa, Everforest Dark, …)
- Animation infrastructure: `FrameClock`/`Tween`/`Ease` at 60 fps, `STRIVO_REDUCE_MOTION` + `[ui] reduce_motion` honored everywhere
- Motion catalog: pane focus ramp (180 ms dim→primary), unfocused fade (120 ms), REC dot pulse, LIVE/REC badge breathing, ResolvingUrl braille spinner, Stopping `◼↔◻` crossfade, Failed `✗` breathing, recording heartbeat `●↔◉`, overlay enter ramps (help/quit/properties/wizard/platform-debug/stopping), toast three-phase alpha, hotkey shimmer, search cursor opacity blend, daemon reconnect banner, thumbnail-container crossfade, day-header gradient rule
- Color audit: zero named `Color::*` in `src/`; all RGB usage is legitimate math
- Adaptive polling: 16 ms while animating, 120 ms idle via `needs_fast_frame()`

### Catalog & recording (2026-05)
- Channel back-catalog pull pipeline with crash recovery (`strivo pull <target> [--format|--since|--max|--force|--no-transcribe]`)
- `strivo doctor` external-tool verification
- `strivo completions`, `strivo man`

---

## M1 — Feature Completion (0.4.0)

Theme: finish features that already half-exist. Back-to-front per phase.

### Phase 1 — Backend gaps

- ⬜ **Recording durability journal** — in-flight + scheduled jobs live in RAM. Daemon OOM forgets active recording metadata (file survives). Add SQLite/JSON journal replayed on startup.
- ⬜ **Patreon parity** — token refresh, backoff, dedupe missing. Generalize into shared `OAuthClient` trait. *Files:* `src/platform/patreon.rs`, `src/monitor/patreon.rs`
- ⬜ **Archiver job persistence** — `update_job` is `#[allow(dead_code)]` at `strivo-plugins/src/archiver/db.rs:115`. Either wire it through or remove. Same for `get_channel_stats`.
- ⬜ **Crunchr semantic search backend** — `SearchMode::Semantic` tab is a stub. Either feature-flag off or land fastembed-rs / OpenAI-embeddings backend with sqlite-vss. *Files:* `strivo-plugins/src/crunchr/types.rs:40–59`, `strivo-plugins/src/crunchr/mod.rs`
- ⬜ **Crunchr retry + cancellation** — one transient API timeout kills the job; once started it can't be aborted. Add `CancellationToken` per job + backoff retry (3 attempts, 5/10/30 s). Adopt yazi's cooperative-cancellation idiom: long inner loops poll the token between chunks. *Files:* `strivo-plugins/src/crunchr/mod.rs:163–289`. *(YAZI-AUDIT §12 — internal note)*
- ⬜ **Archiver durability** — same cancellation-token discipline applied to the back-catalog pull loop in `strivo-plugins/src/archiver/downloader.rs`; ties into the recording journal above. *(YAZI-AUDIT §12 — internal note)*
- ⬜ **Crunchr token counting** — `words / 0.75` is wrong for code and non-English. Use `tiktoken-rs`. *Files:* `strivo-plugins/src/crunchr/pipeline.rs:171–174`
- ⬜ **Stream URL validation before ffmpeg launch** — HEAD the URL or parse streamlink exit codes distinctly so stale HLS manifests don't yield cryptic ffmpeg errors. *Files:* `src/stream/resolver.rs`
- ⬜ **Monitor first-poll race** — 10 s timeout can fire concurrently with auth. Gate poll on *auth-notified OR (timeout AND auth-present)*. *Files:* `src/monitor/mod.rs:62–112`
- ⬜ **Plugin shutdown error surfacing** — `src/tui/mod.rs:41` swallows results. At minimum log + toast.

### Phase 2 — Middle (state / event / scheduling)

- ⬜ Wire cron `ScheduleManager` → `AppState`; emit `ScheduleFired` event consumable by TUI + notifications
- ⬜ Watch history persistence → `~/.local/state/strivo/watched.json`
- ⬜ In-memory event ring buffer (last 100 user-facing events, distinct from trace log) — feeds Phase 3 event log pop-over
- ⬜ Selection-by-ID in RecordingList (Sidebar already does this at `src/app.rs:448–456`; mirror)

### Phase 3 — TUI surfaces for existing backend

- ⬜ **Schedule pane** — list with next-fire times, add/edit/delete dialogs writing back to `config.toml`; "next scheduled" indicator in Sidebar
- ⬜ **Recording management** — `v` multi-select, `D` delete-to-trash (`~/.local/share/strivo/trash/`, 7-day TTL), `Enter` metadata pane (codec, bitrate, size, start/end), `shift+r` rename, `shift+m` move. Selection state shape: `IndexMap<RecordingId, u64>` (insertion order + microsecond timestamp tiebreaker) mirroring yazi's `Selected`. *(YAZI-AUDIT §11 — internal note)*
- ⬜ **Playback overlay (mpv)** — `[⏸ 1:23 / 5:45  1.0x  vol 80%]`; `Space` pause, `<`/`>` speed, `j/k` seek ±10 s, `u` resume-from-last-position. Backend exposed in `src/playback/mod.rs`, never rendered.
- ⬜ **Live log tail** — subscribe TUI to a `tracing_subscriber` layer; mirror events into in-memory ring
- ⬜ **Event log pop-over** (`Shift+E`) — last 100 user-facing events with timestamp/level/source
- ⬜ **Setup wizard credential validation** — "Test connection" pass after config changes so stale creds surface immediately

---

## M2 — Cohesive Settings Suite (0.5.0)

**Goal:** every config field reachable from the TUI; every TUI toggle persisted; consistent edit/commit/reset UX.

### Phase 1 — Audit & schema
- ⬜ Enumerate all ~67 fields across 15 structs in `src/config/`
- ⬜ Tag each as `{exposed, hidden, derived, secret}`; emit a coverage report doc
- ⬜ Decide persistence split: `config.toml` (user-authored) vs `~/.local/state/strivo/state.json` (TUI-managed: watched flags, selection, search history, last-used-theme). Documented in a short ARCHITECTURE.md follow-up.
- ⬜ **Defaults-as-preset** — defaults live in code as a `Default` struct; user TOML is a strict overlay (not a full file). "Reset to defaults" rewrites the overlay back to empty. Optional follow-up: split `[[auto_record_channels]]` / `[[schedules]]` into prepend/append vecs so future official additions don't force user-file rewrites. *(YAZI-AUDIT §10 — internal note)*

### Phase 2 — Settings tab redesign
- ⬜ Hierarchical groups: **Recording / Archiver / Crunchr / Notifications / Output / Theme / Keymap**
- ⬜ Inline editors per type: `bool` (toggle), `enum` (cycle / picker), `int` (numeric), `path` (text + validator), `string`, `secret` (masked, with reveal-on-hold)
- ⬜ Per-row validation + reset-to-default
- ⬜ Live preview vs commit-on-save policy: match theme picker (snapshot on enter, Esc reverts)
- ⬜ Reset-all-to-defaults action behind confirm dialog

### Phase 3 — Backfill config↔TUI gaps
Config fields with no TUI surface today:
- ⬜ `[recording]` — codec, bitrate, quality, temp_dir
- ⬜ `[archiver]` — enabled, source_dir, db_path, watch_interval, concurrent_downloads, retention_days
- ⬜ `[crunchr]` — enabled, whisper_model, chunk_size, analysis_enabled, openrouter_key (masked)
- ⬜ `[output]` — notifications_enabled, log_level

TUI controls deliberately *not* backed by config (decide explicitly):
- ⬜ Search filter — ephemeral (decided: not persisted)
- ⬜ Sidebar column order / sorting — punt until needed

---

## M3 — Cohesive Keymap (0.5.0, alongside M2)

**Goal:** one keymap source of truth, no per-pane drift, room for remap (deferred but unblocked).

### Phase 1 — Centralize *(highest-leverage adoption from the yazi audit)* *(YAZI-AUDIT §2 — internal note)*
- ⬜ New module `src/tui/keymap.rs` with `KeyAction` enum + binding table; chord struct analogous to yazi's `Chord { on, run, desc }`
- ⬜ Each pane consumes `(ActivePane, KeyEvent) → Option<KeyAction>` via lookup, not match arms
- ⬜ Replace scattered match arms in `src/app.rs::handle_key`
- ⬜ Key parsing helper accepts `<C-s>` / `<S-Tab>` syntax (mirrors `yazi-config/src/keymap/key.rs`) so future user-remap TOML is straightforward

### Phase 2 — Audit & best-practice pass
- ⬜ Universal: `hjkl` + arrows, `g/G` + Home/End on every navigable pane
- ⬜ Reserve single-letter alphas for pane-local actions; collisions caught at table-build time
- ⬜ Modifier discipline: Ctrl for global (`Ctrl+T` theme, `Ctrl+P` palette, `Ctrl+/` help), Alt unused, Shift for inverse
- ⬜ Universal: `/` search · `?` help · `:` command palette (new — see M4)
- ⬜ Document precedence: overlay > plugin > pane > global

### Phase 3 — Conflict / coverage verification
- ⬜ Build-time check: dedupe `(pane, key, modifiers)`; fail if duplicate (yazi's prepend/append dedup hook is the reference shape)
- ⬜ **Help overlay auto-generated from the keymap table** — three columns (key / action / desc), pane-aware so "what keys work right now?" stops being a hand-maintained string. *(YAZI-AUDIT §8 — internal note)*

### Phase 4 — Foundation for remap (deferred from Tier 4)
- ⬜ Lookup-driven dispatch means `~/.config/strivo/keybindings.toml` becomes a config overlay rather than a rewrite — `prepend_keymap` / `keymap` / `append_keymap` exactly as yazi does it

---

## M4 — Yazi-grade TUI Polish (0.6.0)

Driven by findings in YAZI-AUDIT.md. Adversarial framework review (ratatui vs opentui) in REVIEW.md — verdict: stay on ratatui; this milestone is what unlocks the polish that previously felt blocked.

### Phase 1 — Substrate (the two big structural lifts)
- ⬜ **Async task manager** — new `src/tasks/` module: `TaskId`, `TaskKind` (Record / Transcode / ArchiverPull / CrunchrAnalyze / ThemeImport), `Progress` enum with byte-based percent. Per-task `CancellationToken`. Right-side or status-bar tasks pane: `[2 active · Crunchr 73% · Pull 1.2 GB/3.4 GB · ETA 2m]`. Subsumes the half-built progress spinners and gives notifications a clean event source. *(YAZI-AUDIT §4 — internal note)*
- ⬜ **Input modes** — `InputMode { Normal, Visual, Insert }` distinct from `ActivePane`. Stateless enum; transitions live in the keymap dispatcher from M3. Visual mode is the home for multi-select (M1 Phase 3 graduates into Visual). *(YAZI-AUDIT §1 — internal note)*

### Phase 2 — Discoverability
- ⬜ **Command palette** (`:`) — input widget parses a string into the same `KeyAction` enum from M3. Keys and commands dispatch through one path. *(YAZI-AUDIT §3 — internal note)*
- ⬜ **Marks / registers** for channels — `'a` jumps to mark; persisted to `~/.local/state/strivo/marks.json` as `BTreeMap<char, ChannelId>`.
- ⬜ **Fuzzy finder upgrade** — `src/search.rs` returns score + highlight spans (via `nucleo` or `skim`); match-cache parallel to yazi's `Finder.matched` so `n` / `N` jump-to-next is O(1); field filters (`date:`, `channel:`, `duration:`, `size:`); sort by relevance. *(YAZI-AUDIT §7 — internal note)*

### Phase 3 — Preview pipeline
- ⬜ **Lazy preview lock + spawn-cancel** — debounce/cancel previous preview job on Sidebar selection change; one `tokio::spawn` per preview, results back via a render lock. *(YAZI-AUDIT §6 — internal note)*
- ⬜ **Hover / preview pane** — channel preview (thumbnail + last-N stream meta), recording preview (codec/bitrate/duration + first-frame thumbnail via FFmpeg-extract), schedule preview ("if this cron fires, next 5 dates").

### Phase 4 — Plugin system maturation
- ⬜ **Plugin manifest format** — `[plugin]` section per crate declaring name, version, hotkeys, pane preferences. Lets users introspect installed plugins without grep. *(YAZI-AUDIT §5 — internal note)*
- ⬜ **Plugin discovery** — scan `~/.config/strivo/plugins/*.toml` for declared out-of-tree plugin paths. (Lua not adopted; Rust crates remain the plugin substrate.)
- ⬜ Lifecycle hygiene — adopt yazi's hook pattern (run on completion + cancellation) so plugin shutdown errors are surfaced rather than swallowed.

### Phase 5 — Render-loop micro-optimization
- ⬜ **Batched event draining** — when daemon emits a burst (`ChannelsUpdated` + `RecordingProgress` + …), drain via `try_recv()` until empty, then redraw once. Mirrors yazi's `recv().await` + drain loop. *(YAZI-AUDIT §9 — internal note)* (Other render-loop ideas — partial render, synchronized output — skipped; see audit §9 rationale.)

### Phase 6 — Polish (the loose-end pile)
- ⬜ Notifications extended beyond go-live: recording-complete, schedule-fired, transcription-done, disk-low
- ⬜ Clipboard / open-folder: `y` copy (via `arboard`), `o` open URL in Detail, `O` open recording folder in RecordingList
- ⬜ Theme picker swatch shows palette + theme name on hover (already shows hex codes)
- ⬜ Respect `NO_COLOR` env (monochrome) and `NO_MOTION` (alias for `STRIVO_REDUCE_MOTION`)
- ⬜ Undo buffer — last 5 destructive actions (stop-recording, clear-log, toggle-auto-record); `u` in-memory, cleared on quit

---

## M5 — Killer-app wedges (0.7.0+)

Pick one or two per minor release; ordered by leverage.

1. ⬜ **Clip export from Crunchr timeline** — `c` on a transcript chunk → `ffmpeg -ss/-to -c copy` into `clips/`. Highest leverage; data already exists.
2. ⬜ **Transcript-scoped mpv seek** — Enter on a chunk opens mpv at `--start=<sec>`. Turns StriVo into grep-to-watch.
3. ⬜ **Auto-chaptering (MKV chapters from Crunchr topics)** via `mkvpropedit`.
4. ⬜ **Thumbnail grid in recording list** — `ratatui-image` already a dep; `Picker` already wired.
5. ⬜ **Stream gap detection / resume** — yt-dlp `--live-from-start --wait-for-video` + append MKV segments on drop.
6. ⬜ **Cost display for Crunchr** — OpenRouter / Mistral spend per recording.
7. ⬜ **OBS / Streamlink config import** — one-command onboarding for users with existing configs.

### Web UI (parallel track)
- 🟡 **`strivo serve` *arr-style web UI** — developed in worktree at `../StriVo-webui` on `feat/webui`. Axum + Askama + HTMX, talks to the existing daemon via IPC. Default bind `127.0.0.1:8989`. Mirrors M2 settings groups; `/api/v1/*` JSON API with `X-Api-Key` for external automation.

---

## Cross-cutting / infrastructure

- ⬜ **Shared `PlatformBase` / `OAuthClient` trait** — centralize refresh / backoff / 401 handling across Twitch/YouTube/Patreon
- ⬜ **Single SQLite handle per plugin** — Archiver + Crunchr each open their own connection; FTS + analysis writes serialize. Use `r2d2-sqlite` or async wrapper.
- ⬜ **FTS snippet rendering for recording search** — `snippet(chunks_fts, …)` exists in Crunchr; wire same treatment for `src/search.rs` via file-metadata FTS
- ⬜ **Sidebar filter rebuild race** — audit `search_filtered_channels` rebuild on every channel mutation for edge cases
- ⬜ **Error surface design** — distinct info/warn/error in status bar; persistent error panel; ties into M1 Phase 2 event ring + M4 event log pop-over
- ⬜ **Testing harness** — fake ffmpeg binary, wiremock Twitch/YouTube, tmp-socket daemon harness, `insta` snapshot tests over `ratatui::buffer::Buffer`. Each is a separate milestone.
- ⬜ **Observability** — recording count, bytes written, failure rate, auth-refresh rate, last-poll-at per platform. Expose via `strivo status` (CLI) and optional Prometheus text endpoint.
- ⬜ **ARCHITECTURE.md** — daemon vs standalone topology, plugin ABI, keybinding cheat sheet, troubleshooting (keyring / socket), schedule TOML format
- ⬜ **Platform CI coverage** — Windows (`win11-ci` runner) and macOS (`macos-sonoma` runner) folded into workflow alongside the Linux self-hosted runner
- ⬜ **Workspace clippy cleanup** — ~20 pre-existing warnings outside sprint-touched code; CI currently gates `--all-targets` on root crate only

---

## Deferred / non-goals (with reasons)

### Motion / animation (closed sprint decisions)
- ⏸ Per-row selection animation (sidebar/recording-list/settings/log) — requires either abandoning `ratatui::List` (manual render of all rows) or threading per-row animation state maps. Scope creep for a subtle effect; revisit if sidebar is rewritten.
- ⏸ Alpha-blend overlay backdrops — ratatui renders full cells; no true cell alpha. Would require every widget to accept a dim factor; ~2× render cost.
- ⏸ Pane slide-in / cyan underline slide — layout::render isn't a pane-router; faking offsets requires tweening `Rect.x`. Border ramps already communicate focus.
- ⏸ Wizard fade-out — overlay close path has no tail state; would need `Option<Instant>` for close timestamps.
- ⏸ Toast queue — single-slot overlap is rare; refactoring ~30 `status_message = …` write sites isn't worth the gain.
- ⏸ Viewer-count sparkline — monitors keep only the latest snapshot; needs a polling history buffer at the monitor layer.
- ⏸ Log smooth-scroll / log-level crossfade — ratatui renders at integer cells; sub-cell scroll needs a separate line buffer. Severity rarely updates mid-stream.
- ⏸ Launch / shutdown choreography — terminal init/restore are synchronous; animating would delay restore behind a tween. Minor UX gain.
- ⏸ Loading skeletons, ASCII empty states, keystroke echo, launch spinner, transcoding donut, log heatmap, audible bell, clipboard auto-copy in wizard — each documented as low leverage for the implementation cost; see git history of DESIGN-TODOS.md.

### Feature
- ⏸ **Mouse support** — ratatui enables it easily but policy unclear (always-on? opt-in flag?). Defer pending posture decision.
- ⏸ **Display density toggle** (Compact/Normal/Spacious) — one-day knob; not blocking anyone.
- ⏸ **Config reload without restart** (`Ctrl+R`) — re-read + diff is easy; restarting monitor poll on interval change is the hard part.
- ⏸ **Light-mode theme audit** — neon-light shipped; full contrast audit pending.
- ⏸ **Twitch EventSub / YouTube push subscriptions** — polling is fine for the channel counts users actually have; webhooks would mean inbound HTTP, which conflicts with the local-only posture (revisit if web UI changes that).
- ⏸ **Theme dir file-watch** (`notify` crate) — manual `R` rescan in picker is acceptable.

---

## Phase release sequencing

```
0.3.0 → M1 (0.4.0) → M2 + M3 (0.5.0) → M4 (0.6.0) → M5 wedges (0.7.0+)
                                              │
                                              └── webui parallel track on feat/webui
```

Each milestone closes with: green CI, CHANGELOG entry, README status refresh.
