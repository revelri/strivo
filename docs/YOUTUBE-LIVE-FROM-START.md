# YouTube Live-From-Start

## Why this exists

YouTube's live broadcasts expose a "DVR window" — when you join the
stream mid-broadcast, the player gives you a seekable timeline back to
broadcast start (assuming the streamer didn't disable DVR). yt-dlp
exposes this via `--live-from-start`:

> Download livestreams from the start. Currently only supported for
> YouTube (experimental).

We have wired it since 0.2 (`src/recording/mod.rs` → `RecordingCommand::Start`
with `from_start: true` routes to `YtDlpProcess::with_options(...,
live_from_start: true)`), but users reported recordings still landing at
the join-time slice and filenames coming out as bare `UC...` channel
IDs.

This document captures the fixes shipped under "YT-1 / YT-2 / YT-3"
and the trade-offs versus the Twitch live-from-start path
(`docs/TWITCH-LIVE-FROM-START.md`).

## What's different vs. Twitch

| Concern | Twitch | YouTube |
|---|---|---|
| Native API | None (HLS DVR ~5 min) | `--live-from-start` (yt-dlp) |
| Custom extractor needed? | **Yes** — Rewind playlist via GQL `PlaybackAccessToken(isVod=true)`; see TWITCH-LIVE-FROM-START.md | No — yt-dlp speaks the YouTube manifest |
| Bootstrap latency | 30–120 s (archive video_id appears after stream goes live) | None (`/live` alias resolves immediately) |
| Failure mode | Falls back to live-edge HLS + post-stream VOD backfill | Falls back to live-edge HLS via same yt-dlp invocation |
| TOS posture | Uses documented player traffic on accessible channels | Same — yt-dlp's standard usage |

The YouTube path is **structurally simpler** because yt-dlp does the
hard work. Our job reduces to (a) feeding yt-dlp the right URL form
and (b) propagating the right slug into the filename template.

## Root causes of the two reported symptoms

### Symptom 1 — recordings start at the join-time slice

`--live-from-start` was being requested but yt-dlp was downloading
from the live edge anyway. Root cause: the URL form we passed.

We were invoking yt-dlp with the channel-alias URL (
`https://www.youtube.com/channel/UC.../live` or
`https://www.youtube.com/@handle/live`). yt-dlp's extractor follows
the redirect to the underlying `watch?v=<id>` page but, in our
observation, races the redirect path with the stream's live-edge
cursor — sometimes landing at the broadcast start, sometimes at the
join-time slice.

**Fix (YT-2):** resolve the channel `/live` alias to a concrete
`/watch?v=<id>` URL *before* the recording yt-dlp invocation. We do
this with a fast pre-pass:

```text
yt-dlp --print id --no-warnings --no-download --no-playlist \
       --socket-timeout 20 \
       https://www.youtube.com/channel/UC.../live
```

That returns the broadcast's video ID (11 chars, base64-ish). We
compose `https://www.youtube.com/watch?v=<id>` and invoke yt-dlp's
recording call against it. `--live-from-start` reliably lands at
t=0 from a stable video URL.

Implementation: `src/recording/ytdlp.rs::resolve_live_video_id()`.
Hooked in `src/recording/mod.rs` for the
`PlatformKind::YouTube + from_start` branch. On resolution failure
(e.g. yt-dlp not installed, network blip, channel not actually
live), we **fall back to the original alias URL** rather than
killing the recording outright — the user gets join-time-slice
behavior as before but the recording still starts.

### Symptom 2 — filenames are bare alphanumeric strings

`build_output_path` was rendering the filename template with
`channel_name` = the YouTube channel ID (the 24-char `UC...` string),
not the human-readable channel display name. The default template
is `{channel}_{date}_{title}.mkv`, so the user saw files like:

```
UCxxxxxxxxxxxxxxxxxxxxxx_2026-05-23_153000_MyStreamTitle.mkv
```

instead of:

```
SomeChannelName_2026-05-23_153000_MyStreamTitle.mkv
```

Root cause: `ChannelEntry.name` is the platform's stable identifier
(needed for URL construction), while `ChannelEntry.display_name` is
the human-readable string. The recording layer was only carrying the
stable identifier through.

**Fix (YT-1):** add `display_name: Option<String>` to
`AppAction::StartRecording` and `RecordingCommand::Start`. The
recording manager prefers `display_name` when present, falls back to
`channel_name` otherwise (so schedule-fired starts that don't have a
separate display name continue to work).

This also fixes the symptom for Twitch when a streamer's display name
diverges from their login (lower-case canonical), and for Patreon
where the display name is a properly-cased creator name vs the
slug-friendly identifier.

## Safety nets

Beyond the URL-form fix, the recording invocation now also passes
`--wait-for-video 30` (YT-3). This gives yt-dlp a 30-second grace
period to find the broadcast when the stream is just coming online —
otherwise yt-dlp's default behavior is to fail immediately if the
video is "upcoming" rather than "live", which loses the first few
seconds to the user's reaction time.

## Verification

End-to-end check (requires a known YouTube channel to actually be
live at the time of test):

1. `strivo doctor` — confirm `yt-dlp` is in the doctor's tool list
   with an OK status.
2. Pick a known-live YouTube channel from the sidebar.
3. Press `r` (start recording from start). The status bar reads
   "Starting recording from stream start..."
4. Tail the log:
   `tail -f ~/.local/state/strivo/strivo.log | grep -E 'yt-dlp|live-from-start'`
5. Expected log lines, in order:
   - `INFO ... yt-dlp: resolved /live → /watch?v= for live-from-start`
     (the YT-2 pre-pass succeeded).
   - yt-dlp's own progress output mentions a "fragment" or
     "segment" count — for `--live-from-start` this starts at a high
     number (the entire DVR window) and counts down.
6. After ~30 s the output file should exist with a non-trivial size
   (several MB if the broadcast has been live for a while).
7. Verify the filename: should be
   `<display_name>_<YYYY-MM-DD_HHMMSS>_<title>.mkv`, NOT
   `UC...._<date>_<title>.mkv`.
8. After stopping (`s`), play the file: `mpv <path>`. Seek to t=0
   — the recording should start at broadcast start, not at the
   join-time slice.

Manual smoke-test of just the YT-2 pre-pass, without recording:

```sh
cargo run -- doctor   # warm-up
yt-dlp --print id --no-warnings --no-download --no-playlist \
       https://www.youtube.com/@<handle>/live
# Expected: an 11-char video ID printed to stdout.
```

## Known limitations

- **DVR disabled.** Some channels disable archive / DVR. For those,
  `--live-from-start` falls back to live-edge behavior. There is no
  client-side workaround. (Twitch's Rewind feature has the analogous
  limitation — `archiveVideo.id` is absent on rewind-disabled
  channels.)
- **Very long broadcasts.** yt-dlp's `--live-from-start` is marked
  experimental and is known to occasionally truncate multi-hour
  streams. The post-stream VOD backfill path (M5.7) is still the
  durable archive — it grabs the published `/watch?v=` VOD after the
  stream ends.
- **Membership-only streams.** Same as Twitch sub-only — requires
  `--cookies` and a logged-in account that already has access. The
  YouTube cookies path (`youtube.cookies_path` in config) is wired
  through.

## File map

- `src/recording/ytdlp.rs::resolve_live_video_id` — the YT-2 pre-pass.
- `src/recording/ytdlp.rs::YtDlpProcess::with_options` — added
  `--wait-for-video 30` under the `live_from_start` branch (YT-3).
- `src/recording/mod.rs` — YouTube + from_start branch resolves the
  video URL before spawning the recorder.
- `src/app.rs::AppAction::StartRecording` — now carries
  `display_name`.
- `src/recording/mod.rs::RecordingCommand::Start` — same.
- `src/recording/mod.rs::build_output_path` callsite — uses
  `display_name` when present.
