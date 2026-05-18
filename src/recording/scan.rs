use crate::config::AppConfig;
use crate::platform::PlatformKind;
use crate::recording::job::RecordingJob;

/// Scan the recording directory for existing video files and create RecordingJob entries.
pub fn scan_existing_recordings(config: &AppConfig) -> Vec<RecordingJob> {
    let dir = &config.recording_dir;
    if !dir.exists() {
        return Vec::new();
    }

    let extensions = ["mkv", "mp4", "webm", "ts"];

    let mut jobs = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to read recording directory {}: {e}", dir.display());
            return Vec::new();
        }
    };

    let template = &config.recording.filename_template;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !extensions.contains(&ext) {
            continue;
        }

        let filename = match path.file_stem().and_then(|s| s.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };

        let parsed = parse_filename(template, &filename);

        let meta = std::fs::metadata(&path).ok();
        let started_at = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<chrono::Utc>::from)
            .unwrap_or_else(chrono::Utc::now);

        let channel_name = parsed.channel.unwrap_or_else(|| filename.clone());
        let platform = parsed
            .platform
            .and_then(|p| match p.to_lowercase().as_str() {
                "twitch" => Some(PlatformKind::Twitch),
                "youtube" => Some(PlatformKind::YouTube),
                "patreon" => Some(PlatformKind::Patreon),
                _ => None,
            })
            .unwrap_or(PlatformKind::Twitch);

        let job = RecordingJob::from_file(path, channel_name, platform, parsed.title, started_at);
        jobs.push(job);
    }

    tracing::info!(
        "Scanned {} existing recordings from {}",
        jobs.len(),
        dir.display()
    );
    jobs
}

struct ParsedFilename {
    channel: Option<String>,
    title: Option<String>,
    platform: Option<String>,
}

/// Parse a filename according to the template pattern.
/// Template uses {channel}, {date}, {title}, {platform} placeholders.
fn parse_filename(template: &str, filename: &str) -> ParsedFilename {
    // Strip the extension from the template if present
    let template_stem = template
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(template);

    let result = try_template_parse(template_stem, filename);

    if result.channel.is_some() || result.title.is_some() {
        return result;
    }

    // Fallback: use filename as title
    ParsedFilename {
        channel: None,
        title: Some(filename.to_string()),
        platform: None,
    }
}

/// Simple template-based parsing without regex dependency.
/// Splits template and filename by underscore separators between placeholders.
fn try_template_parse(template: &str, filename: &str) -> ParsedFilename {
    let mut result = ParsedFilename {
        channel: None,
        title: None,
        platform: None,
    };

    // Default template: "{channel}_{date}_{title}"
    // Find the positions of placeholders and their separators
    let parts: Vec<&str> = template.split(|c: char| c == '{').collect();
    if parts.len() < 2 {
        return result;
    }

    // Parse template into segments: (separator, placeholder_name)
    let mut segments: Vec<(String, String)> = Vec::new();
    let mut prefix = parts[0].to_string();

    for part in &parts[1..] {
        if let Some((name, rest)) = part.split_once('}') {
            segments.push((prefix.clone(), name.to_string()));
            prefix = rest.to_string();
        }
    }

    if segments.is_empty() {
        return result;
    }

    // Try to extract values by splitting on the separators
    let mut remaining = filename;

    for (i, (sep, name)) in segments.iter().enumerate() {
        // Skip the separator prefix
        if i == 0 && !sep.is_empty() {
            if let Some(r) = remaining.strip_prefix(sep.as_str()) {
                remaining = r;
            } else {
                return result;
            }
        }

        // Find the next separator (the separator of the next segment)
        let next_sep = segments.get(i + 1).map(|(s, _)| s.as_str()).unwrap_or("");

        let value = if next_sep.is_empty() && i == segments.len() - 1 {
            // Last segment: take everything remaining
            let v = remaining;
            remaining = "";
            v
        } else if !next_sep.is_empty() {
            // Split on next separator
            if let Some((v, r)) = remaining.split_once(next_sep) {
                remaining = r;
                v
            } else {
                // Can't find separator, take everything
                let v = remaining;
                remaining = "";
                v
            }
        } else {
            remaining
        };

        match name.as_str() {
            "channel" => result.channel = Some(value.to_string()),
            "title" => result.title = Some(value.to_string()),
            "platform" => result.platform = Some(value.to_string()),
            "date" => {} // ignore
            _ => {}
        }
    }

    result
}
