# First run

This page covers what happens the very first time you launch strivo, where
state lives on disk, and the failure modes new users hit most often.

## Quick start

`strivo` with no arguments starts the daemon and serves the web UI at
`http://127.0.0.1:8181`. Point a browser at that URL to complete setup.

You'll be asked, in order:

1. **Platform credentials.** For each platform you want to use, provide a
   `client_id` / `client_secret` pair (Twitch and YouTube) or point at a
   `cookies.txt` export (YouTube members-only, Patreon). The browser
   completes the OAuth device-code dance against each platform.
2. **Recording directory.** Default is `~/Videos/strivo`. Created if
   missing.
3. **External tools check.** `strivo doctor` warns if `ffmpeg`, `mpv`,
   `streamlink`, or `yt-dlp` is missing from `PATH` — run it before
   adding channels.
4. **Initial channel list.** Add channels from the SPA sidebar; you can
   skip this and add them later.

Credentials are persisted to the OS keyring; non-secret preferences land
in `~/.config/strivo/config.toml`. To start over, delete that file (and
the matching keyring entries) or run `strivo config reset` to keep
credentials but reset preferences.

## Where state lives

strivo follows the XDG Base Directory specification.

| Purpose | Default path | Notes |
|---------|--------------|-------|
| Config | `~/.config/strivo/config.toml` | `strivo config path` prints the live value |
| Plugin manifests | `~/.config/strivo/plugins/` | One `.toml` per plugin; see `docs/PLUGIN-MANIFEST.md` |
| Logs | `~/.local/state/strivo/strivo.log` | `strivo log path` prints it; rotated by the harness |
| Recording journal | `~/.local/state/strivo/journal.sqlite` | Crash-recovery state; safe to delete (recordings are lost on truncate) |
| Recordings | Per `recording_dir` in config | Defaults to `~/Videos/strivo` |
| Daemon socket | `$XDG_RUNTIME_DIR/strivo/daemon.sock` | Falls back to `~/.cache/strivo/daemon.sock` |

On macOS the XDG roots map to `~/Library/Application Support/strivo/`,
`~/Library/Logs/strivo/`, and so on; on Windows the daemon honours
`%APPDATA%\strivo\` and `%LOCALAPPDATA%\strivo\`.

## Logging

Default log level is `info`. Bump it to `debug` for bug reports:

```bash
strivo -l debug
# or
RUST_LOG=debug strivo
```

`-l` is honoured for any subcommand; `RUST_LOG` overrides it when set.
Logs go to stderr **and** to the file at `strivo log path`.

To live-tail the file:

```bash
strivo log tail
```

## Common first-run failures

### `error: ffmpeg not found in PATH`

Install ffmpeg from your distribution (`pacman -S ffmpeg`,
`brew install ffmpeg`, `winget install ffmpeg`) and re-run
`strivo doctor`.

### OAuth verification page closes immediately

Some browsers race the device-code window. After clicking "Authorize",
return to the SPA — strivo polls the platform every 5 seconds and
surfaces `OAuth: token granted` once the grant lands.

### `keyring: no secret service available`

Common on headless Linux. Either start a Secret Service implementation
(gnome-keyring-daemon, kwalletd, keepassxc with the integration plugin) or
fall back to environment variables:

```bash
export STRIVO_TWITCH_CLIENT_ID=...
export STRIVO_TWITCH_CLIENT_SECRET=...
```

strivo logs a one-shot warning when it falls back to `STRIVO_*` env vars.

### "config corrupt — restored from .backup"

strivo round-trips its config through a `.backup` file. If the live file
fails to parse on startup, the previous good copy is restored
automatically and the broken file is quarantined as
`config.toml.quarantine-<timestamp>`.

### Recordings end with `exit code: 1` and no useful log

Almost always a stream-URL or codec issue. Re-run with `-l debug`, and
include the ffmpeg command line and the last ~30 lines of ffmpeg stderr
in the bug report.

## Next steps

- [docs/DAEMON.md](./DAEMON.md) — running strivo as a background service.
- [docs/SETTINGS-COVERAGE.md](./SETTINGS-COVERAGE.md) — which config keys
  the in-app settings panel can edit.
