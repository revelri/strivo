//! MKV chapter embedding via `mkvpropedit` (M5.3).
//!
//! `mkvpropedit` is part of the `mkvtoolnix` suite and lets us embed
//! chapter markers into a Matroska file without re-muxing. Chapter
//! data is provided as a simple XML document.
//!
//! Today the helper exposes two surfaces:
//! - `embed_chapters(path, chapters)` — sized list of `(start_sec, title)`
//!   tuples → temp XML → mkvpropedit subprocess
//! - `every_n_minutes(duration_secs, n)` — utility for generating
//!   time-based chapters at fixed intervals when no semantic source
//!   (Crunchr topics) is available
//!
//! The Crunchr-driven topic-to-time chaptering still needs a schema
//! change (chapters per topic), so this commit lands the substrate;
//! the wire-up follows once Crunchr stores topic timestamps.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
pub struct Chapter {
    pub start_secs: f64,
    pub title: String,
}

/// Build the XML body Matroska's `--chapters` flag expects. The schema
/// is well-known and documented in the mkvtoolnix manual; see
/// matroska-chapters DTD.
pub fn chapters_xml(chapters: &[Chapter]) -> String {
    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str("<Chapters>\n  <EditionEntry>\n");
    for (i, ch) in chapters.iter().enumerate() {
        let secs = ch.start_secs.max(0.0);
        let h = (secs / 3600.0) as u64;
        let m = ((secs % 3600.0) / 60.0) as u64;
        let s = secs % 60.0;
        let timestamp = format!("{h:02}:{m:02}:{s:06.3}");
        let escaped_title = escape_xml(&ch.title);
        out.push_str(&format!(
            "    <ChapterAtom>\n      <ChapterUID>{}</ChapterUID>\n      <ChapterTimeStart>{}</ChapterTimeStart>\n      <ChapterDisplay>\n        <ChapterString>{}</ChapterString>\n        <ChapterLanguage>eng</ChapterLanguage>\n      </ChapterDisplay>\n    </ChapterAtom>\n",
            // ChapterUID must be a positive integer unique within the file.
            (i as u64) + 1,
            timestamp,
            escaped_title,
        ));
    }
    out.push_str("  </EditionEntry>\n</Chapters>\n");
    out
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Embed the chapter list into the MKV file via `mkvpropedit
/// --chapters <xml>`. The temp XML is written to a sibling file and
/// deleted on success; on failure it's left for the user to inspect.
pub fn embed_chapters(file: &Path, chapters: &[Chapter]) -> Result<()> {
    if chapters.is_empty() {
        return Err(anyhow!("refusing to embed empty chapter list"));
    }
    if !file.exists() {
        return Err(anyhow!("file does not exist: {}", file.display()));
    }
    let xml = chapters_xml(chapters);
    let xml_path = file.with_extension("chapters.xml");
    {
        let mut f = std::fs::File::create(&xml_path)
            .with_context(|| format!("create chapter xml {}", xml_path.display()))?;
        f.write_all(xml.as_bytes())
            .with_context(|| format!("write chapter xml {}", xml_path.display()))?;
    }
    let status = Command::new("mkvpropedit")
        .arg(file)
        .arg("--chapters")
        .arg(&xml_path)
        .status()
        .with_context(|| "spawn mkvpropedit (is mkvtoolnix installed?)")?;
    if !status.success() {
        return Err(anyhow!(
            "mkvpropedit exited with status {status} (xml left at {})",
            xml_path.display()
        ));
    }
    let _ = std::fs::remove_file(&xml_path);
    Ok(())
}

/// Emit one chapter every `n` minutes between 0 and `duration_secs`.
/// Each chapter's title is `"Part 1"`, `"Part 2"`, … — pure interval
/// scaffolding for callers without semantic source data.
pub fn every_n_minutes(duration_secs: f64, n: u64) -> Vec<Chapter> {
    if duration_secs <= 0.0 || n == 0 {
        return Vec::new();
    }
    let step = (n * 60) as f64;
    let mut out = Vec::new();
    let mut t = 0.0_f64;
    let mut idx: u64 = 1;
    while t < duration_secs {
        out.push(Chapter {
            start_secs: t,
            title: format!("Part {idx}"),
        });
        t += step;
        idx += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_well_formed() {
        let chs = vec![
            Chapter { start_secs: 0.0, title: "Intro".into() },
            Chapter { start_secs: 754.0, title: "Q&A".into() },
        ];
        let xml = chapters_xml(&chs);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<ChapterTimeStart>00:00:00.000</ChapterTimeStart>"));
        assert!(xml.contains("<ChapterTimeStart>00:12:34.000</ChapterTimeStart>"));
        assert!(xml.contains("<ChapterString>Q&amp;A</ChapterString>"));
    }

    #[test]
    fn every_n_minutes_step() {
        let chs = every_n_minutes(31.0 * 60.0, 10);
        assert_eq!(chs.len(), 4);
        assert_eq!(chs[0].start_secs, 0.0);
        assert_eq!(chs[1].start_secs, 600.0);
        assert_eq!(chs[3].start_secs, 1800.0);
    }

    #[test]
    fn empty_duration_no_chapters() {
        assert!(every_n_minutes(0.0, 5).is_empty());
        assert!(every_n_minutes(300.0, 0).is_empty());
    }
}
