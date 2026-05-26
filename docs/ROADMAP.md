# StriVo Web UI — Hardening & Research Roadmap

Fuses the **High-severity code-review findings** with the **outstanding
recommendations** from `docs/WEBUI-RESEARCH.md` (the *arr / Jellyfin / Jellyseerr
study). Items 1–8 of the prior improvement loop are already landed; this roadmap
is what remains. Worked top-to-bottom by the 6m cron loop — each item is one
focused PR-sized commit: implement, `cargo clippy --workspace --all-targets -- -D
warnings` clean + tests green, commit (no AI attribution, no backticks in `-m`),
push, redeploy (rebuild daemon + restart `strivo serve`), then tick the box.

Source tags: `[review]` = code-quality review High/Medium finding; `[F]`/`[A]`/`[B]`/
`[C]`/`[D]`/`[E]` = WEBUI-RESEARCH.md section.

---

## Phase 1 — Security hardening

- [x] **1. Login rate-limiting** `[review]` — cap failed `POST /login` attempts
  (per-IP token bucket / sliding window); lock-out + `Retry-After`. Loopback/TS
  mitigates but doesn't remove brute-force.
- [x] **2. Recordings path containment** `[review]` — canonicalise the served file
  path against the recordings root; reject traversal (`..`, symlink escape) with 403.
- [x] **3. `login.rs` Set-Cookie hardening** `[review]` — replace `.parse().unwrap()`
  with graceful error; never panic the handler on a malformed header value.
- [x] **4. Cookie attributes + idle refresh** `[F]` — adaptive
  `__Host-`/`Secure`/`SameSite=Lax` + dual-name read + idle session refresh
  (re-issues past the halfway mark via response middleware). — `__Host-strivo_session; HttpOnly;
  Secure; SameSite=Lax; Path=/`; rotate HMAC session on activity; expired/invalid HMAC
  ⇒ logged-out (302/401), never 500. Must still work on `*.ts.net` over HTTPS.
- [x] **5. CSRF custom-header on cookie mutations** `[F]` — require `X-Strivo-CSRF`
  (or `X-Requested-With`) on all cookie-authed state-changing requests, plus strict
  `Origin`/`Host` allowlist (`127.0.0.1`, `*.ts.net`). `X-Api-Key` track stays
  CSRF-exempt by design. All mutations POST/PUT/DELETE (never GET).
- [x] **6. Security unit tests** `[review][F]` — cover HMAC encode/verify + expiry,
  CSRF header/Origin/Host checks, and the path-containment guard. These are
  load-bearing; they get dedicated tests.

## Phase 2 — API correctness & robustness

- [x] **7. RFC 9457 Problem Details envelope** `[A]` — single `Problem` type
  (application/problem+json); all api.rs + login.rs error returns converted
  (the 429 rate-limit keeps its bespoke Retry-After response). — one axum `IntoResponse` error
  type (`type,title,status,detail,instance`); replace ad-hoc JSON error shapes.
- [x] **8. Bound the recordings map** `[review]` — evict finished/failed jobs past a
  cap or age so `app.recordings` doesn't grow unbounded for the process lifetime.
- [x] **9. Cap concurrent client tasks** `[review]` — bound the per-connection IPC
  task spawns in the daemon (semaphore / join-set with limit).
- [x] **10. Dead-code sweep** `[review]` — deleted the 6 legacy htmx page
  modules + orphan templates; `session_secret` Option→String (always set at
  startup) with `session_from_headers`/`check_dual` taking `&str`; removed
  the now-dead lazy-secret branch in login. — `session_secret: Option<…>` ⇒ `String`
  (always Some at startup); delete the retired/unmounted legacy htmx route modules
  and `with_filter`-style dead helpers.
- [x] **11. `/health` JSON endpoint** `[E]` — `/api/v1/health` now probes
  daemon (IPC snapshot), jobs DB (open), and free disk; 200 when all ok, 503
  when degraded, with a per-check breakdown for monitors. — machine-readable: recorder up, DB
  reachable, disk free; separate from the UI panel, for CI/monitoring.
- [x] **12. Visible SSE liveness** `[A]` — `X-Accel-Buffering: no` on
  `/events` so proxies don't buffer SSE; the SPA's reconnecting pill +
  degraded re-poll were already in place. — set `X-Accel-Buffering: no` + no response
  buffering on `/events`; confirm/finish the "reconnecting" badge so a dropped SSE is
  never silently stale.

## Phase 3 — System / Health UX

- [x] **13. Health-check registry** `[E]` — backend
  `GET /api/v1/health/checks` (grouped Storage/Platform Auth/Network checks
  with severity + fix + worst-rollup) + SPA: global topbar health pill
  (amber/red, links to System, hidden when ok) and a domain-grouped checks
  list on the System page sourced from the registry.
- [x] **13.5. ElegantFin webui restyle** `[D]` — ported ElegantFin tokens +
  near-black gradient into `:root`; glass topbar/leftrail; ElegantFin buttons;
  section-title leading bar; glass+rounded+shadow cards (rec/channel/cfg),
  recordings table, dialogs/modals, toasts; rounded translucent inputs/select/
  textarea with accent focus ring. Variable-driven so the palette propagates;
  DOM/class names unchanged, e2e green. Restyle
  `crates/strivo-web/assets/spa.css` to follow the user's Jellyfin theme as
  **literally as possible**: the ElegantFin theme + the near-black YouTube
  gradient override. Port the token table from DESIGN.md ("Web UI Theme")
  into `:root` verbatim (gradient `#101010→#050505`, accent `rgb(119,91,244)`,
  text `rgb(209,213,219)`, radii 1.25/1/.5/.375em, blur 2/5/10/20px, shadow,
  borders), then apply the component idioms: section titles with a leading
  white bar, non-primary buttons `rgba(0,0,0,.2)` r10px + hover `rgba(0,0,0,.5)`,
  link-buttons with grow-on-hover underline, glass cards (`1em`, blur, soft
  shadow), submit/delete button colors. Reference CSS in `docs/reference/`.
  Keep all existing DOM/class names; e2e must stay green. Likely multi-fire
  (mark `- [~]`): tokens+chrome first, then cards/tables/dialogs/forms.
- [x] **14. Scheduled-task duality** `[A][E]` — System "Tasks" section makes
  scheduled-vs-on-demand explicit: the channel-poll task shows its cadence +
  a Run-now button (enqueues the same `PollNow` command as the timer);
  scheduled recordings listed with cron/duration; active recordings link to
  the dashboard Stop. (Live interval editing split to item 14b — needs daemon
  config hot-reload since the monitor reads poll_interval once at startup.)
- [ ] **14b. Live-editable poll interval** `[E]` — daemon config hot-reload:
  monitor re-reads `poll_interval_secs` on a `SetPollInterval` IPC message +
  a settings-write endpoint, so the System Tasks interval becomes editable
  without a restart. (Deferred from 14 to avoid a config-write that silently
  needs a restart.)
- [x] **15. Logs viewer polish** `[E]` — daemon logs to rolling/capped files
  (daily rotation, keep 7 via tracing-appender); `GET /api/v1/logs?level=&lines=`
  tails the newest file with min-level filtering; SPA Logs route (📜) renders
  the tail in a mono pane with a level-selector dropdown + refresh. e2e covers it.
- [x] **16. Config/DB backup + restore** `[E]` — dep-free backup sets under
  `data_dir/backups/<ts>/` (config.toml + jobs.db); `POST /api/v1/backup`,
  `GET /api/v1/backups`, `POST /api/v1/backups/{name}/restore` (name-sanitized,
  restart-to-apply); SPA System "Backup" card with Backup-now + list + restore
  (confirm dialog). On-demand only; scheduled/automatic backups deferred (the
  manual snapshot + restore path covers the high-trust "irreplaceable config"
  need from research §E).

## Phase 4 — Information architecture & journey

- [x] **17. Durable History + Blocklist** `[B]` — Blocklist: durable table +
  catalog skip-wiring + `GET/POST/DELETE /api/v1/blocklist` + System card
  (list/unblock) + channel-detail Block button. History: `GET /api/v1/history`
  over the jobs DB + a History route (🗂) rendering the durable completed/failed
  audit (survives restart, unlike the in-memory /recordings snapshot).
- [x] **18. Upcoming calendar/agenda** `[B]` — the Schedule route (was a stub)
  is now a first-class agenda: `/schedule` entries grouped by day
  (Today/Tomorrow/date) sorted by server-computed `next_fire`, each with time +
  cron cadence; unparseable-cron entries bucketed separately. (Source = StriVo's
  own scheduled recordings; platform-side broadcast schedules aren't API-exposed.)
- [x] **19. Add-Channel two-phase wizard** `[B]` — backend resolve
  (`ClientMessage::ResolveChannel` → bulk-manager `resolve_channel`: Twitch
  login→id, YT/Patreon id pass-through → `DaemonEvent::ChannelResolved` over
  SSE; `POST /api/v1/channels/resolve`) + SPA topbar "＋ Add" wizard modal:
  phase 1 pick platform + search (live resolve), phase 2 confirm the resolved
  entity → enable auto-record. Config deferred until the entity is confirmed.
- [ ] **20. First-run wizard** `[B]` — gate the SPA behind connect platforms → pick
  channels → recording defaults → storage path; no half-configured dashboard.
- [ ] **21. Named capture profiles + cutoff** `[B]` — define once
  ("1080p60+transcript", "audio-only"), attach to many channels, with a cutoff so
  StriVo stops re-capturing once met. Warn on pathological perpetual-re-record configs.
- [ ] **22. Index density + mass-edit** `[B]` — switchable table/overview density over
  the recordings/channels dataset + multi-select mass-edit action bar (re-run plugins /
  delete / re-record).

## Phase 5 — Live preview & micro-UX

- [ ] **23. Hover/detail live preview** `[C]` — card static refreshing thumbnail →
  upgrade to `<video muted playsinline autoplay poster>` on detail-open / scroll-into-
  view → teardown off-screen → tap-to-play on mobile. Path A self-proxied HLS (hls.js,
  `autoStartLoad:false` + IntersectionObserver) for recordings + Twitch rewind; Path B
  iframe for un-proxyable live Twitch/YT; Patreon thumbnail-only.
- [ ] **24. Toast + ARIA live regions** `[D]` — singleton with two pre-created regions
  (`polite/status`, `assertive/alert`); success ≥5s, errors sticky + dismissible,
  pause-on-hover, cap ~3–4, `prefers-reduced-motion`, 4.5:1 contrast, non-interactive.
- [ ] **25. Async-feedback polish** `[D]` — `aria-busy` + label swap + debounce on
  buttons (kill double-submit); skeletons for grids; inline field-level validation
  (`aria-describedby`/`aria-invalid`); actionable empty states wired to real CTAs;
  never strand a spinner (timeout + error surface + guaranteed teardown everywhere).
