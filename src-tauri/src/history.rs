// Translation history — SQLite persistence

use rusqlite::Connection;
use serde::Serialize;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub timestamp: String,
    pub source_text: String,
    pub source_lang: String,
    pub target_text: String,
    pub target_lang: String,
    pub method: String,
}

pub fn init(data_dir: &Path) -> Result<(), String> {
    let db_path = data_dir.join("history.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }

    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open history DB: {e}"))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL DEFAULT (datetime('now')),
            source_text TEXT NOT NULL,
            source_lang TEXT NOT NULL,
            target_text TEXT NOT NULL,
            target_lang TEXT NOT NULL,
            method TEXT NOT NULL DEFAULT 'model'
        );
        CREATE INDEX IF NOT EXISTS idx_history_ts ON history(timestamp DESC);"
    ).map_err(|e| format!("Failed to create history table: {e}"))?;

    DB.set(Mutex::new(conn))
        .map_err(|_| "History DB already initialized".to_string())
}

pub fn insert(
    src_text: &str,
    src_lang: &str,
    tgt_text: &str,
    tgt_lang: &str,
    method: &str,
) -> Result<(), String> {
    let db = DB.get().ok_or("History DB not initialized")?;
    let conn = db.lock().map_err(|e| format!("History DB lock: {e}"))?;

    conn.execute(
        "INSERT INTO history (source_text, source_lang, target_text, target_lang, method) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![src_text, src_lang, tgt_text, tgt_lang, method],
    ).map_err(|e| format!("History insert failed: {e}"))?;

    Ok(())
}

pub fn query_recent(
    limit: u32,
    offset: u32,
    search: Option<&str>,
) -> Result<Vec<HistoryEntry>, String> {
    let db = DB.get().ok_or("History DB not initialized")?;
    let conn = db.lock().map_err(|e| format!("History DB lock: {e}"))?;

    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(q) = search {
        let pattern = format!("%{q}%");
        (
            "SELECT id, timestamp, source_text, source_lang, target_text, target_lang, method FROM history WHERE source_text LIKE ?1 OR target_text LIKE ?1 ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3".into(),
            vec![Box::new(pattern), Box::new(limit), Box::new(offset)],
        )
    } else {
        (
            "SELECT id, timestamp, source_text, source_lang, target_text, target_lang, method FROM history ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2".into(),
            vec![Box::new(limit), Box::new(offset)],
        )
    };

    let mut stmt = conn.prepare(&sql).map_err(|e| format!("History query prepare: {e}"))?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(HistoryEntry {
            id: row.get(0)?,
            timestamp: row.get(1)?,
            source_text: row.get(2)?,
            source_lang: row.get(3)?,
            target_text: row.get(4)?,
            target_lang: row.get(5)?,
            method: row.get(6)?,
        })
    }).map_err(|e| format!("History query failed: {e}"))?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| format!("History row error: {e}"))?);
    }
    Ok(entries)
}

pub fn count(search: Option<&str>) -> Result<u32, String> {
    let db = DB.get().ok_or("History DB not initialized")?;
    let conn = db.lock().map_err(|e| format!("History DB lock: {e}"))?;

    let count: u32 = if let Some(q) = search {
        let pattern = format!("%{q}%");
        conn.query_row(
            "SELECT COUNT(*) FROM history WHERE source_text LIKE ?1 OR target_text LIKE ?1",
            rusqlite::params![pattern],
            |row| row.get(0),
        ).map_err(|e| format!("History count failed: {e}"))?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM history",
            [],
            |row| row.get(0),
        ).map_err(|e| format!("History count failed: {e}"))?
    };

    Ok(count)
}
