# strivo-web — *arr-style HTTP frontend

Companion to the TUI. Speaks to a running `strivo` daemon over the same Unix-socket IPC the TUI uses; never owns state directly.

## Quickstart

```bash
# In one terminal:
strivo daemon

# In another:
strivo serve --bind 127.0.0.1:8181
# strivo-web on http://127.0.0.1:8181 (X-Api-Key: <persisted>)

curl -H "X-Api-Key: $KEY" http://127.0.0.1:8181/api/v1/channels
xdg-open http://127.0.0.1:8181/
```

## Status

**~80% complete; dev-build ready.** All major routes implemented and verified against a live daemon. Outstanding work tracked in [WEBUI-ROADMAP.md](../../WEBUI-ROADMAP.md): full per-form CSRF tokens (currently Origin/Host check + localhost bind), per-route integration tests, hover/preview niceties.

## Stack

- Axum 0.8 (HTTP)
- Askama 0.12 (typed templates, compile-time checked)
- HTMX 2 (vendored at `assets/htmx.min.js`, ~50 KiB)
- rust-embed (single-binary asset serving)
- No JS build pipeline

## Routes

| Path | Sonarr equivalent | Implemented |
|---|---|---|
| `/` | Activity | ✅ |
| `/channels` | Series | ✅ |
| `/recordings` | Wanted + History | ✅ (with range-request file streaming) |
| `/schedule` | Calendar | ✅ |
| `/settings` | Settings | ✅ (whitelisted writes) |
| `/system/logs` | System > Logs | ✅ (SSE live-tail) |
| `/system/status` | System > Status | ✅ (daemon up/down, disk, plugins) |
| `/events` | n/a | ✅ (SSE daemon-event relay) |
| `/api/v1/health` | `/api/v3/system/status` | ✅ (no auth — liveness) |
| `/api/v1/channels` | `/api/v3/series` | ✅ |
| `/api/v1/recordings` | `/api/v3/history` | ✅ |
| `/api/v1/recordings/{id}` | n/a | ✅ |
| `/api/v1/schedule` | `/api/v3/calendar` | ✅ |
| `/api/v1/settings` | `/api/v3/config` | ✅ (read-only) |

## Auth

Single API key, header `X-Api-Key` (matches *arr convention). Default `127.0.0.1` bind.

The key is persisted to `[web] api_key` in `config.toml` so repeated `strivo serve` invocations hand the same key to your scripts. Override per-run with `strivo serve --api-key <KEY>`. To rotate, delete the field from `config.toml` and restart.

`X-Api-Key` is compared in constant time to mitigate timing oracles.

## CSRF posture

State-changing requests (POST / PUT / PATCH / DELETE) outside `/api/v1/*` must carry an `Origin` (preferred) or `Referer` header whose host matches the server's `Host` header. Cross-origin form submits are rejected with `403 Forbidden`.

`/api/v1/*` is exempt — it's already gated by the `X-Api-Key` header, which a cross-origin browser can't forge without a CORS preflight (which we don't grant).

Combined with the default `127.0.0.1` bind, this defeats the cross-origin-form CSRF class without per-form tokens. Operators binding to a non-loopback interface should run behind a reverse proxy that adds CSRF tokens; full per-form tokens land in a follow-up.

## Daemon dependency

`strivo serve` refuses to start if the daemon isn't reachable; suggests `strivo daemon`.
