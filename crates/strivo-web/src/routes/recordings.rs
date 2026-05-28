//! Recording file-serving endpoints.
//!
//! - GET /api/v1/recordings/<id>/download   raw file stream (range requests)
//! - GET /api/v1/recordings/<id>/play       redirect to /download
//!
//! Earlier iterations of this module rendered the recordings page server-
//! side via askama; that surface was retired when the SPA took over. The
//! file-serving handlers, the path-containment guard (with its tests), and
//! the extension → Content-Type map remain because they're the only path
//! through which the webui's player and download links touch real bytes on
//! disk.

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use strivo_core::ipc::ServerMessage;
use uuid::Uuid;

use crate::server::AppState;

async fn lookup_path(state: &AppState, id: Uuid) -> Result<PathBuf, String> {
    let snap = state.ipc.snapshot().await.map_err(|e| e.to_string())?;
    let ServerMessage::StateSnapshot { recordings, .. } = snap else {
        return Err("unexpected ServerMessage".into());
    };
    recordings
        .get(&id)
        .map(|j| j.output_path.clone())
        .ok_or_else(|| "recording not found".into())
}

/// Reject any path that, once canonicalised, escapes the recording root.
/// `output_path` is daemon-set, but a corrupted snapshot/DB (or a future
/// caller that does take user input) must never let the web process stream
/// a file outside the recording directory — symlinks included.
fn contain_in_root(
    candidate: &std::path::Path,
    root: &std::path::Path,
) -> Result<PathBuf, StatusCode> {
    let real_root = root
        .canonicalize()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let real = candidate.canonicalize().map_err(|_| StatusCode::NOT_FOUND)?;
    if real.starts_with(&real_root) {
        Ok(real)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Sniff the actual container by reading the file's leading bytes and
/// matching magic signatures. Beats extension-based guessing when a file
/// has been mis-renamed (e.g. `foo.mkv` that's actually an MP3 inside —
/// Firefox refuses the mismatch). Returns None if no signature matches;
/// caller falls back to `guess_mime`.
async fn sniff_mime(path: &std::path::Path) -> Option<&'static str> {
    use tokio::io::AsyncReadExt;
    const HEAD: usize = 4096;
    let mut buf = vec![0u8; HEAD];
    let mut f = tokio::fs::File::open(path).await.ok()?;
    let n = f.read(&mut buf).await.ok()?;
    if n == 0 {
        return None;
    }
    buf.truncate(n);
    Some(detect_mime(&buf)?)
}

/// Pure-byte signature dispatch — split out so it can be unit-tested
/// against fabricated headers without writing files.
fn detect_mime(buf: &[u8]) -> Option<&'static str> {
    // EBML — matroska or webm. Look for the DocType ascii anywhere in the
    // EBML header (first ~256 bytes); the literal "webm" / "matroska" lives
    // a few bytes past the DocType element id (0x4282) and a length VINT.
    if buf.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        if buf.windows(4).take(256).any(|w| w == b"webm") {
            return Some("video/webm");
        }
        return Some("video/x-matroska");
    }
    // MP4 family — `ftyp` box at offset 4..8; brand at 8..12 distinguishes
    // audio (M4A/M4B/M4P) from video (everything else) from QuickTime.
    if buf.len() >= 12 && &buf[4..8] == b"ftyp" {
        return Some(match &buf[8..12] {
            b"M4A " | b"M4B " | b"M4P " => "audio/mp4",
            b"qt  " => "video/quicktime",
            _ => "video/mp4",
        });
    }
    // ID3-tagged MP3 (most encoders prepend an ID3v2 header).
    if buf.starts_with(b"ID3") {
        return Some("audio/mpeg");
    }
    // MPEG audio frame sync — 11 high bits set (0xFFE0). Covers MP3 + AAC
    // ADTS, both of which Firefox/Chrome play as audio/mpeg / audio/aac;
    // we collapse to audio/mpeg since browsers accept it for both.
    if buf.len() >= 2 && buf[0] == 0xFF && (buf[1] & 0xE0) == 0xE0 {
        return Some("audio/mpeg");
    }
    // FLAC stream marker.
    if buf.starts_with(b"fLaC") {
        return Some("audio/flac");
    }
    // OGG container — covers Vorbis + Opus; browsers decode either.
    if buf.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    // RIFF/WAVE — RIFF<size>WAVE.
    if buf.starts_with(b"RIFF") && buf.len() >= 12 && &buf[8..12] == b"WAVE" {
        return Some("audio/wav");
    }
    // MPEG-TS — 0x47 sync byte at offsets 0 and 188 (one packet apart).
    // The double-check beats false-positive single-byte matches.
    if buf.len() >= 189 && buf[0] == 0x47 && buf[188] == 0x47 {
        return Some("video/mp2t");
    }
    None
}

/// Map a file extension to a Content-Type the browser will play happily.
/// Old behaviour hard-coded `video/x-matroska` on every download, which (a)
/// is wrong for audio-only pulls (yt-dlp may write .m4a / .mp3 / .opus when
/// the source is a Patreon audio post) and (b) Firefox refuses the mismatch.
fn guess_mime(p: &std::path::Path) -> &'static str {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("mkv") => "video/x-matroska",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("ts") => "video/mp2t",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        Some("m4a") => "audio/mp4",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("aac") => "audio/aac",
        _ => "application/octet-stream",
    }
}

async fn download(State(state): State<AppState>, Path(id): Path<Uuid>) -> Response {
    let raw = match lookup_path(&state, id).await {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, e).into_response(),
    };
    // Containment check before opening: canonicalise against the configured
    // recording root and refuse anything that escapes it.
    let root = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c.recording_dir,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let path = match contain_in_root(&raw, &root) {
        Ok(p) => p,
        Err(code) => return (code, "path outside recording directory").into_response(),
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let len = file.metadata().await.map(|m| m.len()).ok();
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("recording.mkv");
    // Prefer magic-byte sniffing over extension-based guessing — a file
    // mis-named foo.mkv that actually contains MP3 audio inside would
    // otherwise get video/x-matroska and Firefox refuses the mismatch.
    let mime = match sniff_mime(&path).await {
        Some(m) => m,
        None => guess_mime(&path),
    };
    let mut resp = Response::new(body);
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        mime.parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        format!("inline; filename=\"{filename}\"")
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("inline")),
    );
    if let Some(l) = len {
        if let Ok(v) = header::HeaderValue::from_str(&l.to_string()) {
            resp.headers_mut().insert(header::CONTENT_LENGTH, v);
        }
    }
    resp
}

async fn play(Path(id): Path<Uuid>) -> Redirect {
    Redirect::temporary(&format!("/api/v1/recordings/{id}/download"))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/recordings/{id}/download", get(download))
        .route("/api/v1/recordings/{id}/play", get(play))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("strivo-contain-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn allows_file_inside_root() {
        let root = temp_root("inside");
        let file = root.join("rec.mkv");
        fs::write(&file, b"x").unwrap();
        let got = contain_in_root(&file, &root).unwrap();
        assert!(got.starts_with(root.canonicalize().unwrap()));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_traversal_outside_root() {
        let root = temp_root("escape");
        let outside = root.join("..").join("..").join("etc").join("hostname");
        let err = contain_in_root(&outside, &root).unwrap_err();
        assert!(err == StatusCode::FORBIDDEN || err == StatusCode::NOT_FOUND);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_symlink_escape() {
        let root = temp_root("symlink");
        let secret = temp_root("symlink-secret");
        let secret_file = secret.join("secret.txt");
        fs::write(&secret_file, b"top secret").unwrap();
        let link = root.join("link.mkv");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret_file, &link).unwrap();
            assert_eq!(contain_in_root(&link, &root).unwrap_err(), StatusCode::FORBIDDEN);
        }
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&secret).ok();
    }

    #[test]
    fn detect_mime_recognises_real_signatures() {
        // EBML — matroska vs webm
        assert_eq!(
            detect_mime(&[0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01]),
            Some("video/x-matroska"),
        );
        // EBML with DocType "webm" anywhere in the header window
        let mut webm = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x82, 0x84];
        webm.extend_from_slice(b"webm");
        assert_eq!(detect_mime(&webm), Some("video/webm"));

        // MP4 family ftyp brands
        let mut hdr = [0u8; 16];
        hdr[4..8].copy_from_slice(b"ftyp");
        hdr[8..12].copy_from_slice(b"M4A ");
        assert_eq!(detect_mime(&hdr), Some("audio/mp4"));
        hdr[8..12].copy_from_slice(b"qt  ");
        assert_eq!(detect_mime(&hdr), Some("video/quicktime"));
        hdr[8..12].copy_from_slice(b"isom");
        assert_eq!(detect_mime(&hdr), Some("video/mp4"));

        // MP3 — ID3 prefix
        assert_eq!(detect_mime(b"ID3\x03\x00\x00\x00\x00\x00\x00"), Some("audio/mpeg"));
        // MP3 — bare MPEG sync
        assert_eq!(detect_mime(&[0xFF, 0xFB, 0x90, 0x00]), Some("audio/mpeg"));

        // FLAC / OGG / WAV
        assert_eq!(detect_mime(b"fLaC\0\0\0\0"), Some("audio/flac"));
        assert_eq!(detect_mime(b"OggS\0\0\0\0\0\0\0\0"), Some("audio/ogg"));
        let mut wav = vec![0u8; 12];
        wav[..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        assert_eq!(detect_mime(&wav), Some("audio/wav"));

        // MPEG-TS — sync at 0 and 188
        let mut ts = vec![0u8; 200];
        ts[0] = 0x47;
        ts[188] = 0x47;
        assert_eq!(detect_mime(&ts), Some("video/mp2t"));

        // Unrecognised
        assert_eq!(detect_mime(b"random garbage"), None);
        assert_eq!(detect_mime(&[]), None);
    }

    #[test]
    fn mime_map_covers_audio_and_video_extensions() {
        let cases = [
            ("/tmp/x.mkv", "video/x-matroska"),
            ("/tmp/x.mp4", "video/mp4"),
            ("/tmp/x.webm", "video/webm"),
            ("/tmp/x.m4a", "audio/mp4"),
            ("/tmp/x.mp3", "audio/mpeg"),
            ("/tmp/x.opus", "audio/ogg"),
            ("/tmp/x.flac", "audio/flac"),
            ("/tmp/x.unknown", "application/octet-stream"),
        ];
        for (path, want) in cases {
            assert_eq!(guess_mime(std::path::Path::new(path)), want, "for {path}");
        }
    }
}
