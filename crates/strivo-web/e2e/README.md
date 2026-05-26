# StriVo Web UI — E2E tests (W7)

Headless Playwright suite covering the critical webui journeys: login,
library (live/offline channels), bulk-download trigger, ⌘K command
palette navigation, recordings grid filter/sort, and the Patreon route.

The tests run against the **real SPA assets** (`../assets`) served by a
small Node mock backend (`mock-server.mjs`) that stubs `/api/v1` and
`/events`. No daemon, no platform auth, fully deterministic.

## Run

```sh
cd crates/strivo-web/e2e
npm install
npm run install-browser   # one-time: downloads Chromium (skip if already cached)
npm test
```

Playwright's `webServer` config starts the mock server automatically on
port 8199 and tears it down after the run.

## Files

- `mock-server.mjs` — static asset server + API/SSE stubs + fixtures.
- `playwright.config.ts` — chromium project, `webServer`, `baseURL`.
- `tests/smoke.spec.ts` — the journeys.

`node_modules/` and reports are gitignored; only the source is checked in.
