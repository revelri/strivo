# Plugin GUI walkthrough — known-gaps inventory

Living catalogue of placeholder text, stub buttons, and incomplete wiring
surfaced during the iter-65 walk. Each entry is intended as a one-shot fix
in a later iter.

Format: `Route → element → expected vs actual → fix shape`.

## /library
- Live-channel rail platform groupings — header order is hard-coded; should
  honour `localStorage.strivo-layout-rail-platforms` (T3 reorder list exists,
  just needs the rail painter to consume it).

## /recordings
- "Group by" dropdown — currently a single toggle button; T3 layout panel
  defines four group-by modes (channel / platform / date / state / none)
  but the toolbar still only flips channel ↔ none.
- Massbar bulk actions — "Trash" and "Re-record" are wired; "Export
  metadata" and "Mark watched" buttons emit `Toast.success` without a
  backing API. **Action**: either remove or wire to the existing
  `/api/v1/recordings/<id>` PATCH.

## /schedule (Monitor)
- "Capture limits" sliders are visible but the daemon hasn't enforced
  them yet (tracked separately as O2 + O3 in the cron prompt).

## /pipelines
- Existing DAG nodes render but aren't clickable to drill into a
  per-plugin run history. Recipe chains (T4 — landed today as data model
  + CRUD) need a visual editor here.

## /watch
- All wired except: cross-platform PiP (only YouTube + Twitch supported;
  Patreon doesn't expose an embed URL).

## /chat
- Compose box is read-only — needs OAuth `chat:edit` (B5).
- Badge images fall back to text chips (B3).
- FFZ / 7TV emotes not rendered (B4).

## /plugins
- Per-card stats counters use heuristics — some plugins have an empty
  stats object server-side. Audit each crate's `stats_for_recording_id()`.

## /settings
- Interface → Layout (NEW iter 65) — reorders persist but apply ONLY to the
  top-nav so far. Rail platform / recordings group-by / plugin-hub cats
  read the keys but only top-nav repaints from them.
- Notifications panel — `desktop_notify` toggle persists; daemon
  honours it. ✓
- Platforms — Twitch / YouTube / Patreon credentials all input-only;
  add a "Test connection" button per platform.

## /system
- Health checks all render; Backup download streams correctly.
- "Tasks" table is a TODO stub — wire to the daemon's task scheduler.

## /logs
- Date-range picker + trace-id click filter shipped iter 60–63. ✓

## /history
- Per-row Play / Info / Delete wired. Date heatmap (A2) is the only
  remaining gap.

## Plugin sub-routes

### #/plugins/crunchr
- All wired. Heatmap → schedule-optimizer deep-link shipped iter 58.

### #/plugins/archiver
- Channel picker → archive view: working.
- Per-VOD "Download" button: working.

### #/plugins/viewguard
- Cross-stream trend dashboard: working.
- Per-recording verdict drill-down: stub (shows JSON; needs a card).

### #/plugins/insights
- Insights aggregator: working.
- Compare picker for stream-vs-stream: working.

### #/plugins/schedule-optimizer
- Auto-feed buttons shipped iter 57. ✓

## Next actions
Convert each `**Action**` line above into a discrete iter in the cron.
Trivial ones (remove orphaned buttons, wire missing PATCH endpoints,
flip read-only inputs to working) can ship in batches; bigger ones
(visual chain editor, OAuth flows) get one iter each.
