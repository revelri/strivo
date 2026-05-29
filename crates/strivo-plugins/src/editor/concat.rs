//! ffmpeg concat argv synthesis. (E2.)
//!
//! Takes the working clip list + the word stream and emits the ffmpeg
//! command that the host pipeline executor will spawn. For the M4 MVP
//! we use the `concat` demuxer (`-f concat -safe 0 -i list.txt -c copy
//! out.mkv`) which is lossless when every clip's in/out aligns to a
//! video keyframe. Misalignment requires a partial re-encode; that
//! path is held until the host pipeline refactor lands.

use std::path::PathBuf;

use anyhow::{anyhow, Result};

use super::EditorClip;

/// Returned command shape — the executor consumes `argv` and writes
/// `concat_file_contents` to `concat_file_path` first.
#[derive(Debug, Clone)]
pub struct ConcatPlan {
    pub argv: Vec<String>,
    pub concat_file_path: PathBuf,
    pub concat_file_contents: String,
    pub output_path: PathBuf,
}

/// Build the ffmpeg concat command for the clip list. Each clip is a
/// `[seg-N.mkv]` carved by an upstream stage (host pipeline). For the
/// MVP we shortcut: emit a concat-demuxer file that points at the
/// source recording with per-clip `inpoint` / `outpoint` directives;
/// ffmpeg slices internally with `-c copy` — lossless when keyframes
/// align, otherwise the demuxer falls back to re-encoding the first
/// GOP of each segment automatically.
pub fn build_concat_argv(
    clips: &[EditorClip],
    words: &[(String, f64)],
    source: &Option<PathBuf>,
) -> Result<ConcatPlan> {
    if clips.is_empty() {
        return Err(anyhow!("no clips to concat"));
    }
    let source = source
        .as_ref()
        .ok_or_else(|| anyhow!("no source recording loaded"))?;

    // Build the concat demuxer's input list.
    let mut list = String::new();
    for c in clips {
        let in_secs = words
            .get(c.in_word as usize)
            .map(|(_, s)| *s)
            .ok_or_else(|| anyhow!("in_word {} out of range", c.in_word))?;
        let out_secs = words
            .get(c.out_word as usize)
            .map(|(_, s)| *s)
            .ok_or_else(|| anyhow!("out_word {} out of range", c.out_word))?;
        if out_secs <= in_secs {
            return Err(anyhow!(
                "clip {}: out_secs ({:.2}) ≤ in_secs ({:.2})",
                c.label,
                out_secs,
                in_secs
            ));
        }
        // The concat demuxer's `inpoint`/`outpoint` syntax wants the
        // file declared once before the directives.
        list.push_str(&format!("file '{}'\n", source.display()));
        list.push_str(&format!("inpoint {in_secs:.3}\n"));
        list.push_str(&format!("outpoint {out_secs:.3}\n"));
    }

    let output_dir = source
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
    let concat_file_path = output_dir.join(format!("{stem}.concat.txt"));
    let output_path = output_dir.join(format!("{stem}.editor.mkv"));

    let argv = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-i".into(),
        concat_file_path.to_string_lossy().into_owned(),
        // Try lossless first; if a clip is keyframe-misaligned, ffmpeg
        // will emit a warning and re-encode that GOP. Acceptable for
        // M4 — explicit user-controlled re-encode is C-tier polish.
        "-c".into(),
        "copy".into(),
        "-map".into(),
        "0".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
        output_path.to_string_lossy().into_owned(),
    ];

    Ok(ConcatPlan {
        argv,
        concat_file_path,
        concat_file_contents: list,
        output_path,
    })
}

/// Multi-VOD clip — references a recording by index into the source
/// list, plus its own in/out word indices and label. (E3.)
#[derive(Debug, Clone)]
pub struct CompilationClip {
    /// Index into the `sources` Vec passed to
    /// [`build_compilation_argv`].
    pub source_index: usize,
    /// In/out word indices into that source's word stream.
    pub in_word: u32,
    pub out_word: u32,
    pub label: String,
}

/// One source recording in a compilation EDL. Carries the path + the
/// word stream so the renderer can resolve word indices → seconds
/// without round-tripping back to Crunchr's DB. (E3.)
pub struct CompilationSource {
    pub path: PathBuf,
    pub words: Vec<(String, f64)>,
}

/// Build the ffmpeg concat command for a multi-VOD compilation. Each
/// clip can point at a different source; the demuxer file lists each
/// source's `file 'path'` line before every clip's
/// `inpoint/outpoint` pair. Output lands next to the first source.
pub fn build_compilation_argv(
    clips: &[CompilationClip],
    sources: &[CompilationSource],
    output_label: &str,
) -> Result<ConcatPlan> {
    if clips.is_empty() {
        return Err(anyhow!("no clips to concat"));
    }
    if sources.is_empty() {
        return Err(anyhow!("no sources supplied"));
    }
    let mut list = String::new();
    for c in clips {
        let source = sources.get(c.source_index).ok_or_else(|| {
            anyhow!(
                "clip {}: source_index {} out of range (have {} sources)",
                c.label,
                c.source_index,
                sources.len()
            )
        })?;
        let in_secs = source
            .words
            .get(c.in_word as usize)
            .map(|(_, s)| *s)
            .ok_or_else(|| anyhow!("clip {}: in_word out of range", c.label))?;
        let out_secs = source
            .words
            .get(c.out_word as usize)
            .map(|(_, s)| *s)
            .ok_or_else(|| anyhow!("clip {}: out_word out of range", c.label))?;
        if out_secs <= in_secs {
            return Err(anyhow!(
                "clip {}: out_secs ({:.2}) ≤ in_secs ({:.2})",
                c.label,
                out_secs,
                in_secs
            ));
        }
        list.push_str(&format!("file '{}'\n", source.path.display()));
        list.push_str(&format!("inpoint {in_secs:.3}\n"));
        list.push_str(&format!("outpoint {out_secs:.3}\n"));
    }

    let first = &sources[0].path;
    let output_dir = first
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let safe_label: String = output_label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, '-' | '_'))
        .collect();
    let label = if safe_label.is_empty() {
        "compilation".to_string()
    } else {
        safe_label
    };
    let concat_file_path = output_dir.join(format!("{label}.compilation.txt"));
    let output_path = output_dir.join(format!("{label}.compilation.mkv"));

    let argv = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-i".into(),
        concat_file_path.to_string_lossy().into_owned(),
        "-c".into(),
        "copy".into(),
        "-map".into(),
        "0".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
        output_path.to_string_lossy().into_owned(),
    ];

    Ok(ConcatPlan {
        argv,
        concat_file_path,
        concat_file_contents: list,
        output_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(words: &[(&str, f64)]) -> Vec<(String, f64)> {
        words.iter().map(|(w, s)| ((*w).to_string(), *s)).collect()
    }

    #[test]
    fn empty_clip_list_errors() {
        let res = build_concat_argv(
            &[],
            &ws(&[("a", 0.0)]),
            &Some(PathBuf::from("/tmp/x.mkv")),
        );
        assert!(res.is_err());
    }

    #[test]
    fn missing_source_errors() {
        let clips = vec![EditorClip {
            in_word: 0,
            out_word: 1,
            label: "x".into(),
        }];
        let res = build_concat_argv(&clips, &ws(&[("a", 0.0), ("b", 1.0)]), &None);
        assert!(res.is_err());
    }

    #[test]
    fn out_of_range_errors() {
        let clips = vec![EditorClip {
            in_word: 0,
            out_word: 99,
            label: "x".into(),
        }];
        let res = build_concat_argv(
            &clips,
            &ws(&[("a", 0.0), ("b", 1.0)]),
            &Some(PathBuf::from("/tmp/x.mkv")),
        );
        assert!(res.is_err());
    }

    #[test]
    fn compilation_fans_across_sources() {
        let sources = vec![
            CompilationSource {
                path: PathBuf::from("/tmp/show-1.mkv"),
                words: vec![("hello".into(), 0.0), ("world".into(), 1.5)],
            },
            CompilationSource {
                path: PathBuf::from("/tmp/show-2.mkv"),
                words: vec![("good".into(), 0.0), ("night".into(), 2.0)],
            },
        ];
        let clips = vec![
            CompilationClip {
                source_index: 0,
                in_word: 0,
                out_word: 1,
                label: "a".into(),
            },
            CompilationClip {
                source_index: 1,
                in_word: 0,
                out_word: 1,
                label: "b".into(),
            },
        ];
        let plan = build_compilation_argv(&clips, &sources, "highlight-reel").unwrap();
        assert!(plan.concat_file_contents.contains("show-1.mkv"));
        assert!(plan.concat_file_contents.contains("show-2.mkv"));
        // 'inpoint 0.000' appears once per clip.
        assert_eq!(
            plan.concat_file_contents.matches("inpoint 0.000").count(),
            2
        );
        assert!(plan
            .output_path
            .to_string_lossy()
            .ends_with("highlight-reel.compilation.mkv"));
    }

    #[test]
    fn compilation_rejects_bad_source_index() {
        let sources = vec![CompilationSource {
            path: PathBuf::from("/tmp/x.mkv"),
            words: vec![("a".into(), 0.0), ("b".into(), 1.0)],
        }];
        let clips = vec![CompilationClip {
            source_index: 99,
            in_word: 0,
            out_word: 1,
            label: "x".into(),
        }];
        assert!(build_compilation_argv(&clips, &sources, "r").is_err());
    }

    #[test]
    fn compilation_label_sanitization() {
        let sources = vec![CompilationSource {
            path: PathBuf::from("/tmp/x.mkv"),
            words: vec![("a".into(), 0.0), ("b".into(), 1.0)],
        }];
        let clips = vec![CompilationClip {
            source_index: 0,
            in_word: 0,
            out_word: 1,
            label: "x".into(),
        }];
        let plan = build_compilation_argv(&clips, &sources, "Highlight Reel!!!").unwrap();
        // Non-alnum chars stripped.
        assert!(plan
            .output_path
            .to_string_lossy()
            .contains("HighlightReel"));
    }

    #[test]
    fn concat_argv_shape() {
        let clips = vec![
            EditorClip {
                in_word: 0,
                out_word: 1,
                label: "a".into(),
            },
            EditorClip {
                in_word: 1,
                out_word: 2,
                label: "b".into(),
            },
        ];
        let plan = build_concat_argv(
            &clips,
            &ws(&[("hi", 0.0), ("there", 1.5), ("end", 3.2)]),
            &Some(PathBuf::from("/tmp/show.mkv")),
        )
        .unwrap();
        assert_eq!(plan.argv[0], "ffmpeg");
        assert!(plan.argv.iter().any(|a| a == "concat"));
        assert!(plan.argv.iter().any(|a| a == "-c"));
        assert!(plan.argv.iter().any(|a| a == "copy"));
        assert!(plan.concat_file_contents.contains("inpoint 0.000"));
        assert!(plan.concat_file_contents.contains("outpoint 1.500"));
        assert!(plan.concat_file_contents.contains("inpoint 1.500"));
        assert!(plan.concat_file_contents.contains("outpoint 3.200"));
    }
}
