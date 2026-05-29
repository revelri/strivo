use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS channels (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    url         TEXT UNIQUE NOT NULL,
    platform    TEXT NOT NULL,
    archive_dir TEXT NOT NULL,
    last_scan   TIMESTAMP
);

CREATE TABLE IF NOT EXISTS videos (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id  INTEGER REFERENCES channels(id) ON DELETE CASCADE,
    video_id    TEXT NOT NULL,
    title       TEXT NOT NULL,
    upload_date TEXT,
    duration    REAL,
    playlist    TEXT,
    downloaded  BOOLEAN DEFAULT FALSE,
    UNIQUE(channel_id, video_id)
);

CREATE INDEX IF NOT EXISTS idx_videos_channel ON videos(channel_id);
CREATE INDEX IF NOT EXISTS idx_videos_downloaded ON videos(channel_id, downloaded);
"#;

pub fn open_and_init(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn upsert_channel(
    conn: &Connection,
    name: &str,
    url: &str,
    platform: &str,
    archive_dir: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO channels (name, url, platform, archive_dir) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(url) DO UPDATE SET name = ?1, archive_dir = ?4",
        rusqlite::params![name, url, platform, archive_dir],
    )?;
    let id = conn.query_row("SELECT id FROM channels WHERE url = ?1", [url], |r| {
        r.get(0)
    })?;
    Ok(id)
}

pub fn insert_videos(
    conn: &Connection,
    channel_id: i64,
    videos: &[(String, String, String, Option<f64>, Option<String>)],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO videos (channel_id, video_id, title, upload_date, duration, playlist) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (vid, title, date, dur, playlist) in videos {
        stmt.execute(rusqlite::params![
            channel_id, vid, title, date, dur, playlist
        ])?;
    }
    Ok(())
}

pub fn mark_downloaded(conn: &Connection, channel_id: i64, video_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE videos SET downloaded = TRUE WHERE channel_id = ?1 AND video_id = ?2",
        rusqlite::params![channel_id, video_id],
    )?;
    Ok(())
}

pub fn get_pending_videos(
    conn: &Connection,
    channel_id: i64,
) -> Result<Vec<(String, String, String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT video_id, title, upload_date, playlist FROM videos WHERE channel_id = ?1 AND downloaded = FALSE ORDER BY upload_date DESC",
    )?;
    let results = stmt
        .query_map([channel_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(results)
}

/// One archived channel with rollup counts. Used by the webui's Archiver
/// page to list catalogs without an N+1 query per channel.
#[derive(Debug, Clone)]
pub struct ChannelRow {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub platform: String,
    pub archive_dir: String,
    pub last_scan: Option<String>,
    pub video_count: i64,
    pub downloaded_count: i64,
}

/// Every tracked channel, newest-scanned first, with catalog rollups.
pub fn list_channels(conn: &Connection) -> Result<Vec<ChannelRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.url, c.platform, c.archive_dir, c.last_scan, \
                (SELECT COUNT(*) FROM videos v WHERE v.channel_id = c.id) AS total, \
                (SELECT COUNT(*) FROM videos v WHERE v.channel_id = c.id AND v.downloaded) AS got \
         FROM channels c ORDER BY c.last_scan DESC NULLS LAST, c.name",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ChannelRow {
                id: row.get(0)?,
                name: row.get(1)?,
                url: row.get(2)?,
                platform: row.get(3)?,
                archive_dir: row.get(4)?,
                last_scan: row.get(5)?,
                video_count: row.get(6)?,
                downloaded_count: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One catalog entry (downloaded or pending).
#[derive(Debug, Clone)]
pub struct VideoRow {
    pub video_id: String,
    pub title: String,
    pub upload_date: Option<String>,
    pub duration: Option<f64>,
    pub playlist: Option<String>,
    pub downloaded: bool,
}

/// Every catalog entry for a channel, newest upload first.
pub fn list_videos(conn: &Connection, channel_id: i64) -> Result<Vec<VideoRow>> {
    let mut stmt = conn.prepare(
        "SELECT video_id, title, upload_date, duration, playlist, downloaded \
         FROM videos WHERE channel_id = ?1 ORDER BY upload_date DESC",
    )?;
    let rows = stmt
        .query_map([channel_id], |row| {
            Ok(VideoRow {
                video_id: row.get(0)?,
                title: row.get(1)?,
                upload_date: row.get(2)?,
                duration: row.get(3)?,
                playlist: row.get(4)?,
                downloaded: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn list_channels_and_videos_roll_up_download_state() {
        let conn = fresh();
        let cid = upsert_channel(&conn, "Alpha", "https://t/alpha", "Twitch", "/arc/alpha").unwrap();
        let vids = vec![
            ("v1".to_string(), "One".to_string(), "20260101".to_string(), Some(60.0), None),
            ("v2".to_string(), "Two".to_string(), "20260102".to_string(), None, Some("pl".to_string())),
        ];
        insert_videos(&conn, cid, &vids).unwrap();
        mark_downloaded(&conn, cid, "v1").unwrap();

        let chans = list_channels(&conn).unwrap();
        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].video_count, 2);
        assert_eq!(chans[0].downloaded_count, 1);
        assert_eq!(chans[0].platform, "Twitch");

        let listed = list_videos(&conn, cid).unwrap();
        assert_eq!(listed.len(), 2);
        // Newest upload first.
        assert_eq!(listed[0].video_id, "v2");
        assert!(!listed[0].downloaded);
        let v1 = listed.iter().find(|v| v.video_id == "v1").unwrap();
        assert!(v1.downloaded);
    }
}
