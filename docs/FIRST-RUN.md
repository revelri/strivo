# First run

This page covers what happens the very first time you launch strivo, where
state lives on disk, and the failure modes new users hit most often.

## The setup wizard

Run `strivo` with no config. The TUI opens onto a first-run wizard that
walks you through:

1. **Platform credentials.** For each platform you want to use, you provide
   a `client_id` / `client_secret` pair (Twitch and YouTube) or point at a
   `cookies.txt` export (YouTube members-only, Patreon). The wizard runs
   the OAuth device-code flow live; pressing Enter on the verification line
   opens the platform's verification URL in your browser via `xdg-open`
   (Linux), `open` (macOS), or `start` (Windows).
2. **Recording directory.** Default is `~/Videos/strivo`. The wizard
   creates it if missing.
3. **External tools check.** The wizard runs `strivo doctor` and warns if
   `ffmpeg`, `mpv`, `streamlink`, or `yt-dlp` are missing from `PATH`.
4. **Initial channel list.** You can skip and add channels later from the
   sidebar (`A` to add).

The wizard writes `~/.config/strivo/config.toml` and stores secrets in the
OS keyring. To start over, delete that file and the matching keyring
entries (or run `strivo config reset` to keep credentials but reset
preferences).

## Where state lives

strivo follows the XDG Base Directory specification.

| Purpose | Default path | Notes |
|---------|--------------|-------|
| Config | `~/.config/strivo/config.toml` | `strivo config path` prints the live value |
| User themes | `~/.config/strivo/themes/` | `*.toml` (native) or `*.conf` (Kitty / Ghostty) |
| Plugin manifests | `~/.config/strivo/plugins/` | One `.toml` per plugin; see `docs/PLUGIN-MANIFEST.md` |
| Logs | `~/.local/state/strivo/strivo.log` | `strivo log path` prints it; rotated by the harness |
| Recording journal | `~/.local/state/strivo/journal.sqlite` | Crash-recovery state; safe to delete (recordings are lost on truncate) |
| Recordings | Per `recording_dir` in config | Defaults to `~/Videos/strivo` |
| Daemon socket | `$XDG_RUNTIME_DIR/strivo/daemon.sock` | Falls back to `~/.cache/strivo/daemon.sock` |

On macOS the XDG roots map to `~/Library/Application Support/strivo/`,
`~/Library/Logs/strivo/`, and so on; on Windows the TUI honours
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
return to the terminal — strivo polls the platform every 5 seconds and
prints `OAuth: token granted` once the grant lands.

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
- [docs/KEYMAP.md](./KEYMAP.md) — TUI key bindings.
- [docs/SETTINGS-COVERAGE.md](./SETTINGS-COVERAGE.md) — which config keys
  the in-TUI settings panel can edit.
