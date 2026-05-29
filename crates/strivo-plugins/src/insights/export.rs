//! Dataset export — CSV and JSON. (I4.)
//!
//! Writes to `<XDG_DOWNLOAD_DIR>/strivo/insights/<view>-<timestamp>.<ext>`
//! and returns the path. Atomic: writes to `.tmp` then renames.

use std::path::PathBuf;

use anyhow::Result;

use super::frequency::FrequencyRow;

fn export_dir() -> PathBuf {
    let base = std::env::var_os("STRIVO_INSIGHTS_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::download_dir().map(|d| d.join("strivo").join("insights")))
        .unwrap_or_else(|| std::env::temp_dir().join("strivo-insights"));
    let _ = std::fs::create_dir_all(&base);
    base
}

/// Test-only: export CSV into a specific directory (bypasses the
/// env-var lookup so parallel tests don't race on STRIVO_INSIGHTS_DIR).
#[cfg(test)]
fn export_csv_to(rows: &[FrequencyRow], dir: &std::path::Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("frequency-{}.csv", timestamp()));
    let mut body = String::from("word,count\n");
    for r in rows {
        let needs_quote = r.word.contains(',') || r.word.contains('"');
        if needs_quote {
            let escaped = r.word.replace('"', "\"\"");
            body.push('"');
            body.push_str(&escaped);
            body.push('"');
        } else {
            body.push_str(&r.word);
        }
        body.push(',');
        body.push_str(&r.count.to_string());
        body.push('\n');
    }
    atomic_write(&path, &body)?;
    Ok(path)
}

#[cfg(test)]
fn export_json_to(rows: &[FrequencyRow], dir: &std::path::Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("frequency-{}.json", timestamp()));
    #[derive(serde::Serialize)]
    struct Row<'a> {
        word: &'a str,
        count: i64,
    }
    let mapped: Vec<Row<'_>> = rows
        .iter()
        .map(|r| Row {
            word: &r.word,
            count: r.count,
        })
        .collect();
    let body = serde_json::to_string_pretty(&mapped)?;
    atomic_write(&path, &body)?;
    Ok(path)
}

fn timestamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

fn atomic_write(path: &std::path::Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Export the current frequency dataset as CSV. Two columns: word, count.
pub fn export_csv(rows: &[FrequencyRow]) -> Result<PathBuf> {
    let path = export_dir().join(format!("frequency-{}.csv", timestamp()));
    let mut body = String::from("word,count\n");
    for r in rows {
        // Quote words containing commas or quotes.
        let needs_quote = r.word.contains(',') || r.word.contains('"');
        if needs_quote {
            let escaped = r.word.replace('"', "\"\"");
            body.push('"');
            body.push_str(&escaped);
            body.push('"');
        } else {
            body.push_str(&r.word);
        }
        body.push(',');
        body.push_str(&r.count.to_string());
        body.push('\n');
    }
    atomic_write(&path, &body)?;
    Ok(path)
}

/// Export as JSON array of {word, count} objects.
pub fn export_json(rows: &[FrequencyRow]) -> Result<PathBuf> {
    let path = export_dir().join(format!("frequency-{}.json", timestamp()));
    #[derive(serde::Serialize)]
    struct Row<'a> {
        word: &'a str,
        count: i64,
    }
    let mapped: Vec<Row<'_>> = rows
        .iter()
        .map(|r| Row {
            word: &r.word,
            count: r.count,
        })
        .collect();
    let body = serde_json::to_string_pretty(&mapped)?;
    atomic_write(&path, &body)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_words_with_commas() {
        let tmp = tempfile::tempdir().unwrap();
        let rows = vec![FrequencyRow {
            word: "hi, there".into(),
            count: 5,
        }];
        let path = export_csv_to(&rows, tmp.path()).unwrap();
        let body = std::fs::read_to_string(path).unwrap();
        assert!(body.contains("\"hi, there\",5"));
    }

    #[test]
    fn json_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let rows = vec![
            FrequencyRow {
                word: "stream".into(),
                count: 142,
            },
            FrequencyRow {
                word: "chat".into(),
                count: 98,
            },
        ];
        let path = export_json_to(&rows, tmp.path()).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["word"], "stream");
        assert_eq!(parsed[0]["count"], 142);
    }
}
