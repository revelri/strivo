use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use super::types::SearchResult;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS videos (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    recording_id    TEXT UNIQUE NOT NULL,
    channel_name    TEXT NOT NULL,
    title           TEXT NOT NULL,
    video_path      TEXT,
    audio_path      TEXT,
    transcript_text TEXT,
    status          TEXT DEFAULT 'pending'
                    CHECK(status IN ('pending','extracting_audio','transcribing','chunking','analyzing','complete','failed')),
    error_message   TEXT,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS segments (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    video_id        INTEGER REFERENCES videos(id) ON DELETE CASCADE,
    segment_index   INTEGER NOT NULL,
    start_sec       REAL NOT NULL,
    end_sec         REAL NOT NULL,
    text            TEXT NOT NULL,
    speaker         TEXT,
    confidence      REAL,
    UNIQUE(video_id, segment_index)
);

CREATE TABLE IF NOT EXISTS chunks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    video_id        INTEGER REFERENCES videos(id) ON DELETE CASCADE,
    chunk_index     INTEGER NOT NULL,
    text            TEXT NOT NULL,
    start_sec       REAL,
    end_sec         REAL,
    token_count     INTEGER,
    embedding       BLOB,
    UNIQUE(video_id, chunk_index)
);

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    text,
    content=chunks,
    content_rowid=id,
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS chunks_fts_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TABLE IF NOT EXISTS word_frequency (
    video_id        INTEGER REFERENCES videos(id) ON DELETE CASCADE,
    word            TEXT NOT NULL,
    count           INTEGER NOT NULL,
    PRIMARY KEY(video_id, word)
);

CREATE TABLE IF NOT EXISTS tfidf_vocabulary (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    term            TEXT UNIQUE NOT NULL,
    doc_frequency   INTEGER DEFAULT 0,
    idf             REAL
);

CREATE TABLE IF NOT EXISTS video_analysis (
    video_id        INTEGER PRIMARY KEY REFERENCES videos(id) ON DELETE CASCADE,
    summary         TEXT,
    topics          TEXT,
    sentiment       TEXT,
    analyzed_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_segments_video ON segments(video_id);
CREATE INDEX IF NOT EXISTS idx_chunks_video ON chunks(video_id);
CREATE INDEX IF NOT EXISTS idx_wordfreq_word ON word_frequency(word);
"#;

/// Open database connection and run schema migration.
pub fn open_and_init(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn insert_video(
    conn: &Connection,
    recording_id: &str,
    channel_name: &str,
    title: &str,
    video_path: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO videos (recording_id, channel_name, title, video_path) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![recording_id, channel_name, title, video_path],
    )?;
    let id = conn.query_row(
        "SELECT id FROM videos WHERE recording_id = ?1",
        [recording_id],
        |row| row.get(0),
    )?;
    Ok(id)
}

pub fn update_video_status(conn: &Connection, recording_id: &str, status: &str, error: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE videos SET status = ?1, error_message = ?2 WHERE recording_id = ?3",
        rusqlite::params![status, error, recording_id],
    )?;
    Ok(())
}

pub fn update_video_audio_path(conn: &Connection, recording_id: &str, audio_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE videos SET audio_path = ?1 WHERE recording_id = ?2",
        rusqlite::params![audio_path, recording_id],
    )?;
    Ok(())
}

pub fn update_video_transcript(conn: &Connection, recording_id: &str, transcript: &str) -> Result<()> {
    conn.execute(
        "UPDATE videos SET transcript_text = ?1 WHERE recording_id = ?2",
        rusqlite::params![transcript, recording_id],
    )?;
    Ok(())
}

pub fn insert_segments(
    conn: &Connection,
    video_id: i64,
    segments: &[(usize, f64, f64, &str, Option<&str>, Option<f64>)],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO segments (video_id, segment_index, start_sec, end_sec, text, speaker, confidence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (idx, start, end, text, speaker, confidence) in segments {
        stmt.execute(rusqlite::params![video_id, idx, start, end, text, speaker, confidence])?;
    }
    Ok(())
}

pub fn insert_chunks(
    conn: &Connection,
    video_id: i64,
    chunks: &[(usize, &str, f64, f64, usize)],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO chunks (video_id, chunk_index, text, start_sec, end_sec, token_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (idx, text, start, end, tokens) in chunks {
        stmt.execute(rusqlite::params![video_id, idx, text, start, end, tokens])?;
    }
    Ok(())
}

pub fn insert_word_frequencies(
    conn: &Connection,
    video_id: i64,
    frequencies: &[(String, usize)],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO word_frequency (video_id, word, count) VALUES (?1, ?2, ?3)",
    )?;
    for (word, count) in frequencies {
        stmt.execute(rusqlite::params![video_id, word, count])?;
    }
    Ok(())
}

/// Sanitize a user query for FTS5 MATCH: wrap in double quotes to treat as literal phrase.
fn sanitize_fts_query(query: &str) -> String {
    // Escape internal double quotes and wrap in quotes for literal matching
    let escaped = query.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

pub fn fts_search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let safe_query = sanitize_fts_query(query);

    let mut stmt = conn.prepare(
        "SELECT c.id, v.title, v.channel_name, snippet(chunks_fts, 0, '>>>', '<<<', '...', 40), c.start_sec, rank, v.video_path
         FROM chunks_fts
         JOIN chunks c ON c.id = chunks_fts.rowid
         JOIN videos v ON v.id = c.video_id
         WHERE chunks_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let results = stmt.query_map(rusqlite::params![safe_query, limit], |row| {
        Ok(SearchResult {
            chunk_id: row.get(0)?,
            video_title: row.get(1)?,
            channel_name: row.get(2)?,
            snippet: row.get(3)?,
            start_sec: row.get(4)?,
            score: row.get::<_, f64>(5)?.abs(),
            video_path: row.get(6)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

pub fn get_top_words(conn: &Connection, limit: usize) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT word, SUM(count) as total FROM word_frequency GROUP BY word ORDER BY total DESC LIMIT ?1",
    )?;
    let results = stmt
        .query_map([limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(results)
}

/// Get analysis data for a video that owns the given chunk.
pub fn get_analysis_for_chunk(conn: &Connection, chunk_id: i64) -> Result<Option<super::types::AnalysisData>> {
    let result = conn.query_row(
        "SELECT va.summary, va.topics, va.sentiment
         FROM video_analysis va
         JOIN chunks c ON c.video_id = va.video_id
         WHERE c.id = ?1",
        [chunk_id],
        |row| {
            let summary: String = row.get(0)?;
            let topics_json: String = row.get(1)?;
            let sentiment: String = row.get(2)?;
            Ok((summary, topics_json, sentiment))
        },
    );
    match result {
        Ok((summary, topics_json, sentiment)) => {
            let topics: Vec<String> = serde_json::from_str(&topics_json).unwrap_or_default();
            Ok(Some(super::types::AnalysisData { summary, topics, sentiment }))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get speaker label for a chunk's time range.
pub fn get_speaker_for_chunk(conn: &Connection, chunk_id: i64) -> Result<Option<String>> {
    let result = conn.query_row(
        "SELECT s.speaker
         FROM segments s
         JOIN chunks c ON c.video_id = s.video_id
         WHERE c.id = ?1 AND s.start_sec <= c.start_sec AND s.end_sec >= c.start_sec AND s.speaker IS NOT NULL
         ORDER BY s.start_sec
         LIMIT 1",
        [chunk_id],
        |row| row.get(0),
    );
    match result {
        Ok(speaker) => Ok(Some(speaker)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn get_video_id_by_recording(conn: &Connection, recording_id: &str) -> Result<Option<i64>> {
    let result = conn.query_row(
        "SELECT id FROM videos WHERE recording_id = ?1",
        [recording_id],
        |row| row.get(0),
    );
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn get_segments_for_video(conn: &Connection, video_id: i64) -> Result<Vec<(usize, f64, f64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT segment_index, start_sec, end_sec, text FROM segments WHERE video_id = ?1 ORDER BY segment_index",
    )?;
    let results = stmt
        .query_map([video_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(results)
}
