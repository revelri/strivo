# REVIEW — Should StriVo migrate from ratatui to opentui?

Adversarial single-question review. Steelman the migration, counter it, name the verdict and the tripwires that would force a revisit.

**Context.** StriVo is ~70 `.rs` files, ratatui 0.30 with a custom event loop, 13 themes including Kitty `.conf` import, animation infrastructure (`FrameClock`, easings, `Tween<T: Lerp>`), 60 fps adaptive cadence, plugin system that already renders ratatui widgets across the FFI surface. Two viable opentui implementations exist in 2026: [anomalyco/opentui](https://github.com/anomalyco/opentui) (TypeScript, the upstream — most mature) and [Dicklesworthstone/opentui_rust](https://github.com/Dicklesworthstone/opentui_rust) (Rust port, stabilizing). Only the Rust port is migration-relevant.

---

## 1. Steelman the migration

The strongest possible argument for moving.

### 1.1 What opentui actually offers
opentui is **a rendering engine, not a framework** — you get buffers, cells, colors, scissor clipping, and **true RGBA alpha blending with double-buffered cell composition**. The Zig core has ~15.9k LOC and powers production apps. The Rust port exposes the same engine to Rust without a prescribed widget tree.

The relevant feature for StriVo is *cell alpha*. DESIGN-TODOS C3.7 ("alpha-blend overlay backdrops") was deferred specifically because **ratatui has no cell alpha** — overlays look like hard cutouts against the panes underneath. The closing-sprint note: *"We'd have to re-render every widget at a dimmed variant — ~2× the render cost."* opentui solves this natively. Same for C4 pane slides (sub-cell offset not possible in ratatui), C6.5 log smooth-scroll (integer cell rows only), C6.2 sparkline cell-blending.

If StriVo's visual ambition is "polished motion design with overlay depth," opentui removes the four largest motion-catalog deferrals from the previous sprint in one move.

### 1.2 Synchronized output as a first-class feature
opentui emits `ESC[?2026h` (synchronized output mode) by default. Modern terminals (kitty, foot, WezTerm, Ghostty, recent xterm) flush atomic frames; StriVo currently relies on crossterm's per-frame heuristics. For 60 fps motion this matters — tearing during the REC pulse vs. uptime tick has been observed but not measured.

### 1.3 Lower-level control is *good* for an app at our maturity
ratatui's immediate-mode widget tree is great for v0 but starts to fight against:
- Per-row animation (DESIGN-TODOS C1.3–6, all deferred because `ratatui::List` doesn't expose per-row state)
- Pane sliding (C4, deferred because there's no pane-router intermediate)
- Custom blend math (we already do `Color::Rgb` lerp by hand in `src/tui/anim/tween.rs`)

opentui gives us the substrate where these are trivial. Our pain point isn't *missing widgets* — we've shipped the widgets. It's *the assumptions baked into ratatui's widget contract*.

### 1.4 Plausible migration path
Not all-or-nothing:
1. Keep ratatui for current widgets; add opentui as the **compositor** that ratatui draws into. ratatui already has a pluggable Backend trait — `OpentuiBackend` implements it, ratatui widgets render into opentui's cell grid, overlays and animations use opentui directly for alpha + sub-cell.
2. Migrate widget-by-widget when each gains an animation that needs alpha (recording list selected-row tint, overlay backdrop dim, sparkline, log smooth scroll).
3. ratatui shim can be removed when the last widget is migrated, or kept indefinitely.

This is a six-month parallel track, not a rewrite.

### 1.5 What we gain
- All four currently-deferred motion items (C1.3–6, C3.7, C4, C6.5, C6.2) become possible.
- A clean substrate for M4 polish (`async task pane` with translucent overlay during long ops, preview pane with crossfade).
- Future-proofing: if opentui's ecosystem grows around the Zig core, we ride that growth instead of waiting for ratatui to add cell-alpha (which would be a major-version break).
- Cleaner mental model: rendering engine + our own widget contract is conceptually simpler than fighting a framework whose widgets we increasingly customize.

---

## 2. Counterargument

### 2.1 Our pain isn't rendering — it's missing features
Walk the ROADMAP. M1 is *recording journal, Patreon parity, Crunchr semantic search, schedule pane, recording management*. M2 is *settings suite*. M3 is *cohesive keymap*. None of these need cell alpha or sub-cell motion. The features users will judge StriVo on — does the back-catalog pull resume after a crash, do schedules fire reliably, can I delete a recording from the TUI — are entirely orthogonal to the renderer.

Migrating now means **months of work that produces zero user-visible feature progress.** The deferred motion items aren't blocking users; they're blocking aesthetics.

### 2.2 Maturity gap is real
ratatui:
- 0.30 series, ~4 years of production usage, dozens of TUI apps shipping on it (yazi, atuin, gitui, lazygit-rs, gh-dash, …)
- Stable widget contract, well-known performance characteristics
- Theming integrates with `crossterm`, kitty `.conf` import (which we shipped) assumes ratatui semantics

opentui_rust:
- Rust port of Zig core; Zig core is mature, the **Rust API is still stabilizing**
- Used by approximately… we're not sure. No notable production Rust apps cited.
- Different terminal-feature assumptions (synchronized output, RGBA cells) — what happens on terminals that don't support synchronized output? Apple Terminal? GNU screen? tmux's older versions? ratatui degrades gracefully because it never asked for any of that.

We have a P0/P1 quality bar (Tier 1 sprint). Adopting a stabilizing dependency for our core renderer is *exactly* the kind of foundation risk that gets us back to P0 territory.

### 2.3 The migration is not a checkbox; it's a roadmap detour
"Six-month parallel track" assumes:
- `OpentuiBackend: ratatui::Backend` works on day one. ratatui's `Backend` trait expects a particular flush model; opentui's double-buffered composition may not map cleanly.
- All current animations (`Tween<Color::Rgb>`, easings, the FrameClock) keep working. Likely yes, but unverified.
- Plugin widgets (Crunchr, Archiver) keep rendering. They use `ratatui::Frame` directly. If we shim, they keep working; if we migrate them, the plugin ABI changes — that's a breaking change to the sibling repo we manage.
- Kitty theme import logic (`src/tui/theme/kitty_import.rs`) doesn't depend on ratatui specifics. Verify, but plausibly clean.

Realistically: **6 months is the optimistic estimate**, and during those six months we are not shipping M1.

### 2.4 The wins are smaller than they sound
- **Cell alpha for overlays**: real win. But the existing border-ramp pattern (`Theme::border_ramp(progress)`) already communicates overlay depth without dimming the background. Users do not notice the absence of backdrop dim.
- **Sub-cell sliding**: in monospace terminal output, sub-cell motion is *imperceptible at 60 fps* on most fonts. Users notice 1-cell jumps; they do not notice fractional offsets.
- **Smooth log scroll**: yazi doesn't do this. Atuin doesn't. gh-dash doesn't. Users do not expect it from terminal apps. The 1-row jump is canonical.
- **Sparkline cell-blending**: nice-to-have. The viewer-count history isn't even captured by the monitor layer yet (C6.2 was deferred for *that* reason, not the rendering one).

We are looking at one real win (overlay alpha) and three taste-level wins. **Six months for one real win is bad arithmetic.**

### 2.5 We just shipped the animation infrastructure
2026-04-20: `FrameClock`, `Tween`, `Ease`, 60 fps adaptive cadence, full motion catalog applied, reduce-motion honored everywhere. The motion infrastructure is *new*. Throwing the substrate out one month after shipping the substrate is a textbook foundation-thrash anti-pattern — it suggests we didn't understand what we wanted before we built the first version.

### 2.6 Ecosystem cost
We rely on ratatui-adjacent crates:
- `ratatui-image` (channel thumbnails — already wired)
- `tui-textarea`-style patterns (search input)
- Theme conventions that match other Rust TUIs users might use simultaneously

opentui_rust does not yet have analogs for any of these. Either we wait, or we build them, or we maintain a hybrid forever.

### 2.7 The right time to migrate isn't now
A renderer migration is right when:
- The current renderer is the bottleneck for ≥2 of the next 5 release-blocking features, OR
- The current renderer has a fatal flaw (incompatibility with new terminal protocols, abandonment, security issue), OR
- The new renderer has multi-year production reference apps proving the migration is safe.

None of these conditions hold in May 2026.

---

## 3. Verdict

**Do not migrate. Stay on ratatui through M5. Revisit after 0.7.0 ships.**

The migration is technically plausible, the gains are real but narrow, and the cost is six months of opportunity cost during the highest-leverage phase of the roadmap (M1–M3 close the largest user-facing feature gaps). The deferred motion items that opentui would unlock are aesthetic, not functional.

Migrate when the answer to "what feature will this unlock?" is concrete and ROADMAP-blocking, not "polish we deferred a year ago."

---

## 4. Tripwires — when we revisit

Trigger a re-review of this decision if **any** of the following becomes true:

1. **ratatui abandonment signal** — primary maintainers step back, release cadence falls below one release / 6 months, or critical CVE goes unpatched for ≥30 days.
2. **opentui_rust production reference** — ≥2 substantial Rust apps (10k+ LOC, real users) ship on it for ≥6 months without major substrate issues.
3. **Motion-design block on a real feature** — a ROADMAP M4/M5 item is genuinely blocked by lack of cell alpha or sub-cell motion (not just aesthetically diminished by it).
4. **Synchronized-output requirement** — terminal vendors start mandating `ESC[?2026h` for certain effects we need (e.g., a future image protocol), and ratatui hasn't adopted it.
5. **Plugin ABI rewrite** — we end up rewriting the plugin ABI for unrelated reasons. At that point, a renderer change rides along cheaply.
6. **>3 motion deferrals in a future sprint** — if a future polish sprint defers more than three items for the same "ratatui can't do this" reason, the cost-benefit shifts.

---

## 5. Mini cost estimate (if we did migrate)

For reference, so future-us can sanity-check against actual cost when we revisit:

| Phase | Scope | Effort |
|---|---|---|
| 1. `OpentuiBackend: ratatui::Backend` shim | All current ratatui widgets keep working | 2–3 weeks (uncertainty: high — opentui's flush model) |
| 2. Migrate `src/tui/anim/*` to opentui-native blending | Replace manual `Color::Rgb` lerp with cell-alpha | 1 week |
| 3. Migrate overlay backdrops + REC/LIVE/spinner pulses | First feature win — backdrop dim | 2 weeks |
| 4. Migrate Sidebar, Detail, RecordingList, Settings, Log | Per-row selection animation enabled | 4–6 weeks |
| 5. Migrate widgets in `strivo-plugins` (Crunchr, Archiver) | Plugin ABI break or shim retained | 2–3 weeks |
| 6. Theme system: extend Kitty `.conf` import for cell-alpha slots | Optional; some terminals don't expose alpha | 1 week |
| 7. Terminal-compat matrix re-validation (Kitty, Ghostty, WezTerm, Alacritty, foot, Apple Terminal, tmux, GNU screen) | G.10-equivalent QA pass | 1–2 weeks |
| 8. Performance regression check (60 fps under heavy log) | Validate adaptive cadence still works | 1 week |
| **Total** | **End-to-end migration** | **~5–6 months** |

For comparison, M1 + M2 + M3 combined is estimated at ~4 months. Migration would cost more than the next three milestones combined and ship nothing users asked for.

---

## Sources

- [anomalyco/opentui (TypeScript upstream)](https://github.com/anomalyco/opentui)
- [Dicklesworthstone/opentui_rust](https://github.com/Dicklesworthstone/opentui_rust)
- [opentui_rust on lib.rs](https://lib.rs/crates/opentui_rust)
- [opentui.com](https://opentui.com/)
- DESIGN-TODOS closing-sprint notes (now folded into ROADMAP) — deferral reasons for C1.3–6, C3.7, C4, C5.5, C6.2, C6.5
