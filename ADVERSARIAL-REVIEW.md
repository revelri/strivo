# StriVo — Adversarial Design Review

**Date:** 2026-05-29
**Reviewers:** five parallel adversarial agents (UX/UI, routing & business logic, project categorization, user journey, commercial viability)
**Method:** independent investigations of the source tree, docs, roadmap, and licence backend. Each reviewer was instructed to be hostile, cite file:line, and rank findings. No praise sandwich.

---

## TL;DR — One Sentence Per Reviewer

| Axis | Verdict |
|---|---|
| **UX/UI** | A 16-layer modal architecture whose own keymap doctrine breaks on contact with the keymap. |
| **Routing & business logic** | No `RecordingService`; recording intent is rebuilt at 4+ sites, with `Recording(DownloadVod)` and `DownloadVod` as competing wire shapes. |
| **Categorization** | 17 of 38 crates are serde-only "DAW analogues" with no spawning consumer; two competing ROADMAPs; private notes leaked into the public root. |
| **User journey** | The roadmap describes a different product than the README ships; adding a channel needs a numeric ID with no discovery path; failure has no in-product home. |
| **Commercial** | Wrong customer, wrong form factor, wrong moat, wrong pricing, wrong legal posture. As priced today, not viable. |

---

## 1. UX/UI — Modal Sprawl and Self-Contradiction

**The central wound is modal load.** `src/tui/keymap.rs:25-44` defines sixteen layers — Global, Sidebar, Detail, RecordingList, Schedule, Settings, Log, Wizard, StatusBar, ThemePicker, EventLog, PlaybackOverlay, SearchInput, QuitConfirm, PropertiesModal, PlatformDebugModal — plus a Plugin layer that `Layer::for_pane` can't even map (`keymap.rs:60` returns `None`). The active key meaning is `(pane × overlay × visual-mode × playback-overlay × pending_auth)`. The wizard can promote itself to overlay from another pane via `show_wizard = active_pane == Wizard || pending_auth.is_some()` (`layout.rs:78`), so the keymap silently mutates mid-task whenever a device-code flow fires.

**KEYMAP.md's own discipline doesn't survive a 30-second audit.** The doc (`docs/KEYMAP.md:43-54`) forbids pane-local `Ctrl+key` — `keymap.rs:983` binds `Ctrl+V` on RecordingList. `Shift+D` is "trash" on RecordingList, "immediate-delete" on Schedule. `Shift+R` is "record-from-start" on Detail, "rename" on RecordingList. `Shift+P` is "plugin browser" globally but "pick playlist" on Detail (`keymap.rs:763` vs `920`). The doc admits "single-letter d/D divergence is intentional" — that is an apology, not a doctrine.

**Single-letter overload.** `a` = ToggleAutoRecord (Sidebar), Add (Schedule), Actions popup (RecordingList). `s` = Settings (Sidebar), Stop recording (RecordingList). `B` does different bulk-download things on Sidebar vs Detail. Detail alone holds nine pane-local alpha bindings. `?` is the only escape valve and the doc admits per-pane bindings still live in match arms (KEYMAP.md:79–81), so `?` lies by omission.

**Visual hierarchy is asserted, not delivered.** DESIGN.md:9-14 promises "Retro-Futuristic Neon"; the color table (DESIGN.md:84-96) assigns *six* meaningful semantic colors plus three immutable platform brand colors. Red simultaneously means "recording," "destructive," and "error." When the pulsing REC dot sits next to a YouTube red brand glyph next to an error toast, nothing is signal.

**Color-only signaling.** Live/Recording/Warning are color-coded with no mandated glyph fallback in DESIGN.md. ~8% of male users can't reliably distinguish those reds and greens. A `neon-hc` theme exists (DESIGN.md:171) but there's no rule that status must also carry a glyph.

**DESIGN.md vs reality drift.** Two-thirds of DESIGN.md specifies an ElegantFin/Jellyfin SPA theme (glass cards, 1.25em radii, Satoshi 900 hero type) — then DESIGN.md:62-63 shrugs: *"The TUI does not specify fonts."* The design system is for a surface this product mostly isn't.

**Top 3 UX Wounds**
1. **Keymap violates its own written doctrine.** `Ctrl+V` pane-local, `Shift+D` dual-meaning, `a`/`s`/`P`/`b` overloaded per pane. The "remap config (M3.4)" punt is admission the table is unshippable.
2. **Sixteen-layer modal architecture with silent overlay promotion.** `pending_auth` hijacking the wizard mid-pane plus a Plugin layer the help system can't enumerate makes the active keymap unknowable from on-screen state.
3. **FIRST-RUN.md contradicts the keymap on the user's first action.** It says press `A` to add a channel; Sidebar binds lowercase `a` to ToggleAutoRecord and has no `Add Channel` action. A new user's first keystroke arms an auto-record on whatever is highlighted.

---

## 2. Routing & Business Logic — Four Sources of Truth, No Service Layer

**Recording dispatch is forked across four translation sites** that each independently call `recording::build_output_path` and synthesize a `RecordingCommand::DownloadVod`:
- `src/daemon.rs:870` (PatreonPull) and `src/daemon.rs:893` (DownloadVod)
- `src/tui/mod.rs:700` (PullPatreonPost)
- `src/monitor/patreon.rs:167` (auto-pull)
- `src/recording/vod_backfill.rs:100`

`RecordingCommand::Start` is constructed from **four more** sites (`src/tui/mod.rs:546`, `src/monitor/mod.rs:316`, `src/recording/schedule.rs:265`, `crates/strivo-web/src/routes/api.rs:701`). Cookies-path lookup (`daemon.rs:901-911`) is duplicated against config keys with no platform adapter. **There is no `RecordingService`.** Business logic is the union of whatever each entry point remembers.

**IPC contract is a junk drawer.** `ClientMessage` (`ipc.rs:13-98`) holds both typed envelopes (`Recording(RecordingCommand)`) and feature verbs (`PatreonPull`, `DownloadVod`, `BulkDownload`, `DeleteRecording`, `ClearErroredRecordings`, …). `Recording(RecordingCommand::DownloadVod)` and `ClientMessage::DownloadVod` both exist and do almost the same thing — one expects the *client* to compute path+cookies, the other lets the daemon. The webui uses the latter; the TUI uses the former (`tui/mod.rs:706`). **Same verb, two wire shapes**, picked by which frontend you're on.

**Implicit, racing state machines.** `DaemonState` (`daemon.rs:30-150`) is mutated from a broadcast stream while every client and the TUI's `AppState` independently maintain `recordings: HashMap`. `ClearErroredRecordings` (`daemon.rs:976-1029`) literally carries `// Source of truth #1 … #2` comments and ships the union. Persist-journal strings round-trip through `persist::map_journal_state` with no schema versioning. The auth queue (`daemon.rs:119-146`) is an unbounded `VecDeque` with no timeout or cancellation — a stuck `pending_auth` blocks it forever.

**Plugin model = three incompatible contracts.** (a) In-process `Plugin` trait that bakes `ratatui::Frame`, `Rect`, and `crossterm::KeyEvent` into its public surface (`plugin/mod.rs:457-560`) — that's why the daemon had to invent a narrow `VerbContext` (`plugin/mod.rs:453`). (b) IPC `PluginRpc` verbs. (c) **Undocumented SQLite-schema-as-API**: `crates/strivo-web/src/routes/plugins.rs` is **4083 lines** of direct read-only opens against `crunchr.db`, `archiver.db`, `viewguard.db` (30+ call sites). `route()` at `plugins.rs:3819-3886` hand-rolls ~70 per-plugin endpoints. Adding a plugin means editing `strivo-web`. Plugin schemas cannot evolve without breaking the web crate.

**Dual frontend with divergent behavior baked in.** TUI translates intents through `AppAction` (~40 variants in `app.rs:4459+`) and `RecordingCommand`. The SPA (`spa.js`, **11,191 lines**, 199 `fetch` calls) talks to `/api/v1/*` (`api.rs`, **2399 lines**), which re-implements the same intents as HTTP. Result: TUI passes `cookies_path: None` for Patreon pulls (`tui/mod.rs:711`); daemon path looks it up from config (`daemon.rs:883`). Same product behavior, two answers, frontend-dependent.

**`app.rs` is 4639 lines and a god-object.** Owns AppState, AppEvent, AppAction, OverlayKey, palette, theme picker, mpv lifecycle, focus-fade timing, keymap dispatcher, plugin verb hydration, undo stack, preview-probe spawning. `lib.rs` re-exports it so `daemon.rs` depends on it for `DaemonEvent`/`AppEvent`. **The daemon and the TUI share an event type defined in the TUI's monolith.**

**Error propagation.** Most failures are swallowed via `let _ = tx.send(...)` + `tracing::warn!`. The webui's `start_recording` (`api.rs:693`) returns 200 the instant the IPC line is written — no correlation id, no round-trip. The SPA polls SSE and hopes.

**Top 3 Architectural Wounds**
1. **No `RecordingService`.** Intent translation is duplicated at 4+ sites with divergent cookie/path/title logic; two competing wire shapes for "download VOD". This is the codebase's single bug-farm. Fix: collapse to one `intents::start_recording(spec)` in `strivo-core`, called by both daemon IPC and TUI.
2. **Plugin model is three incompatible contracts** (Trait + IPC verb + raw SQLite scrape). `strivo-web/routes/plugins.rs` is a 4083-line schema-scraper. Plugins cannot evolve schemas without breaking the web crate.
3. **`app.rs` is a 4639-line god-object** that both TUI and daemon depend on for event types. Extract `strivo-events` and `strivo-state`; reduce `app.rs` to TUI presentation only.

---

## 3. Categorization & Sprawl — A DAW Wearing a PVR's Skin

**Headline.** This is a TUI Twitch/YouTube PVR (`Cargo.toml:31`) that has metastasized into a 38-crate "DAW-equivalent post-production toolkit" (`ROADMAP.md:3-7`). The TUI promise is now a single-file SPA (`crates/strivo-web/assets/spa.js`, 11k lines). `src/tui/` is a graveyard of widgets while `strivo-web` is 8,537 LOC across 17 files — a ~30× imbalance against the original product surface.

**Crate triage (38 total)**
- **Core / real (≈6, ~16%)**: `strivo-bin`, `strivo-web`, `editor`, `chat`, `multistream`, `marketplace`. Real LOC, real tests, tied to shipped routes.
- **Supporting / plausible (≈15)**: `chapters`, `cuepoints`, `clipper`, `thumbnails`, `captions`, `multitrack`, `loudness`, `automation`, `deadair`, `vad`, `branding`, `heatmap`, `chat-density`, `scenes`, `pipelines-dag`. Each is one file, 300–550 LOC. Fine in isolation, but several could be modules in one `strivo-postprocess` crate.
- **Speculative / DAW cargo-cult (≈17, ~45%)**: `ab-render`, `submix`, `demucs-split`, `beat-detect`, `sidechain`, `insert-fx`, `pitch`, `structure`, `schedule-optimizer`, `viewguard-trend`, `brandsafe`, `broll`, `casebook`, `reuse`, `insights-compare`, `dataviz`. `ab-render` and `dataviz` self-describe as "pure-data: no IO. The host owns the ffmpeg spawn" — typed-struct + filter-string builders with no consumer ever spawning ffmpeg against them. `demucs-split` is a 213-LOC wrapper for a CLI flagged in ROADMAP as "needs external demucs binary" (`ROADMAP.md:201`). They inflate the marketplace count.

**Naming coherence.** A new contributor reading the workspace `members = [...]` (`Cargo.toml:2`) sees `ab-render, beat-detect, brandsafe, broll, demucs-split, schedule-optimizer, sidechain, submix, viewguard-trend, ...` and learns nothing about Twitch/YouTube PVR. Naming is "DAW feature parity," not "what this tool does." `viewguard-trend` and `brandsafe` are domain-specific verbs with no glossary entry.

**Boundary violations.** `src/plugin/registry.rs` and `src/plugin/mod.rs` both import `ratatui`/`crossterm`. The dynamic plugin loader — supposedly transport-agnostic — is TUI-coupled in the core lib. Meanwhile `strivo-web` ships the actual plugin UI. Pick one.

**Docs sprawl.** Two competing roadmaps: `/ROADMAP.md` (335 lines, product-truth) and `/docs/ROADMAP.md` (173 lines, "Web UI Hardening" — completely different content). CHANGELOG `[Unreleased]:Removed` claims `REVIEW.md` and `YAZI-AUDIT.md` are "no longer tracked" — both still sit at repo root. `FOLLOWUP-PLUGIN-WALK.md` and `PLUGINS_PRIVATE.md` are maintainer-private notes leaking into the public tree. `docs/` adds 14 more files including overlapping pairs (`TWITCH-LIVE-FROM-START.md` + `TWITCH-LIVE-FROM-START-INTEL.md`).

**Workspace hygiene.** Version drift in one workspace: `strivo-core` is `0.5.0`; ROADMAP says the workspace bumped to `0.4.0` (`ROADMAP.md:284`); `viewguard-trend`, `brandsafe`, `casebook`, `broll` are `0.3.0` with `publish = false`; `ab-render`, `submix`, `dataviz` are `0.1.0`. Three version cohorts, no policy. `publish = false` only on the older cohort — newer speculative crates are silently publishable.

**10-minute new-contributor verdict.** They conclude this is a partially-abandoned DAW pretending to be a PVR, with a TUI quietly displaced by a single-file SPA, three competing roadmap surfaces, and ~17 "plugin" crates that exist as serde structs awaiting integration that may never come.

**Top 3 Sprawl Wounds**
1. **17 of 38 crates are speculative serde-only DAW analogues** with no real consumer. Merge into 2–3 cohesive crates (`strivo-audio-fx`, `strivo-postprocess`, `strivo-analytics`) or delete.
2. **Two `ROADMAP.md` files + four private notes in the public root.** CHANGELOG claims `REVIEW.md` + `YAZI-AUDIT.md` were removed — they weren't.
3. **TUI–core–SPA boundary collapse.** Product is sold as TUI; engineering reality is a SPA. Either rip `src/tui/` and rename to "StriVo Web," or rip `crates/strivo-web/` back into its lane and finish the TUI. The straddle pays the cost of both and the benefit of neither.

---

## 4. User Journey — The Roadmap Describes a Different Product

A hostile new user trying to record their first Twitch stream walks ~8 hands-on steps + 1 external developer-console detour + 1 numeric-ID lookup that has no in-app affordance.

**Numbered friction**
1. README install assumes Arch; no apt/dnf/nix path.
2. No screenshot or GIF of the wizard.
3. Twitch/YouTube credential setup punts to upstream dev consoles with zero screenshots or "register this redirect URL" guidance.
4. `strivo doctor` output is undocumented — user can't tell pass from fail.
5. **Adding a channel requires the numeric `channel_id`** with no in-app or doc-driven lookup. A user installs strivo to record xqc and has no idea how to translate "xqc" into the numeric ID `config.toml.example` shows.
6. KEYMAP.md is a *conventions* doc, not a key list. The exhaustive list lives in `keymap.rs` or the `?` overlay. You must launch the app to learn the app.
7. Recording failure recovery is "re-run with `-l debug` and read ffmpeg stderr." No in-TUI incident path.
8. README admits in-flight recordings are not durable across daemon crashes; the workaround is "M1/0.4.0" — i.e. not today.
9. README mentions `strivo serve` as if shipped; the webui worktree is stale.
10. **ROADMAP describes a different product** (DAW + 38 plugins + marketplace) than README ships (poll-and-record + mpv playback + two transcription plugins). No "today vs tomorrow" demarcation.
11. Transcription backend choice (5 options, 3 env vars, 2 GPU paths) is gated behind a TOML edit.
12. Schedules require cron syntax authored by hand in TOML.
13. Settings panel is admitted-incomplete via `docs/SETTINGS-COVERAGE.md` — itself a footgun document.
14. Plugin system disclaims itself: "third-party plugins not recommended for end users." Worst of both worlds.
15. Windows table uses ⚠️/❌ without explaining what works in foreground mode.
16. `docs/TWITCH-LIVE-FROM-START.md` etc. are internal engineering scoping documents shipped in `/docs/` — a confused user clicks and lands in GQL persisted-query hashes.
17. There is no "first recording in under 5 minutes" speedrun.
18. README's "OAuth verification page closes immediately" failure mode hints at known UX bugs — buried in FIRST-RUN.md.
19. Keyring fallback to `STRIVO_*` env vars is documented only as a failure-mode block; headless server users have to fail first to learn the supported path.
20. README's stated 0.3.0 known limitations list undercuts the marketing bullets directly above it.

**Top 3 Journey Wounds**
1. **The roadmap is a different product.** Anyone arriving via ROADMAP.md expects an editor with sidechain ducking, B-roll suggestions, marketplace plugins, and a web SPA. What ships is a poll-and-record TUI with two bundled plugins. No honest "today / tomorrow" page.
2. **Adding a channel is identifier-paste with no discovery.** Without translating "xqc" → numeric ID, the product fails its own one-line pitch.
3. **Failure has no in-product home.** When a recording dies, when the daemon crashes mid-stream, when OAuth races, the user is sent to logs and external dev consoles. For a PVR — software whose value proposition is "don't miss the stream" — this breaks trust fastest.

---

## 5. Commercial Viability — There Is No Customer

**The core problem in one sentence.** You built a 38-crate "DAW-equivalent" post-production toolkit attached to a Twitch/YouTube recorder, gated it behind a $25 Lemon Squeezy paywall, and called it a TUI. There is no customer here. There is a portfolio.

**Who pays $25?** Name them. Not a persona — a person.
- **VOD hoarders** already use Streamlink + cron. Free. Will not pay.
- **Streamers doing post-production** use OBS + Resolve/Premiere/Descript. They will not edit clips in a *terminal*. The TUI is actively repulsive.
- **Researchers / journalists** want a CLI, not a paywall. They are ~200 people globally.

There is **no demographic overlap between "lives in a terminal" and "needs styled ASS captions with karaoke `\k` tags."** You've built for the intersection of two non-intersecting circles.

**Market sizing.** Streamlink ~10k stars; yt-dlp ~80k. Willing to pay: 1–2%. Want a TUI specifically: rounding error. Also want a 30-plugin DAW bolted on: zero. TAM ≈ **single-digit thousands of users**; 1% conversion at $25 one-time = **<$2,500 lifetime revenue**. Not a business.

**Competition demolishes the moat.**

| Competitor | What it does | Why they win |
|---|---|---|
| Streamlink + cron | Free, scriptable | Zero switching cost, no licence ping |
| yt-dlp | Free, downloads anything | It's the dependency you shell out to |
| OBS | Local recording, free | Streamers already have it open |
| TwitchDownloader | VOD download GUI, free | Lower friction for hoarders |
| Descript | AI transcription + edit | Has the post-prod buyer you want |

Differentiators are "it's a terminal" and "38 plugins." Neither is a reason to switch. The 38 plugins are a **liability** — buyers see surface area, not value.

**Pricing & moat.** $25 one-time via Lemon Squeezy with a 3-day trial and JWT machine-binding. This is 2014-era shareware pricing. No recurring revenue, no expansion motion, no upsell ladder. The hard parts (ffmpeg, streamlink, yt-dlp, whisper) are FOSS dependencies you shell out to. **You own the glue. Glue is not a moat.** Net ~$20/sale after fees; you need 500 sales to clear $10k and you won't clear 50 through HN alone.

**Distribution dead end.** TUI products get one HN launch post. Ceiling ~200 stars / 20 buyers week one, decaying to zero. TUIs don't screenshot well (the demo `.gif` in your README is a placeholder — itself evidence). You cannot run paid acquisition on a $25 SKU.

**Legal / ToS exposure.** Twitch ToS §8 prohibits scraping/copying/aggregating; YouTube ToS forbids downloads. You're commercializing a tool whose primary function is ToS violation. Risks:
- C&D from Twitch/Google (youtube-dl got DMCA'd in 2020; you'd have less sympathy as a paid product).
- DMCA pass-through for copyrighted music in every Twitch stream.
- Lemon Squeezy will drop you on first credible complaint.
- Patreon integration is the spiciest — paid content with stronger contractual protection.

No ToS file, no DMCA agent registered, no safe-harbor posture.

**Scope sprawl is the tell.** A serious commercial product would be: monitor → record → playback. Three crates, one binary, ship. Instead the roadmap is a résumé: beat-detect, demucs-split, brandsafe, sidechain compressor, EBU R128 two-pass loudness, ASS karaoke captions, viewguard fraud scoring. 386 unit tests on pure-data crates that **zero paying customers have asked for**.

**Founder positioning.** "iter-50–53 DAW closeout." Branches like `feat/strivo-pro-phase1`. Licence backend built before the first sale. Marketplace catalog with 18 entries before a single third-party plugin author exists. This is **architecture astronaut energy with a paywall stapled on**.

**Honest read.** **No. Not commercially viable as-is.** Wrong customer (none identified), wrong form factor (TUI for a video task), wrong moat (FOSS glue), wrong pricing ($25 one-time can't sustain), wrong legal posture (ToS violation as core feature, no safe harbor), wrong scope (38 plugins solving problems no buyer asked about).

**How I'd sell it.** Honestly? Stop selling it. Two pivots:
1. **Self-hosted VOD archive appliance** for streamers worried about Twitch bans/deletions. $199 one-time Pi appliance or $9/mo hosted. Drop the DAW. Drop the TUI as primary surface — keep the web UI. Pitch: "your channel insurance policy."
2. **Open-source it fully, build audience, monetize later.** As paid software it's dead. As a free `streamlink-wrapper-with-a-nice-UI` it could hit 5k stars and become a launchpad for something else.

There is no honest pitch for the current "$25 TUI DAW for stream recordings" framing.

**Top 3 Commercial Wounds**
1. **No identified customer.** Every persona that wants recording doesn't want a DAW; every persona that wants a DAW doesn't want a TUI. The Venn diagram is empty.
2. **Legal posture is a glass jaw.** Commercializing ToS violation with no DMCA agent, no ToS, and a payment processor that will terminate on first complaint. One Twitch legal email ends the company.
3. **Zero moat against FOSS substitutes.** Streamlink + yt-dlp + whisper.cpp + a shell script replicates 80% of the value in 200 lines of bash. The remaining 20% (38 plugins) is what the buyer didn't ask for.

---

## Synthesis — The Five Wounds Across All Axes

The reviewers converge on the same underlying pattern from five directions:

1. **Identity collapse.** The README sells a TUI PVR; the ROADMAP sells a DAW; the crates build a marketplace; the SPA quietly became the primary surface. Engineering, product, and marketing each describe a different StriVo. (UX, categorization, journey, commercial.)
2. **Architectural straddle.** The product simultaneously is a TUI and a web app, with `app.rs` as a shared god-object and `routes/plugins.rs` as a SQLite-scraping back-channel. The cost of two frontends is paid; the benefit of either is not realized. (Routing, categorization.)
3. **No service layer for the one thing the product does.** Recording — the core verb — has no `RecordingService`. It is rebuilt at every call site, with divergent behavior between TUI and SPA. Every "the webui does X but the TUI does Y" bug traces here. (Routing.)
4. **Doctrine without enforcement.** DESIGN.md, KEYMAP.md, the "pure-data plugin" boundary, the `publish = false` policy, the "third-party plugins not recommended" disclaimer — each is written, none is enforced. The repo carries five style guides it doesn't follow. (UX, categorization.)
5. **No customer means no forcing function.** Every other wound persists because no buyer is on the other end demanding fixes. Without that pressure, the roadmap becomes a résumé, the crates become hobbies, and the licence backend becomes an idle Cloudflare worker. (Commercial — and root cause of the other four.)

---

## Recommended Triage (Severity-Ordered)

| # | Action | Effort | Why |
|---|---|---|---|
| 1 | **Choose a product identity in one sentence and put it at the top of README.** Either "TUI PVR for streamers who fear bans" or "Self-hosted web archive for Twitch/YouTube." Delete the other vision from public-facing docs. | 1 day | Closes the identity collapse. |
| 2 | **Extract `intents::start_recording`/`download_vod` into `strivo-core`.** Replace 4+ ad-hoc dispatch sites. Collapse the duplicate `ClientMessage::DownloadVod` vs `Recording(DownloadVod)` wire shapes. | 1 week | Eliminates the bug-farm. |
| 3 | **Decide TUI-or-Web.** Move event types out of `app.rs` into `strivo-events`. Demote whichever frontend is not primary. | 2 weeks | Stops paying for both. |
| 4 | **Collapse 17 speculative crates into 2–3, or delete them.** Move private notes (`REVIEW.md`, `YAZI-AUDIT.md`, `FOLLOWUP-PLUGIN-WALK.md`, `PLUGINS_PRIVATE.md`) out of the public root. Pick one ROADMAP. | 1 week | New contributors can orient. |
| 5 | **Fix the keymap doctrine or delete it.** Remove `Ctrl+V` pane-local binding. Resolve `Shift+D` dual-meaning. Add `A`→AddChannel binding to Sidebar so FIRST-RUN.md stops lying. | 2 days | First-run trust. |
| 6 | **Add channel discovery.** Either follow-list import or by-handle search. Numeric `channel_id` paste must stop being the only path. | 3 days | The one-line pitch starts working. |
| 7 | **Pick a posture on legal exposure.** Register a DMCA agent, add ToS, or pivot away from paid distribution before Lemon Squeezy terminates. | 1 week + counsel | Removes the glass jaw. |
| 8 | **Decide whether this is commercial.** If yes, find ten real prospective buyers before writing iter-54. If no, open-source fully and pursue option 2 above. | Founder-level | Restores the forcing function. |

---

*End of review. No hedging applied.*
