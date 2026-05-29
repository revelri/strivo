# StriVo Roadmap

StriVo is a TUI Live-Stream PVR for Twitch and YouTube whose Pro tier ships
a complete DAW-equivalent post-production toolkit, delivered as a swarm
of pure-data Rust plugin crates wired through a web SPA + a daemon over
IPC.

This document records every shipped milestone, the remaining vision gaps,
and the concrete TODOs that future iters should pick up. **Status legend:**
✅ shipped · 🟡 in progress · ⬜ planned · ⏸ deferred (with reason).

---

## Shipping plugins (35 in-tree)

Every plugin below ships in `crates/<name>/` as a pure-data Rust crate
(no IO outside the host) with its own unit tests. The host
(`crates/strivo-web/`) wires each one to a Pro-gated HTTP endpoint and
surfaces it on the SPA.

### Capture · transcribe · catalog
| Plugin | Crate | What it does |
|---|---|---|
| **Crunchr** | `strivo-plugins/crunchr` | Whisper transcription, diarisation, topic segmentation, word timestamps, click-to-seek transcript, speaker filter, exports |
| **Archiver** | `strivo-plugins/archiver` | Back-catalog VOD archiver; per-channel auto-pull; tandem with `monitor.auto_download` |
| **Viewguard** | `strivo-plugins/viewguard` | Live fraud-signal scoring during captures |
| **Insights** | `strivo-plugins/insights` | Cross-stream word frequency, topic shifts, retention proxy |

### Cut-discovery
| Plugin | Crate | What it does |
|---|---|---|
| Chapters | `crates/chapters` | Heuristic chapter generation from pacing |
| Cuepoints | `crates/cuepoints` | Scene-change detection via `ffmpeg select` |
| Clipper | `crates/clipper` | Highlight detection + clip extraction |
| Thumbnails | `crates/thumbnails` | Frame ranking + facecam crop |
| Insights-compare | `crates/insights-compare` | Stream-vs-stream side-by-side + retention proxy |
| Heatmap | `crates/heatmap` | Multi-signal retention overlay |
| Viewguard-trend | `crates/viewguard-trend` | Cross-stream fraud trend dashboard |
| Chat-density | `crates/chat-density` | Audience-retention proxy from chat rate |
| Broll | `crates/broll` | B-roll suggestion from transcript topics |

### Editor stack (the DAW core)
| Plugin | Crate | What it does |
|---|---|---|
| EDL editor | `crates/editor` | Non-destructive split / ripple-delete / fades + revision history with `save_with_label` for full undo across saves |
| Dead-air | `crates/deadair` | ffmpeg silencedetect → recommend trim cuts |
| Branding | `crates/branding` | Watermark + intro/outro banner overlay → ffmpeg `filter_complex` |
| Automation | `crates/automation` | DAW volume automation with Step/Linear/Cosine curves, baked via `asendcmd` |
| Loudness | `crates/loudness` | EBU R128 two-pass parser with platform presets (YouTube/Spotify/Apple/EBU/Twitch) |
| Captions | `crates/captions` | SRT/VTT/TXT + **styled ASS** with per-speaker colour + karaoke `\k` tags |
| Multitrack | `crates/multitrack` | Audio track enumeration + extraction |
| Brandsafe | `crates/brandsafe` | Pre-publish content classifier |
| Structure | `crates/structure` | DAW section labeler (intro/gameplay/break/outro) from chapters + chat + cues |
| Beat-detect | `crates/beat-detect` | Onset picker + autocorrelation BPM from `astats` envelope |
| VAD | `crates/vad` | Hysteresis noise gate + auto-tighten ripple-delete recs |
| Scenes | `crates/scenes` | DAW session save/recall bundling every plugin's per-recording state into a SQLite-backed manifest |
| Sidechain | `crates/sidechain` | DAW sidechain compressor — VAD voice intervals → ducking automation curve baked via the volume-automation render path |
| Insert FX | `crates/insert-fx` | DAW-style ordered insert chain (HP / NR / de-esser / comp / limiter / reverb / EQ) per recording with voice + game bus presets, composes into one ffmpeg `-af` baked at render |
| Pitch / time | `crates/pitch` | Independent pitch + tempo via ffmpeg `rubberband`; fit-to-duration helper for publish-slot mapping, formant-preserving by default for voice |

### Publish · view · meta
| Plugin | Crate | What it does |
|---|---|---|
| Reuse | `crates/reuse` | Cross-format publish-queue drafter |
| Casebook | `crates/casebook` | Post-stream markdown briefing |
| Multistream | `crates/multistream` | Auto-tile multi-stream viewer (Twitch + YouTube embeds) |
| Chat | `crates/chat` | Chatterino-class Twitch IRC + filter pipeline + Twitch emote + BTTV global emote rendering |
| Pipelines-DAG | `crates/pipelines-dag` | Cross-plugin pipeline graph |
| Marketplace | `crates/marketplace` | Third-party plugin manifest spec + catalog stub (15 entries shipped) |
| Schedule-optimizer | `crates/schedule-optimizer` | Publish-slot recommender → 7×24 grid → top weekly times with confidence + plateau coverage |

---

## Shipped surfaces (UI + UX)

### Top-bar routes (11)
| Route | What's there |
|---|---|
| `/library` | Live-channel rail + capture dashboard + first-run hint |
| `/recordings` | Sortable + filterable table with state chips, group-by-channel, persistent bulk-action bar, file-error remediation (Re-scan + Show path), play/info/delete buttons in slot 1 |
| `/schedule` | Monitor: record-when-live + auto-download + capture-limit safety knobs + disk-free gauge + status banner |
| `/pipelines` | Clickable DAG nodes routing to each plugin + per-pipeline "Run on…" recording picker + readiness chip |
| `/watch` | Multi-stream auto-tile / focus / PiP modes; per-tile solo-audio + fullscreen; 30s viewer-count refresh |
| `/chat` | Twitch IRC over WSS with filter chips, mention highlighting, Twitch native emote rendering, BTTV global emote rendering |
| `/plugins` | Capability-matrix + marketplace catalog + first-party cards + per-card ⚙ deep-link to Settings → Plugins |
| `/settings` | 8 sections; per-plugin enable/disable manager; notifications panel; monitor limits; replay-tour + reset-hints |
| `/system` | Health + Network + Storage + Platform Auth + Backup + Blocklist + Tasks |
| `/logs` | Tail-follow + regex filter + level chips + copy + download |
| `/history` | Per-row Play/Info/Delete + filter + state chips + group-by-channel/date |

### Editor topbar workflow (14 buttons)
`Split at time… · Ripple-delete range… · ▢ Trim dead air… · ▢ Voice gate… · 🦆 Sidechain duck… · 🎛 Insert FX… · 🎚 Pitch/time… · ★ Branding… · ♪ Loudness… · 🎼 Beat grid… · ↺ History… · 🎬 Scenes… · ♪ I/TP/LRA gauge · ⚡ Render to MKV`

### Plugin sub-routes
- `#/plugins/crunchr` + `#/plugins/crunchr/rec/<id>` — Pro upsell when not entitled
- `#/plugins/archiver` + `#/plugins/archiver/<channelId>`
- `#/plugins/viewguard`
- `#/plugins/insights`
- `#/plugins/schedule-optimizer` — 7×24 heatmap + top-pick cards

### Onboarding
- 8-step welcome tour on first paint (spotlight pins to each topnav slot)
- Per-page hint banner (one-line tip per route, dismissible, persisted)
- Settings → Interface → Onboarding has Replay tour + Reset hints buttons

---

## Audit catalogue — fully shipped ✅

Every catalogue item from the comprehensive E2E audit has landed.

- **iter 25** plugin hub capability-matrix status fixes + chip spacing
- **iter 26** Pro-gate UX — upsell card replaces raw 402 JSON dump
- **iter 27** candy icons for watch / chat / history topnav slots
- **iter 28** Settings → Notifications + Monitor limits + General at-a-glance
- **iter 29** Recordings table — persistent bulk bar + state chips + group-by-channel
- **iter 30** History — per-row actions + filter + state chips + group-by-channel/date
- **iter 31** file-error pill — hatched red + Re-scan + Show path actions
- **iter 32** Schedule — capture-limits card + status banner + disk gauge
- **iter 33** Pipelines — clickable nodes + readiness chip + Run-on-recording picker
- **iter 34** Watch — solo-audio + per-tile fullscreen + 30s viewer-count refresh
- **iter 35** Chat — Twitch native emote + BTTV global emote rendering + image-badge attempt
- **iter 36** Logs — follow + regex + copy/download
- **iter 37** Plugin enable/disable manager with 25-row grid by category
- **iter 38** Onboarding tour + per-page hint banners

## DAW-vision iters — shipped ✅

- **iter 21** Branding — watermark + intro/outro banner → ffmpeg filter chain
- **iter 22** EDL revision history — DAW-style undo across saves
- **iter 23** Multistream viewer — auto-tile + Focus + PiP layout modes
- **iter 24** Chat client — Chatterino-class IRC + tokenizer + filter pipeline + ring buffer
- **iter 39** Loudness — EBU R128 normalisation with 5 platform presets
- **iter 40** Structure — DAW section labeller (intro/gameplay/break/outro tiling)
- **iter 41** Automation — DAW volume curves baked at render (`asendcmd` + Step/Linear/Cosine)
- **iter 42** Styled ASS captions — per-speaker colour + karaoke `\k` highlight
- **iter 43** Scenes — DAW session save/recall bundling every plugin state
- **iter 44** Schedule-optimizer — DAW launch-quantize for publish slots
- **iter 45** Beat detection — onset picker + BPM autocorrelation
- **iter 46** VAD / noise gate — hysteresis gate + auto-tighten ripple-delete recs
- **iter 47** SPA voice-gate one-click workflow in EDL editor topbar
- **iter 48** SPA scene-snapshot panel (capture / restore / delete inline in editor)
- **iter 49** SPA schedule-optimizer page — 7×24 heatmap + top-pick cards
- **iter 50** Sidechain compressor — VAD intervals → ducking automation via the existing `asendcmd` volume-automation render path (no new ffmpeg plumbing)
- **iter 51** SPA `🦆 Sidechain duck…` one-click — VAD → sidechain → automation composed in a single editor-topbar gesture
- **iter 52** Insert FX chain — 9-variant typed effect model + voice/game bus presets; ordered chain composes into one ffmpeg `-af` baked at render
- **iter 53** Pitch / time-stretch — independent tempo + semitone shift via `rubberband`; `fit_to_duration` helper maps a raw stream to a publish slot without changing voices' pitch

---

## Test inventory

Total pure-data unit tests across in-tree plugin crates (excluding the
`strivo-plugins` submodule which has its own suite):

| Crate | Tests |
|---|---|
| `automation` | 14 |
| `beat-detect` | 12 |
| `brandsafe` | 10 |
| `branding` | 16 |
| `broll` | 11 |
| `captions` | 18 (was 9; +9 ASS) |
| `casebook` | 11 |
| `chapters` | 5 |
| `chat` | 24 |
| `chat-density` | 14 |
| `clipper` | 8 |
| `cuepoints` | 5 |
| `deadair` | 12 |
| `editor` | 22 |
| `heatmap` | 10 |
| `insights-compare` | 10 |
| `loudness` | 12 |
| `marketplace` | 15 |
| `multistream` | 18 |
| `multitrack` | 8 |
| `pipelines-dag` | 10 |
| `reuse` | 12 |
| `scenes` | 12 |
| `schedule-optimizer` | 13 |
| `structure` | 12 |
| `thumbnails` | 8 |
| `viewguard-trend` | 13 |
| `vad` | 12 |
| `sidechain` | 12 |
| `insert-fx` | 14 |
| `pitch` | 15 |
| **Total** | **~386 unit tests** |

All green at the time of merge. Both feature modes (`pro` + `--no-default-features`) build clean.

---

## Marketplace catalog

18 entries shipped (`crates/marketplace/src/lib.rs::default_catalog()`):

✅ Installed Cdylib (16): branding · multistream · chat · deadair · twitch-chat-density · broll-finder · loudness · structure · automation · scenes · schedule-optimizer · beat-detect · vad · sidechain · insert-fx · pitch

🗺 Roadmap (2): `demucs-split` (needs external `demucs` binary) · `yt-publish` (needs YouTube OAuth + API creds)

---

## Capability matrix

Per `GET /api/v1/plugins/capabilities`:

- Every shipped plugin marked `available` (was: most marked `roadmap` despite being live)
- Multi-provider rows list every contributor:
  - `audience_retention` → heatmap + chat-density
  - `stream_comparison` → insights + viewguard-trend
  - `captions` → captions + captions-ass
  - `edl_editor` → editor + deadair + branding + broll
  - `source_track_split` → multitrack + demucs-split (roadmap)
  - `publish_queue` → reuse + yt-publish (roadmap)
- New `x.`-prefixed capabilities from iters 23+: `x.multistream`, `x.chat`, `x.pipelines_dag`, `x.marketplace`, `x.loudness`, `x.structure`, `x.audio_automation`, `x.scenes`, `x.publish_slots`, `x.tempo`, `x.voice_gate`, `x.sidechain`, `x.insert_fx`, `x.pitch_time`

---

## Open vision gaps ⬜

Concrete future iters to keep the cron loop fed.

### Remaining DAW analogues
- **A/B render compare** — render the EDL twice with different filter chains; parse VMAF / SSIM output; produce a diff report. Pure-data parser is fully testable; backend orchestration is heavier (needs two ffmpeg passes). Composes naturally with iter-50 sidechain, iter-52 insert-fx, iter-53 pitch since all three are already typed model+filter slots that can be snapshotted into A and B variants.
- **Sub-mix / bus routing** — route multitrack outputs through a shared sub-mix with shared gain + insert chain. Now that iter-52 ships an InsertChain crate, a SubMix can hold one InsertChain per child track plus a master InsertChain — the filter composer is trivial; the heavy lift is wiring `filter_complex` per-track input mapping in the render path.

Three previously-listed gaps (sidechain, insert effects, pitch/time-stretch) shipped in iters 50 / 52 / 53 respectively.

### Backend integrations that would unblock today's roadmap catalog entries
- **Demucs source separation** — vendor demucs as an optional binary; expose `demucs-split` Cdylib so the catalog entry flips from roadmap to installed.
- **YouTube OAuth + Helix publish** — drives the `yt-publish` catalog entry. Needs the device-code flow + scope `youtube.upload`.
- **Real Twitch badge UUID fetch** — current `chat` plugin badge code falls back to text chips because Twitch CDN requires UUIDs (channel-scoped subscriber badges especially). Wire `/helix/chat/badges/global` + per-channel fetch behind the existing chat plugin.
- **FFZ + 7TV emote integration** — extend the chat tokenizer's emote map. Same pattern as BTTV; just three more endpoints + cache.
- **Chat compose + slash-commands** — OAuth login scoped to `chat:edit`; lets users send messages + `/me` / `/timeout` / `/vip` from inside the SPA.

### Collaboration / multi-user features
- **Per-segment comments** — SQLite-backed comments tied to a recording's timecode. Plus optional WS for live updates so a team can review a stream together.
- **Real-time multi-cursor** — Yjs / Automerge CRDT over EDL state for synchronous editing sessions.
- **Review-request workflow** — share a scene + checkbox approval log.

### SPA-side polish
- **Heatmap row clicks → publish-time recommender deep link** — `audience_retention` row → schedule optimizer pre-loaded with the recording's bucket data.
- **Multistream layout presets** — Quadrant / Highlight / Theatre overrides on top of the existing Auto / Focus / PiP modes.
- **Editor cut → scenes "Capture before" auto-snapshot** — optional "auto-snapshot every save" config so the user always has a pre-edit recovery point.

### Surface gaps from the v1 audit (lower-priority)
- **VOD progress pill polling** — pill renders but the rendering rate could be tightened to match the daemon's SSE cadence
- **Logs date-range picker** — currently the tail returns the last 500 lines; a date filter would let users jump to a specific incident
- **Trace-id linking** — when the host emits structured trace ids, the logs view should make them clickable to filter
- **Settings → Plugins manager actions** — beyond enable/disable, add "Clear stored data" + "View storage size" per plugin
- **Recordings table date range filter** — filter by `started_at` lo/hi to scope the view
- **History date heatmap** — small calendar grid above the History list, click a day to filter

### Operational
- **Per-plugin runtime gate** — `plugin_toggles.<name>.enabled` is currently advisory only. Wire it into the daemon's plugin scheduler so disabled plugins genuinely skip work.
- **Disk-budget circuit breaker enforcement** — `monitor_limits.disk_budget_reserved_gb` is surfaced but not yet wired to defer new captures when crossed.
- **Max-concurrent-recordings enforcement** — same; the field is read, the gauge renders, but the daemon doesn't enforce.
- **License backend integration** — `/licence/trial` + `/licence/activate` return 503 when `STRIVO_LICENCE_URL` isn't set. Future iter brings up the activation backend.
- **Self-hosted CI** — `Chorosyne/strivo` repo has runners for Arch Linux (this host), macOS Sonoma (QEMU VM), Tiny11 (Windows VM). All three currently idle; future iter ships a `release.yml` that bundles per-platform binaries.

### Doc + ops
- **Per-plugin README** — every `crates/<name>` has `lib.rs` doc-comments but no top-level README. A future iter generates per-plugin READMEs from the crate metadata + a structured marketplace manifest.
- **Plugin author guide** — `docs/PLUGIN-MANIFEST.md` covers the manifest spec; needs a companion "writing a plugin" tutorial that walks the directory layout + capability tags.
- **End-user docs** — chorosyne.com still 404s the strivo product page. Future iter ships product copy + screenshots.

---

## Earlier milestones (pre-iter-21 era)

The pre-DAW-vision shipping history covered the foundational TUI + daemon
work. Summary, since it informs why the plugin architecture works the
way it does:

- **0.1.0** (2026-03-14) — initial release. TUI dashboard, first-run wizard,
  Twitch + YouTube + Patreon monitoring, ffmpeg recording, mpv playback,
  desktop notifications, TOML config + OS keyring, daemon mode, systemd unit
  generator, CLI surface.
- **0.2.0 – 0.3.0** (2026-04-19) — Tier-1 UI/UX + P0/P1 quality work. Home/End
  nav everywhere, F5 help overlay, Esc precedence (clear filter → back), live
  search status, quit-during-recording confirmation modal, auto-reconnect
  supervisor with exponential backoff, command palette + unified keymap.

That layer remains intact; the 0.3+ Pro phase added the SPA on top + the
plugin swarm.

---

## Working tree state at this snapshot

- Workspace version bumped 0.3.0 → **0.4.0** to reflect the iter-50–53 DAW
  closeout (sidechain · insert-fx · pitch · the one-click sidechain workflow).
- Branch `feat/strivo-pro-phase1` is fully reachable from `main`; iter
  50+ commits land directly on `main`. Stale `feat/webui` worktree +
  branch pruned at iter 54.
- `strivo-plugins` submodule pinned at `8a06166` (`heads/main`); private repo,
  no pending changes.
- All 35 in-tree Rust crates build clean in both Pro (`--features pro`) and
  free (`--no-default-features`) modes.
- Daemon + serve binary deployed at `~/.local/bin/strivo`; runs unprivileged
  with `entitled:false` by default.
- 16 marketplace entries map to in-tree plugins (Cdylib); 2 to roadmap
  third-party slots.

---

## File map highlights

```
strivo/
├── src/                       # daemon (`strivo daemon`) — IPC, monitor, recording
├── crates/strivo-bin/         # CLI shim
├── crates/strivo-web/         # `strivo serve` SPA host
│   ├── src/routes/
│   │   ├── plugins.rs         # every Pro plugin endpoint
│   │   ├── api.rs             # /settings, /channels, /recordings, capability matrix
│   │   └── licence.rs         # /licence/status, /activate, /trial
│   ├── assets/spa.js          # vanilla SPA (single-file)
│   └── assets/spa.css         # all CSS (single-file)
├── crates/<plugin>/           # 26 in-tree pure-data plugin crates
├── strivo-plugins/            # Git submodule (Crunchr · Archiver · Viewguard · Insights)
├── docs/CODEMAPS/             # per-file route + state codemaps
└── ROADMAP.md                 # this file
```

---

## Conventions

- **Commit prefixes**: `feat:`, `fix:`, `chore:`, `refactor:`, `ci:`, `docs:`, `test:`, `perf:`
- **No AI attribution** in commits, PRs, or code comments (per project CLAUDE.md)
- **Per-iter scope**: one cohesive vertical slice — pure-data crate + tests + backend route + SPA wiring + marketplace + capability matrix + plugin registry + chrome-devtools E2E verify + clean commit
- **Both feature modes verified per iter**: `cargo build -p strivo-web` (Pro) AND `cargo build --no-default-features -p strivo-bin` (free)
- **User state restored after every E2E**: daemon left running, serve restarted without `STRIVO_DEV_UNLOCK_ALL` so `/licence/status` returns `entitled:false`

---

## When in doubt

Pick the smallest cohesive slice that fills a real DAW gap; build the
pure-data crate first with tests; let chrome-devtools E2E surface the
parser / filter / state-machine bugs the unit tests missed; ship in one
commit.
