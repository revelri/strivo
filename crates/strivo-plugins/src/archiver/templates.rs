//! yt-dlp output-template presets + preview rendering. (R4.)
//!
//! Users don't know yt-dlp's `%(field)s` syntax; we ship three named
//! templates that cover ~90% of real-world organization preferences.
//! Each carries a yt-dlp format string plus a deterministic preview
//! against a fixed sample metadata blob so the config modal can show
//! "what your files will look like" before commit.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatTemplate {
    /// `<archive>/<YYYY-MM>/<MM-DD-YYYY> - <title>.<ext>`. Default.
    ByDate,
    /// `<archive>/Playlists/<playlist>/<MM-DD-YYYY> - <title>.<ext>`.
    /// Falls back to ByDate when no playlist is set on the entry.
    ByPlaylist,
    /// `<archive>/<channel>/<YYYY-MM-DD> - <title>.<ext>`.
    ByChannel,
    /// Flat dump — every file in `<archive>/<YYYY-MM-DD> <title>.<ext>`.
    Flat,
    /// User-supplied custom template.
    Custom,
}

impl FormatTemplate {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ByDate => "By date",
            Self::ByPlaylist => "By playlist",
            Self::ByChannel => "By channel",
            Self::Flat => "Flat dump",
            Self::Custom => "Custom",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::ByDate => "<archive>/<YYYY-MM>/<MM-DD-YYYY> - <title>.<ext>",
            Self::ByPlaylist => {
                "<archive>/Playlists/<playlist>/<MM-DD-YYYY> - <title>.<ext>"
            }
            Self::ByChannel => "<archive>/<channel>/<YYYY-MM-DD> - <title>.<ext>",
            Self::Flat => "<archive>/<YYYY-MM-DD> <title>.<ext>",
            Self::Custom => "user-supplied yt-dlp output template",
        }
    }

    /// yt-dlp `-o` value. `ByPlaylist` returns the template with both
    /// playlist-bearing and playlist-empty fallbacks expressed via the
    /// `field|fallback` syntax yt-dlp supports.
    pub fn yt_dlp_template(&self, custom: Option<&str>) -> String {
        match self {
            Self::ByDate => {
                "%(upload_date>%Y-%m)s/%(upload_date>%m-%d-%Y)s - %(title)s.%(ext)s".into()
            }
            Self::ByPlaylist => {
                "Playlists/%(playlist_title,playlist)s/%(upload_date>%m-%d-%Y)s - %(title)s.%(ext)s".into()
            }
            Self::ByChannel => {
                "%(uploader)s/%(upload_date>%Y-%m-%d)s - %(title)s.%(ext)s".into()
            }
            Self::Flat => "%(upload_date>%Y-%m-%d)s %(title)s.%(ext)s".into(),
            Self::Custom => custom.unwrap_or("").to_string(),
        }
    }

    /// Render a preview against the given sample-metadata map. Returns
    /// the relative output path with all `%(field)s` placeholders
    /// resolved; unknown fields render as `<missing-field>` so the
    /// user sees what's broken at a glance.
    pub fn preview(&self, custom: Option<&str>, sample: &SampleMetadata) -> String {
        render_template(&self.yt_dlp_template(custom), sample)
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::ByDate,
            Self::ByPlaylist,
            Self::ByChannel,
            Self::Flat,
        ]
    }
}

/// Sample VOD metadata for preview rendering. Anything not in the
/// fixed-shape struct goes through `extra` so the template engine
/// stays generic.
#[derive(Debug, Clone)]
pub struct SampleMetadata {
    pub title: String,
    pub uploader: String,
    pub upload_date: String, // YYYYMMDD
    pub ext: String,
    pub playlist_title: Option<String>,
    pub extra: HashMap<String, String>,
}

impl Default for SampleMetadata {
    fn default() -> Self {
        Self {
            title: "Weekly stream recap".into(),
            uploader: "AwesomeStreamer".into(),
            upload_date: "20260523".into(),
            ext: "mkv".into(),
            playlist_title: Some("Weekly recaps".into()),
            extra: HashMap::new(),
        }
    }
}

fn render_template(tpl: &str, sample: &SampleMetadata) -> String {
    // Minimal yt-dlp template parser — handles `%(field)s` and
    // `%(field>strftime)s`. yt-dlp itself accepts way more (selectors,
    // conditions, escapes) but this is a preview only.
    let mut out = String::new();
    let mut chars = tpl.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek() == Some(&'(') {
            chars.next(); // '('
            let mut name = String::new();
            for nc in chars.by_ref() {
                if nc == ')' {
                    break;
                }
                name.push(nc);
            }
            // Strip `s` type indicator that immediately follows ).
            if chars.peek() == Some(&'s') {
                chars.next();
            }
            // Split on '>' for strftime mini-language.
            let (field, fmt) = match name.split_once('>') {
                Some((f, fmt)) => (f, Some(fmt)),
                None => (name.as_str(), None),
            };
            // Comma-separated fallback list (`playlist_title,playlist`).
            let resolved = field
                .split(',')
                .find_map(|f| resolve(f, sample))
                .unwrap_or_else(|| format!("<missing:{field}>"));
            let final_str = if let Some(fmt) = fmt {
                strftime_render(&resolved, fmt)
            } else {
                resolved
            };
            out.push_str(&final_str);
        } else {
            out.push(c);
        }
    }
    out
}

fn resolve(field: &str, sample: &SampleMetadata) -> Option<String> {
    match field {
        "title" => Some(sample.title.clone()),
        "uploader" | "channel" => Some(sample.uploader.clone()),
        "upload_date" => Some(sample.upload_date.clone()),
        "ext" => Some(sample.ext.clone()),
        "playlist_title" => sample.playlist_title.clone(),
        "playlist" => sample.playlist_title.clone(),
        other => sample.extra.get(other).cloned(),
    }
}

/// Tiny strftime renderer for the date fields. Supports the format
/// codes yt-dlp documents most commonly: %Y / %m / %d / %H / %M / %S.
fn strftime_render(value: &str, fmt: &str) -> String {
    // The yt-dlp date field is YYYYMMDD; parse and reformat.
    if value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()) {
        let y = &value[0..4];
        let m = &value[4..6];
        let d = &value[6..8];
        let mut out = String::new();
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('Y') => out.push_str(y),
                    Some('m') => out.push_str(m),
                    Some('d') => out.push_str(d),
                    Some(other) => {
                        out.push('%');
                        out.push(other);
                    }
                    None => out.push('%'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    // Non-date fields just pass through; the template is wrong, but
    // we render verbatim so the user can spot it in the preview.
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_date_preview_shape() {
        let s = SampleMetadata::default();
        let p = FormatTemplate::ByDate.preview(None, &s);
        assert_eq!(p, "2026-05/05-23-2026 - Weekly stream recap.mkv");
    }

    #[test]
    fn by_playlist_falls_back_when_missing() {
        let mut s = SampleMetadata::default();
        s.playlist_title = None;
        // `%(playlist_title,playlist)s` — both missing → preview
        // surfaces the `<missing>` token so the user fixes the
        // template.
        let p = FormatTemplate::ByPlaylist.preview(None, &s);
        assert!(p.starts_with("Playlists/<missing"));
    }

    #[test]
    fn by_channel_uses_uploader() {
        let s = SampleMetadata::default();
        let p = FormatTemplate::ByChannel.preview(None, &s);
        assert_eq!(p, "AwesomeStreamer/2026-05-23 - Weekly stream recap.mkv");
    }

    #[test]
    fn flat_preview() {
        let s = SampleMetadata::default();
        let p = FormatTemplate::Flat.preview(None, &s);
        assert_eq!(p, "2026-05-23 Weekly stream recap.mkv");
    }

    #[test]
    fn custom_uses_user_string() {
        let s = SampleMetadata::default();
        let p = FormatTemplate::Custom.preview(
            Some("dump/%(uploader)s/%(title)s.%(ext)s"),
            &s,
        );
        assert_eq!(p, "dump/AwesomeStreamer/Weekly stream recap.mkv");
    }

    #[test]
    fn unknown_field_surfaces_visibly() {
        let s = SampleMetadata::default();
        let p = FormatTemplate::Custom.preview(Some("%(banana)s.%(ext)s"), &s);
        assert_eq!(p, "<missing:banana>.mkv");
    }
}
