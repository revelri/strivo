# StreaVo

TUI Live Stream PVR for Twitch and YouTube. Monitor channels, automatically record live streams, and play them back — all from your terminal.

![Rust](https://img.shields.io/badge/rust-2021-orange) ![Version](https://img.shields.io/badge/version-0.1.0-blue) ![License](https://img.shields.io/badge/license-MIT-green)

## Features

- **Multi-platform** — Twitch and YouTube support with a pluggable platform trait
- **TUI dashboard** — sidebar navigation, channel details, recording list, live log viewer, settings panel, and setup wizard
- **Auto-recording** — configure channels to record automatically when they go live
- **Live playback** — watch streams directly via mpv
- **Stream resolution** — resolves stream URLs through streamlink and yt-dlp
- **FFmpeg recording** — captures streams to MKV with optional transcoding
- **Configurable filenames** — template-based output naming (`{channel}_{date}_{title}.mkv`)
- **Credential management** — platform secrets stored in your OS keyring
- **Desktop notifications** — get notified when monitored channels go live
- **CLI config management** — inspect and modify settings without opening the TUI

## Requirements

- **Rust** 1.75+ (2021 edition)
- **FFmpeg** — stream recording
- **mpv** — live playback
- **streamlink** and/or **yt-dlp** — stream URL resolution

### Platform API credentials

| Platform | Required |
|----------|----------|
| Twitch   | `client_id`, `client_secret` from [dev.twitch.tv](https://dev.twitch.tv/console) |
| YouTube  | `client_id`, `client_secret` from [Google Cloud Console](https://console.cloud.google.com/); optional `cookies_path` for authenticated access |

## Installation

```sh
git clone https://github.com/revelri/StreaVo.git
cd StreaVo
cargo build --release
```

The binary lands at `target/release/streavo`.

## Usage

### Launch the TUI

```sh
streavo
```

Use arrow keys and Enter to navigate the sidebar. The setup wizard runs on first launch to configure platform credentials and recording directory.

### CLI subcommands

```sh
# Configuration
streavo config list            # show all settings
streavo config path            # print config file location
streavo config get <key>       # read a value
streavo config set <key> <val> # write a value
streavo config reset           # restore defaults (keeps credentials)

# Logging
streavo log path               # print log file location
streavo log clear              # wipe the log
streavo log tail [-l 100]      # live-tail the log file
```

### Global flags

| Flag | Description |
|------|-------------|
| `-c, --config <path>` | Custom config file path |
| `-l, --log-level <level>` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |

## Configuration

Config lives at `~/.config/streavo/config.toml` (XDG on Linux, platform-native elsewhere).

```toml
recording_dir = "/home/you/Videos/StreaVo"
poll_interval_secs = 60

[twitch]
client_id = "..."
client_secret = "..."

[youtube]
client_id = "..."
client_secret = "..."
cookies_path = "/path/to/cookies.txt"   # optional

[recording]
transcode = false
filename_template = "{channel}_{date}_{title}.mkv"

[[auto_record_channels]]
platform = "twitch"
channel_id = "12345"
channel_name = "streamer_name"
```

## Project structure

```
src/
  main.rs            — entry point, logging setup, CLI dispatch
  app.rs             — application state, event loop, action handling
  cli.rs             — clap argument definitions
  config/            — TOML config loading/saving, keyring credentials
  platform/          — Twitch & YouTube API clients (trait-based)
  monitor/           — channel polling and go-live detection
  recording/         — recording manager, job definitions, FFmpeg wrapper
  playback/          — mpv controller
  stream/            — stream info types and URL resolver
  tui/               — ratatui rendering, event system, theme
    widgets/         — sidebar, channel detail, recording list, settings,
                       log viewer, dialogs, wizard, status bar
```

## License

MIT
