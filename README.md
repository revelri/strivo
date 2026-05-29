# strivo

Self-hosted live-stream PVR with a web UI. Monitor channels across Twitch, YouTube, and Patreon — automatically record when they go live, play back in the browser, and optionally transcribe recordings with Whisper.

> **TUI removed.** The original ratatui-based TUI was retired; the web UI
> is the only supported frontend. See [CHANGELOG.md](./CHANGELOG.md) for
> the inventory.

[![CI](https://github.com/Chorosyne/strivo/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Chorosyne/strivo/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Chorosyne/strivo?sort=semver&display_name=tag)](https://github.com/Chorosyne/strivo/releases)
[![MSRV](https://img.shields.io/badge/MSRV-1.75%2B-orange?logo=rust&logoColor=white)](Cargo.toml)
[![License](https://img.shields.io/github/license/Chorosyne/strivo?color=blue)](LICENSE)
[![AUR](https://img.shields.io/aur/version/strivo?label=AUR&logo=archlinux&logoColor=white)](https://aur.archlinux.org/packages/strivo)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS-1f6feb?logo=linux&logoColor=white)](#platform-support)
[![Made with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org)

> **Status: alpha.** The configuration format, daemon IPC protocol, and plugin ABI are
> all unstable and will keep changing until 0.5.0. Expect to re-edit `config.toml`
> across releases. See [ROADMAP.md](./ROADMAP.md) for the stability timeline and
> [CHANGELOG.md](./CHANGELOG.md) for migration notes.

---


## What it does

strivo runs a background daemon and serves a web UI on `localhost:8181`.
The daemon watches the channels you tell it to; when one goes live, it
records the broadcast through ffmpeg (resolving the playable URL via
streamlink or yt-dlp) and notifies you. You browse recordings, play them
back in the browser, run optional plugins (Whisper transcription, gallery
archiver), and search across your archive — all from the SPA.

`strivo` with no arguments starts the daemon and the webui in one process.
For systemd setups, `strivo daemon` runs the daemon alone and
`strivo serve` runs the webui alone; both talk over the same Unix socket
the daemon used to share with the legacy TUI.

### Platform support

| Platform | Auth | Monitoring | Recording | Notes |
|----------|------|------------|-----------|-------|
| Twitch | OAuth device flow | Followed-channel polling | `ffmpeg` + `streamlink` | Sub-only streams via OAuth token passthrough |
| YouTube | OAuth + Data API v3 | Live-broadcast detection | `ffmpeg` + `yt-dlp` | Cookie-based auth for members-only streams |
| Patreon | Membership API | Post / stream detection | `yt-dlp` | Subscription-tier extraction |

### Operating systems

| OS | Web UI | Daemon | Status |
|----|--------|--------|--------|
| Linux (x86_64) | ✅ | ✅ | Primary target; CI-gated |
| macOS (aarch64 / x86_64) | ✅ | ✅ | Builds and runs; manual testing pre-release |
| Windows | ❌ | ❌ | Daemon IPC uses Unix sockets — Windows named-pipe transport is on the roadmap |

## Features

**Core**
- Multi-platform channel monitoring with configurable poll intervals
- Automatic recording when channels go live (per-channel toggle)
- Live playback through mpv without downloading first
- Cron-based recording schedules
- Desktop notifications on go-live events
- Configurable filename templates (`{channel}_{date}_{title}.mkv`)
- Retry with exponential backoff on failed recordings

**TUI**
- Sidebar with channel list, auto-record toggles, platform indicators
- Channel detail view with stream metadata and recent recordings
- Recording browser — sortable, filterable, with size and duration
- Settings panel — edit config without leaving the TUI
- Live log viewer
- First-run setup flow for platform credentials in the web UI
- Multiple color themes

**Daemon mode**
- Background service via Unix-socket IPC
- One or more TUI clients can attach to a running daemon
- `strivo daemon install` writes a systemd user unit

**Plugins**
- **Crunchr** — Voxtral via OpenRouter (default, $0.003/min) / Mistral direct (diarization) / WhisperX local (self-hosted GPU diarization) / self-hosted Voxtral / Whisper CLI transcription, Speaker Editor TUI modal, SRT/VTT export with `mkvmerge` soft-sub embedding, tandem-mode auto-trigger, SQLite storage
- **Archiver** — organizes recordings by channel, renders gallery views

First-party plugins live in-tree under [`crates/strivo-plugins/`](./crates/strivo-plugins) —
the former separate `strivo-plugins` repo was folded into the workspace.
See [docs/PLUGIN-MANIFEST.md](./docs/PLUGIN-MANIFEST.md) for ABI notes
and the plugin loader contract.

## Tech stack

- **Language:** Rust 1.75+
- **Web UI:** SPA served from `strivo-web` (axum + a vanilla-JS single-file SPA)
- **Recording:** ffmpeg, streamlink, yt-dlp
- **Playback:** mpv
- **Transcription:** Voxtral via OpenRouter (default), Mistral API (with diarization), WhisperX + pyannote (self-hosted GPU diarization, two-stage VRAM unload for 8 GB cards), self-hosted Voxtral (vLLM / RunPod), Whisper CLI
- **Subtitling:** VTT + SRT sidecars, optional `mkvmerge` soft-sub mux back into the recording
- **Storage:** SQLite (bundled via `rusqlite`) for transcripts and journal
- **IPC:** Unix domain sockets (daemon / client)
- **Config & secrets:** TOML on disk, OS keyring for credentials

## Installation

### Prerequisites

- **Rust** 1.75+ to build from source
- **ffmpeg** — recording
- **mpv** — playback
- **streamlink** — Twitch stream resolution
- **yt-dlp** — YouTube / Patreon stream resolution

### Arch Linux (AUR)

```bash
paru -S strivo      # or: yay -S strivo
strivo doctor       # verify ffmpeg / mpv / streamlink / yt-dlp are installed
strivo              # starts daemon + webui on http://127.0.0.1:8181
```

### From source

```bash
git clone https://github.com/Chorosyne/strivo.git
cd strivo
cargo build --release
```

### Dev install (current checkout → `~/.local/bin/strivo`)

For hacking on a clone: build the latest from the working tree and drop
`strivo` on your `PATH`, with every first-party plugin enabled in a
generated default config.

```bash
scripts/install-dev.sh                # release build
scripts/install-dev.sh --debug        # faster iteration build
scripts/install-dev.sh --reconfigure  # rewrite the managed plugin block
scripts/install-dev.sh --uninstall    # remove installed bits (config kept)
```

The script:

- builds `strivo-bin`, copies the binary to `~/.local/bin/strivo`,
- ships the `whisperx_diarize.py` orchestrator next to it (auto-discovered
  by the `whisperx-local` backend),
- generates bash/zsh/fish completions and a manpage into
  `~/.local/share/strivo/`,
- writes `~/.config/strivo/config.toml` enabling `crunchr` + `archiver` on
  first run only (subsequent runs leave your edits alone unless you pass
  `--reconfigure`, which refreshes only the marker-bracketed block).

Override paths with `STRIVO_BIN_DIR`, `STRIVO_SHARE_DIR`, `STRIVO_CONFIG_DIR`.

(The previous `git submodule update --init` step is no longer needed —
the first-party plugins live in `crates/strivo-plugins/` in this repo.)

The binary lands at `target/release/strivo`. Copy it onto your `PATH`.

### Platform credentials

Complete the web UI's setup flow on first launch, or configure manually:

| Platform | How to get credentials |
|----------|------------------------|
| Twitch | Create an app at [dev.twitch.tv/console](https://dev.twitch.tv/console) — need `client_id` and `client_secret` |
| YouTube | Create OAuth credentials at the [Google Cloud Console](https://console.cloud.google.com/) — need `client_id` and `client_secret` |
| Patreon | Uses the membership API via browser cookies |

Credentials are stored in your OS keyring (macOS Keychain, GNOME Keyring /
Secret Service, Windows Credential Manager).

## Usage

### TUI

```bash
strivo
```

Arrow keys + Enter to navigate. The sidebar shows monitored channels with
live-status indicators. `a` toggles auto-record on a channel; `/` opens search.

### Daemon

```bash
strivo daemon start     # start the background service
strivo daemon stop      # stop it
strivo daemon status    # report whether it is running
strivo daemon install   # write a systemd user unit
```

When the daemon is running, `strivo` launches as a client that connects to
the Unix socket. See [docs/DAEMON.md](./docs/DAEMON.md) for socket paths,
logging, and lifecycle details.

### CLI

```bash
strivo config list              # show all settings
strivo config get <key>         # read a value
strivo config set <key> <val>   # write a value
strivo config path              # print the config file location
strivo config reset             # restore defaults (keeps credentials)

strivo log tail [-l 100]        # live-tail the log
strivo log path                 # print the log file location
strivo log clear                # wipe the log
```

### Flags

| Flag | Description |
|------|-------------|
| `-c, --config <path>` | Custom config file |
| `-l, --log-level <level>` | `trace`, `debug`, `info`, `warn`, `error` |

`RUST_LOG` is also honoured and overrides `-l` when set.

## Configuration

Config lives at `~/.config/strivo/config.toml` (XDG-compliant — see
`strivo config path` for the resolved location on your system). A fully
annotated reference is checked in at
[`config.toml.example`](./config.toml.example); a minimal working starting
point looks like:

```toml
recording_dir = "/home/you/Videos/strivo"
poll_interval_secs = 60

[twitch]
client_id = "..."
client_secret = "..."

[youtube]
client_id = "..."
client_secret = "..."
cookies_path = "/path/to/cookies.txt"   # optional, for members-only streams

[recording]
transcode = false
filename_template = "{channel}_{date}_{title}.mkv"

[[auto_record_channels]]
platform = "twitch"
channel_id = "12345"
channel_name = "streamer_name"

[[schedules]]
platform = "twitch"
channel_id = "12345"
cron = "0 20 * * 1-5"   # weekdays at 8pm
```

## Architecture

```
Twitch / YouTube / Patreon APIs
              │
              ▼
   ┌─────────────────────────┐
   │        Monitor          │
   │  polling, go-live detect│
   └────────┬────────────────┘
            │
       ┌────▼────┐    ┌──────────┐
       │Recorder │───▶│  Plugin  │
       │ ffmpeg  │    │ Crunchr  │
       │ yt-dlp  │    │ Archiver │
       └────┬────┘    └──────────┘
            │
       ┌────▼────┐    ┌──────────┐
       │Playback │    │ Web UI   │
       │  mpv    │◀──▶│ (SPA)    │
       └─────────┘    └──────────┘
```

```
strivo/                        cargo workspace root
├── src/                       strivo-core (library crate)
│   ├── platform/              Trait-based abstraction (Twitch, YouTube, Patreon)
│   ├── monitor/               Channel polling, go-live detection
│   ├── recording/             Job lifecycle, ffmpeg / yt-dlp process management
│   ├── stream/                URL resolution via streamlink / yt-dlp
│   ├── playback/              mpv controller
│   ├── plugin/                Plugin trait, registry, lifecycle
│   ├── intents/               Recording-intent translators (Start, DownloadVod)
│   ├── events.rs              DaemonEvent — IPC broadcast envelope
│   ├── daemon.rs              Background service, Unix-socket listener
│   ├── ipc.rs                 Client-server protocol
│   └── config/                TOML config, OS-keyring integration
├── crates/strivo-bin/         Binary crate (CLI, main.rs)
└── crates/strivo-plugins/     First-party plugins (Crunchr, Archiver,
                               Insights, Editor, Viewguard)
```

The dependency graph is strictly one-way:
`strivo-core` ← `strivo-plugins` ← `strivo-bin`. The core crate has no
awareness of concrete plugins; the binary pulls both together.

## Design rationale

| Decision | Reasoning |
|----------|-----------|
| Platform trait | Adding a new service means implementing one trait — auth, polling, and recording are decoupled from platform specifics |
| Unix-socket IPC | Zero-overhead daemon / client split; the TUI is just another client and headless recording works standalone |
| TUI-first | Terminal-native workflow stays fast, composable, and SSH-friendly. A complementary *arr-style web UI (`strivo serve`) is on the `feat/webui` branch — it talks to the same daemon over the existing socket |
| Plugin event bus | Transcription and archival react to recording events without coupling to the recording pipeline |
| OS keyring | Credentials never touch disk as plaintext — uses platform-native secret storage |

## Known limitations (0.3.0 alpha)

- **Daemon mode is Unix-only.** Linux and macOS work; Windows users can run
  the TUI against locally resolved streams but cannot attach to a daemon
  until the named-pipe transport lands.
- **In-flight recordings are not durable across daemon crashes.** A persisted
  journal exists for status replay but does not yet recover an in-flight
  ffmpeg process; the durability work is tracked in M1 / 0.4.0.
- **Transcription jobs cannot be cancelled or retried** after timeout — a
  single failure currently terminates the job.
- **Plugins require same-toolchain compilation** against the exact strivo
  build that loads them. Third-party plugins are not recommended for end
  users in alpha; see
  [docs/PLUGIN-MANIFEST.md](./docs/PLUGIN-MANIFEST.md).

## Documentation

- [docs/FIRST-RUN.md](./docs/FIRST-RUN.md) — log paths, common failure modes
- [docs/DAEMON.md](./docs/DAEMON.md) — daemon lifecycle, systemd integration, socket location
- [docs/PLUGIN-MANIFEST.md](./docs/PLUGIN-MANIFEST.md) — plugin trait, ABI caveats
- [docs/PLUGIN-TEMPLATE.md](./docs/PLUGIN-TEMPLATE.md) — minimal plugin skeleton
- [docs/SETTINGS-COVERAGE.md](./docs/SETTINGS-COVERAGE.md) — which config fields are surfaced in the settings UI

## Contributing

Bug reports and small fixes are welcome — see
[CONTRIBUTING.md](./CONTRIBUTING.md) for the local-build flow and project
conventions. Security issues should be reported privately via
[SECURITY.md](./SECURITY.md), not as public issues.

## Roadmap

Roadmap, milestones, and explicit deferrals live in
[ROADMAP.md](./ROADMAP.md).

## License

[MIT](./LICENSE)

## Credits

The web UI's topbar icons are vendored from
[EliverLara/candy-icons](https://github.com/EliverLara/candy-icons)
(GPL-3.0) by Eliver Lara — see
`crates/strivo-web/assets/icons/candy/` for the upstream LICENSE and
per-icon attribution.
