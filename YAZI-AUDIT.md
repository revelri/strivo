# YAZI-AUDIT — Gold-Standard TUI Patterns vs. StriVo

Reference repo: `sxyazi/yazi` (commit at `/tmp/yazi`, shallow clone). yazi is a Rust + ratatui terminal file manager with a mature multi-mode UI, Lua plugin runtime, async task system, and which-key help. This audit extracts patterns worth adopting in StriVo and maps each to a concrete StriVo file:line — either the existing equivalent (to refactor) or the absence (to build).

For each pattern: **what yazi does (with file:lines)**, **StriVo today**, **verdict** (Adopt / Adapt / Skip), **target milestone** in [ROADMAP.md](./ROADMAP.md).

---

## 1. Input mode state machine

**yazi** — `yazi-widgets/src/input/mode.rs:1-12`
Three-state enum `InputMode { Normal, Insert, Replace }`. Stateless data; transitions are event-driven externally. `delta()` returns cursor offset for the current mode.

**StriVo** — `src/app.rs::ActivePane` is pane-focused, not mode-focused. There is no "visual" / "select" mode; multi-select on RecordingList does not exist (planned in M1 Phase 3). The search input has implicit text mode driven by `search_active` bool.

**Verdict — Adopt (M4).** Introduce `InputMode { Normal, Visual, Insert }` distinct from `ActivePane`. Visual mode is the home for multi-select on lists and inverts certain bindings (`d` becomes "delete selection" instead of "delete current"). Keep modes stateless; transitions live in the keymap dispatcher (see §2).

---

## 2. Keymap definition, dispatch, and conflict prevention

**yazi** — `yazi-config/src/keymap/`
- `chord.rs:13-98` — `Chord { on: Vec<Key>, run: Vec<Cmd>, desc, for_ }`. Every binding has structured action vec + description; help text is field-derived (no second table).
- `key.rs:1-98` — `Key` with shift/ctrl/alt/super flags. `FromStr` parses `<C-s>`, `<S-Tab>`; OS-specific shift normalization at lines 44–47.
- `rules.rs:11-53` — `KeymapRules<const L: u8>` holds `prepend_keymap`, `keymap`, `append_keymap`. Custom `DeserializeOverHook` (lines 35–52) deduplicates by key signature (`!a_seen.contains(&on(v))` at line 47). Prepend wins; later layers filtered.
- `keymap.rs:9-36` — Layer enum (mgr / tasks / spot / pick / input / confirm / help / cmp). `Keymap::get(layer)` returns a chord slice for active context.

**StriVo** — Scattered `match` arms in `src/app.rs::handle_key`; pane-specific blocks; help overlay text is hand-written. No central table. No build-time conflict check. Per-pane drift is currently held in check by author discipline, not structure.

**Verdict — Adopt (M3).** This is the single biggest leverage point. Build `src/tui/keymap.rs` with:
- A `KeyAction` enum (the analog of yazi's `Cmd`).
- A binding table per layer: `(Layer, Key) → KeyAction`, with `desc: &'static str`.
- Layer precedence: `overlay > plugin > pane > global` (yazi's layer hierarchy mapped onto StriVo's overlay/plugin/pane structure).
- Build-time / startup-time dedupe so a duplicate `(layer, key)` is a hard error.
- Auto-generate the help overlay from the table (yazi: `yazi-fm/src/help/bindings.rs:13-51` renders three columns key/action/desc — directly cribbable).

User keybinding remap (currently deferred under "Tier 4") becomes free once the table exists — `~/.config/strivo/keybindings.toml` is just a `(Layer, Key) → action_name` overlay. M3 Phase 4.

---

## 3. Command palette (`:`)

**yazi** — `yazi-shared/src/event/cmd.rs:1-57`
`Cmd { name, args: HashMap<DataKey, Data> }`. `FromStr` parses shell-like syntax: command name + positional args + `--flag=value`. Commands and keybinding actions share the same `Cmd` type — `:reload-config` is identical to whatever key fires it.

**StriVo** — No command palette. There is no `:` prompt.

**Verdict — Adopt (M4).** Once §2 lands and key actions are typed (`KeyAction`), the palette is a thin input widget that parses strings into the same `KeyAction` enum and dispatches them. The discoverable command surface justifies the effort even if power users will still use keys.

---

## 4. Async task manager

**yazi** — `yazi-scheduler/src/`
- `task.rs:6-48` — `Task { id, title, prog, hook, done, logger }`. Tasks are immutable records; progress is a copyable enum updated via channel.
- `scheduler.rs:1-100` — Worker threads served from priority queues (HIGH/LOW). `add()` / `add_hooked()` (lines 28–50); `cancel(id)` via `CancellationToken` (lines 53–60). Hooks run on completion or cancellation.
- `progress.rs:5-52` — `Progress` trait with `running/cooked/success/failed/cleaned/percent`. Variants (`FileCopy`, `FileDelete`, …) at lines 57–79 carry their own state.

**StriVo** — Long-running ops are scattered:
- `src/recording/mod.rs` runs FFmpeg via `tokio::process`
- `strivo-plugins/src/archiver/downloader.rs` runs the back-catalog pull
- `strivo-plugins/src/crunchr/mod.rs:163-289` runs transcription with no cancellation, no retry
There is no unified "tasks" view; `DaemonEvent::RecordingFinished` is the only completion signal users see.

**Verdict — Adopt (M4 + lights M1 Phase 1).** Build `src/tasks/` with:
- `TaskId`, `TaskKind` (Record, Transcode, ArchiverPull, CrunchrAnalyze, ThemeImport)
- `Progress` enum analogous to yazi's, including byte-based percent for transfers
- `CancellationToken` per task (closes the M1 "Crunchr retry + cancellation" gap)
- A right-side or status-bar tasks pane: `[2 active · Crunchr 73% · Pull 1.2 GB/3.4 GB · ETA 2m]`
This subsumes the current half-implemented progress spinners and gives notifications a clean event source.

---

## 5. Plugin system

**yazi** — `yazi-config/src/plugin/plugin.rs:11-70`
Four plugin types (fetchers / spotters / preloaders / previewers). Each supports prepend/append merging (same dedup pattern as keymaps). Limits enforced at deserialization. Lua bridge in `yazi-plugin/src/standard.rs:11-75` — two-stage init (globals, then user `init.lua`). Plugins are eagerly loaded; no hot reload.

**StriVo** — Plugin trait in `src/plugin/`; concrete plugins (Crunchr, Archiver) ship as a sibling submodule `strivo-plugins/` with a path dep. No manifest format, no discovery beyond compile-time linking. Plugin shutdown errors swallowed (`src/tui/mod.rs:41`).

**Verdict — Adapt (M4).** Don't add Lua — StriVo's plugins are Rust crates and the API is small. But adopt:
- **Manifest format** — a `[plugin]` section per crate declaring name, version, hotkeys, pane preferences. Lets users see what they have installed without grep.
- **Discovery** — scan `~/.config/strivo/plugins/*.toml` for declared plugin paths (out-of-tree plugins).
- **Lifecycle hygiene** — yazi's hook pattern (run on completion + cancellation) maps to plugin shutdown. Fix the swallowed errors.

Lua is the wrong choice for StriVo (binary size, security boundary with platform APIs). Skip that part.

---

## 6. Preview pipeline

**yazi** — `yazi-core/src/tab/preview.rs:15-100` + `yazi-fm/src/mgr/preview.rs:1-31`
Lazy: triggered by cursor move. Mime-based dispatcher selects a previewer; result returned via render lock. `tokio::spawn()` per preview; previous job cancelled when cursor advances. Folder previews chunked (50k items per 500 ms batch).

**StriVo** — `ratatui-image::Picker` is wired and used for channel thumbnails on Detail (single render path, not lazy). There is no preview of recordings beyond the metadata pane (planned).

**Verdict — Adapt (M4).** Wire a lazy `PreviewLock` for:
- Channel preview (already mostly there; debounce/cancel on Sidebar selection change)
- Recording preview pane (codec/bitrate/duration + first-frame thumbnail via FFmpeg-extract)
- Schedule preview ("if this cron fires, next 5 dates")

The yazi `render lock + spawn` pattern is the right shape; lift the structure but keep StriVo-specific previewers.

---

## 7. Fuzzy finder / find

**yazi** — `yazi-core/src/tab/finder.rs:8-98`
`Finder { filter, matched: IndexMap, lock }`. `new()` builds a regex filter from the user string + case mode. `next()`/`prev()` (lines 29–50) scan visible files; `catchup()` rebuilds match cache on folder change (lines 53–75). Caches up to 99 matches.

**StriVo** — `src/search.rs` returns bool only; no score, no highlight spans. Sidebar / RecordingList / Detail all reuse the same predicate. Filter indicator landed in Tier 1.

**Verdict — Adopt (M4).** Upgrade `src/search.rs` to:
- Score-based ranking (existing fuzzy crates: `nucleo` or `skim`)
- Highlight spans returned alongside score
- Field filters (`date:`, `channel:`, `duration:`, `size:`) — already enumerated in ROADMAP M4
- Match cache parallel to yazi's `Finder.matched` so `n`/`N` jump-to-next is O(1)

---

## 8. Help / which-key

**yazi** — `yazi-fm/src/help/{help,bindings}.rs`
`Help::tips()` (`help.rs:14-19`) reads the help layer keymap and surfaces the filter binding hint. `Bindings::render()` (`bindings.rs:13-51`) renders three columns: key / action / desc, all sourced from the `Chord` fields. Sort config-driven (`yazi-config/src/which/which.rs`).

**StriVo** — Help overlay is hand-written in `src/tui/widgets/dialog.rs` and `src/app.rs`. Drift risk: add a binding and forget to update help.

**Verdict — Adopt (M3 Phase 3).** Once §2 lands, the help overlay is **generated** from the keymap table at build/render time. Free correctness, free new-binding documentation, free pane-aware help ("which keys work right now?").

---

## 9. Render loop & event architecture

**yazi** — `yazi-fm/src/app/app.rs:36-95`
- `tokio::select!` between render timer and event channel.
- Batched event draining (lines 67–77): `recv().await` then `try_recv()` loop until empty.
- Re-render scheduled with 10 ms debounce (line 88); skipped if last render < 10 ms ago.
- Two-tier render: full vs partial (notifications/progress only, line 54).
- Synchronized terminal update + collision detection (`render.rs` line 4, lines 30/38/80–94).

**StriVo** — `src/tui/mod.rs` adaptive frame rate (16 ms while animating, 120 ms idle via `needs_fast_frame()`, **B.10/P6 in old DESIGN-TODOS**). Crossterm events polled with the same `poll_duration()`. No partial-render distinction; full draw every frame.

**Verdict — Skip wholesale; cherry-pick batched draining (M4).** StriVo's adaptive cadence is already in the right ballpark for our load — we don't redraw a 50k-file directory. Batched event draining is the one piece worth lifting: when the daemon emits a burst of `ChannelsUpdated` + `RecordingProgress`, we should drain all of them before redrawing once. Partial render isn't worth the complexity at our scale.

---

## 10. Config layering

**yazi** — `yazi-config/src/yazi.rs:8-27`
Root config aggregates sections (mgr, preview, opener, …). `Yazi::read()` loads `~/.config/yazi/yazi.toml`. The `DeserializeOver` macro provides hierarchical merge: defaults compiled in as presets, user TOML overrides, optional in-session overrides via Lua. Three-way merge (prepend/base/append) replaces normal inheritance.

**StriVo** — Single `config.toml`. `AppConfig::load` falls back to `.backup` → defaults (Tier 1 P1 shipped). No prepend/append; no compiled-in preset that the user can extend.

**Verdict — Adapt (M2).** StriVo doesn't need yazi's full DeserializeOver machinery, but the **defaults-as-presets** pattern resolves the M2 Phase 1 question about reset-to-default: defaults live in code, user config is a strict overlay, "Reset" rewrites the overlay back to empty. Independently, `[[auto_record_channels]]` and `[[schedules]]` are natural prepend/append candidates — splitting a `defaults.toml` shipped with the binary out of the user file would let us add new official channels without forcing user-file rewrites. Optional.

---

## 11. Marks / registers / bookmarks (selection state)

**yazi** — `yazi-core/src/tab/selected.rs:9-80`
`Selected { inner: IndexMap<UrlBufCov, u64>, parents }`. Hierarchy-aware (parents count child selections). Timestamp-based insertion order (`timestamp_us()` at line 65) for deterministic order. `add()`/`add_many()`/`remove()` at lines 25–80.

**StriVo** — No multi-select today. RecordingList is single-select. ROADMAP M1 Phase 3 plans `v` multi-select.

**Verdict — Adopt (M1 Phase 3 + M4).** Reuse yazi's `IndexMap + timestamp` shape for the multi-select. For marks proper (jump-to-channel-via-mark, `'a` = my-favorite-streamer), a separate `BTreeMap<char, ChannelId>` persisted to `~/.local/state/strivo/marks.json` works; M4 territory.

---

## 12. Spotting / on-demand background analysis

**yazi** — `yazi-runner/src/spot.rs:17-68`
`Runner::spot()` spawns a plugin per file on demand. Cancellation token checked every 2000 Lua instructions (cooperative cancellation). Errors logged, task cancelled gracefully (lines 56–60).

**StriVo** — Closest equivalent: Crunchr running on a recording. Currently uncancellable; ROADMAP M1 Phase 1 closes that.

**Verdict — Adopt the cancellation pattern (M1 Phase 1).** Per-task `CancellationToken`; long loops in Crunchr / Archiver poll the token between chunks (this is the Rust equivalent of yazi's 2000-instruction hook). Closes "Crunchr retry + cancellation" and "Archiver durability".

---

## Summary: adoption roadmap

| § | Pattern | Verdict | Target |
|---|---|---|---|
| 1 | Input modes (Normal/Visual/Insert) | Adopt | M4 |
| 2 | Centralized keymap table | **Adopt — highest leverage** | M3 |
| 3 | Command palette (`:` + shared action enum) | Adopt | M4 |
| 4 | Async task manager + Progress trait | Adopt | M4 (lights M1) |
| 5 | Plugin manifest + discovery (no Lua) | Adapt | M4 |
| 6 | Lazy preview lock + spawn-cancel | Adapt | M4 |
| 7 | Fuzzy finder with score + cache | Adopt | M4 |
| 8 | Help auto-generated from keymap | Adopt | M3 Phase 3 |
| 9 | Batched event draining | Adopt (only) | M4 |
| 10 | Defaults-as-preset + overlay | Adapt | M2 |
| 11 | Multi-select via IndexMap + timestamp | Adopt | M1 Phase 3 |
| 12 | Cooperative cancellation tokens | Adopt | M1 Phase 1 |

The two big structural lifts — **§2 keymap table** and **§4 task manager** — unblock most of M3, M4, and the still-pending M1 cancellation work. Everything else is incremental.
