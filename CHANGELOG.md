# Changelog

All notable changes to StreaVo will be documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-03-14

### Added

- TUI dashboard with sidebar navigation, channel detail view, recording list, settings panel, and status bar
- Setup wizard for first-run configuration
- Twitch platform integration (OAuth app flow, channel lookup, live status polling)
- YouTube platform integration (Data API v3, live broadcast detection, cookie-based auth)
- FFmpeg-based stream recording with MKV output
- Optional video transcoding pipeline
- Configurable filename templates (`{channel}_{date}_{title}.mkv`)
- Auto-record support for configured channels
- Live playback through mpv
- Stream URL resolution via streamlink and yt-dlp
- Channel monitoring with configurable poll interval
- Desktop notifications on go-live events
- TOML configuration with XDG-compliant paths
- OS keyring credential storage
- Live log viewer widget in the TUI
- CLI subcommands for config management (`config list/get/set/path/reset`)
- CLI subcommands for log management (`log path/clear/tail`)
- Dialog system for confirmations and input
- Color theme system for the TUI
