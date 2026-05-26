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

- [~] **7. RFC 9457 Problem Details envelope** `[A]` — *(part 1 of 2:*
  *`Problem` type (application/problem+json) + the 18 unauthorized blocks*
  *converted; remaining varied-status daemon/validation error returns in*
  *api.rs convert next fire.)* — one axum `IntoResponse` error
  type (`type,title,status,detail,instance`); replace ad-hoc JSON error shapes.
- [ ] **8. Bound the recordings map** `[review]` — evict finished/failed jobs past a
  cap or age so `app.recordings` doesn't grow unbounded for the process lifetime.
- [ ] **9. Cap concurrent client tasks** `[review]` — bound the per-connection IPC
  task spawns in the daemon (semaphore / join-set with limit).
- [ ] **10. Dead-code sweep** `[review]` — `session_secret: Option<…>` ⇒ `String`
  (always Some at startup); delete the retired/unmounted legacy htmx route modules
  and `with_filter`-style dead helpers.
- [ ] **11. `/health` JSON endpoint** `[E]` — machine-readable: recorder up, DB
  reachable, disk free; separate from the UI panel, for CI/monitoring.
- [ ] **12. Visible SSE liveness** `[A]` — set `X-Accel-Buffering: no` + no response
  buffering on `/events`; confirm/finish the "reconnecting" badge so a dropped SSE is
  never silently stale.

## Phase 3 — System / Health UX

- [ ] **13. Health-check registry** `[E]` — each check returns `{severity:
  warn|error, message, fix-link}`, grouped by domain (Storage, Platform Auth, Plugins,
  Network), retestable; global header health pill (amber/red) links to the list.
- [ ] **14. Scheduled-task duality** `[A][E]` — every periodic task gets a manual
  "Run now" enqueuing the same command; intervals editable; running tasks cancellable.
- [ ] **15. Logs viewer polish** `[E]` — in-UI level selector + rolling/capped files
  so users never SSH for logs.
- [ ] **16. Config/DB backup + restore** `[E]` — scheduled + on-demand backup of
  config + jobs DB with a restore path.

## Phase 4 — Information architecture & journey

- [ ] **17. Durable History + Blocklist** `[B]` — completed/failed audit trail that
  survives restart (not toast-and-forget) + skip-this-VOD/channel feedback.
- [ ] **18. Upcoming calendar/agenda** `[B]` — first-class view of known upcoming
  broadcasts (scheduled Twitch/YT, Patreon drops).
- [ ] **19. Add-Channel two-phase wizard** `[B]` — type name → live search → pick
  entity → *then* configure (profile, monitor, plugins). Defer config until confirmed.
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
