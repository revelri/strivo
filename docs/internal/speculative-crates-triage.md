# Speculative crates — triage

The adversarial review (`ADVERSARIAL-REVIEW.md`) called out 17 crates as
"speculative / DAW cargo-cult" with "no spawning consumer ever." The
data below re-runs that audit and shows the reviewer overstated the
case: 14 of the 17 are reached by the SPA via real route handlers in
`crates/strivo-web/src/routes/plugins.rs`. The actual recommendation
shrinks to **delete 1 crate, keep the rest**, and revisit whether the
14 endpoints they back are surfaced features or scaffold UI — that's a
UX audit, not a dead-code cleanup.

Counts collected 2026-05-29 against post-fold-in workspace.

## Method

For each crate `X`:
- `Cargo deps` = workspace `Cargo.toml`s declaring it as a dependency
  (excluding `crates/X/` itself).
- `Source uses` = Rust files importing `strivo_X` (excluding the
  crate's own files).
- `SPA refs` = string matches for `/api/v1/X`, `/api/v1/plugins/X`, or
  `"X"` in `crates/strivo-web/assets/spa.js`.
- `Route symbol uses` = symbol references in
  `crates/strivo-web/src/routes/plugins.rs`.

| Crate                | LoC | Tests | Cargo deps                                   | Source uses                          | SPA refs | Route uses |
|----------------------|----:|------:|----------------------------------------------|--------------------------------------|---------:|-----------:|
| **demucs-split**     | 213 |     8 | —                                            | —                                    |    0     |       0    |
| ab-render            | 227 |     7 | pitch, insert-fx                             | pitch, insert-fx                     |    SPA route only (`#/studio/ab`) | 0 |
| submix               | 194 |     6 | insert-fx                                    | insert-fx                            |    SPA tile only                | 0 |
| insert-fx            | 401 |    14 | strivo-web                                   | strivo-web/routes/plugins.rs         |    2     |       4    |
| pitch                | 275 |    15 | strivo-web                                   | strivo-web/routes/plugins.rs         |    1     |       3    |
| sidechain            | 309 |    12 | strivo-web                                   | strivo-web/routes/plugins.rs         |    1     |       2    |
| vad                  | 383 |    12 | sidechain, strivo-web                        | sidechain, strivo-web                |    1     |       7    |
| beat-detect          | 375 |    12 | strivo-web                                   | strivo-web                           |    2     |       6    |
| structure            | 450 |    12 | strivo-web                                   | strivo-web                           |    2     |       5    |
| schedule-optimizer   | 424 |    13 | strivo-web                                   | strivo-web                           |    4     |       5    |
| viewguard-trend      | 399 |    13 | strivo-web                                   | strivo-web                           |    2     |       3    |
| brandsafe            | 400 |    10 | strivo-web                                   | strivo-web                           |    3     |      15    |
| broll                | 361 |    11 | strivo-web                                   | strivo-web                           |    2     |       4    |
| casebook             | 420 |    11 | strivo-web                                   | strivo-web                           |    3     |      11    |
| reuse                | 538 |    12 | strivo-web                                   | strivo-web                           |    2     |       5    |
| insights-compare     | 318 |    10 | strivo-web                                   | strivo-web                           |    2     |       6    |
| dataviz              | 360 |     8 | strivo-web                                   | strivo-web                           |    4     |       3    |

## Verdict per crate

### Delete (1)

**`demucs-split`** — 0 SPA refs, 0 route uses, 0 transitive consumers.
The only references in the repo are:
- ROADMAP.md:205 listing it as "needs external `demucs` binary" (aspirational).
- multitrack/src/lib.rs:61 — a doc comment naming it as a "future hook."
- marketplace/src/lib.rs:229 — a catalog entry pointing at a separate
  `github.com/Chorosyne/demucs-split` repo URL that doesn't resolve to
  this crate.

Nothing in the workspace calls it, and the audit's own ADVERSARIAL-REVIEW
flagged it as "a 213-LOC wrapper for a CLI." It earns its place on the
delete list. The marketplace catalog entry must be removed in the same
commit (task #18 will renumber the host version anyway).

### Keep — internal helpers (2)

**`ab-render`** — used by `pitch` + `insert-fx` to compose ffmpeg
filter strings. 7 tests, reasonable factoring. Has a SPA route at
`#/studio/ab` but no Rust consumer outside `crates/`. Keep as an
internal helper.

**`submix`** — used by `insert-fx` for sub-mix bus composition. 6 tests.
SPA tile in the studio UI but no current Rust consumer outside `crates/`.
Keep.

(Both could be merged into a single `strivo-audio-fx` crate alongside
`insert-fx`, `pitch`, `sidechain`, `vad` if the per-crate boundary
becomes a cost. Today it isn't — each is 200–400 LoC with its own
tests, and the workspace boundary stops the boundary-crossing cost.)

### Keep — backs real SPA endpoints (14)

The remaining 14 each carry **multiple route handlers** in
`strivo-web/routes/plugins.rs` (2–15 symbol references) and **active
SPA call sites** (1–4 each). The audit's "no consumer" claim was
wrong: the *daemon* doesn't spawn them, but the *web crate* mounts
them as HTTP endpoints the SPA exercises.

Whether those endpoints back **shipping features** or **placeholder
UI** is a separate question. A UX walkthrough — clicking each tile in
the SPA and confirming it does what the description claims — is the
right follow-up. That's a product audit, not a dead-code cleanup.

Names worth disambiguating in a glossary so new contributors can orient
(per the audit's "naming coherence" point):

- `brandsafe` — content-policy detection for ads / sensitivity scoring
- `broll` — automated B-roll suggestions for transcript timestamps
- `casebook` — sample/clip catalog
- `reuse` — cross-recording motif detection
- `viewguard-trend` — viewbot trend analysis over time
- `structure` — narrative-arc detection on transcript segments
- `schedule-optimizer` — recommended broadcast time slots

A `docs/GLOSSARY.md` is a 1-hour task tracked under #15 (docs purge).

## Implications for task #8

Task #8 ("Collapse 17 speculative crates into cohesive bundles") was
sized against the audit's claim that 14 of these are "serde-only DAW
analogues" with no real consumer. That premise doesn't hold — they're
each backing 3–15 route handler uses. **Closing #8 as overstated.** A
narrower follow-up survives: optionally regroup the audio-effect family
(`ab-render`, `submix`, `insert-fx`, `pitch`, `sidechain`, `vad`,
potentially `beat-detect`) into one `strivo-audio-fx` super-crate for
discoverability. That's a 1-day refactor with no behaviour change. Not
urgent.

The one concrete action is **delete `demucs-split`** + its marketplace
entry. That's task #21 below.
