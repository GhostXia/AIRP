//! FTS5 历史检索：SQLite 全文搜索（4.3）。
//!
//! 存储路径：`data/characters/{id}/search.db`（每角色一份 SQLite）
//! 索引时机：finalize 后 best-effort 插入

use crate::error::AirpError;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// 搜索结果条目。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub rank: f64,
}

/// 返回角色的 search.db 路径。
fn search_db_path(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("search.db")
}

/// 打开或创建搜索数据库。
fn open_db(data_root: &Path, character_id: &str) -> Result<Connection, AirpError> {
    let path = search_db_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// 初始化 FTS5 schema。
fn init_schema(conn: &Connection) -> Result<(), AirpError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            message_id TEXT
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            content, role,
            content='messages',
            content_rowid='id'
        );
        CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, content, role) VALUES (new.id, new.content, new.role);
        END;",
    )?;
    Ok(())
}

/// 插入消息到搜索索引。
pub fn index_message(
    data_root: &Path,
    character_id: &str,
    session_id: &str,
    role: &str,
    content: &str,
    timestamp: &str,
    message_id: Option<&str>,
) -> Result<(), AirpError> {
    let conn = open_db(data_root, character_id)?;
    conn.execute(
        "INSERT INTO messages (session_id, role, content, timestamp, message_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, role, content, timestamp, message_id],
    )?;
    Ok(())
}

/// 全文搜索。
pub fn search(
    data_root: &Path,
    character_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, AirpError> {
    let conn = open_db(data_root, character_id)?;
    let mut stmt = conn.prepare(
        "SELECT m.session_id, m.role, m.content, m.timestamp, rank
         FROM messages_fts f
         JOIN messages m ON f.rowid = m.id
         WHERE messages_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let results = stmt
        .query_map(params![query, limit as i64], |row| {
            Ok(SearchResult {
                session_id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                timestamp: row.get(3)?,
                rank: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_index_and_search() {
        let tmp = tempdir().unwrap();

        index_message(
            tmp.path(),
            "hero",
            "session-1",
            "user",
            "hello world from user",
            "2026-01-01T00:00:00Z",
            None,
        )
        .unwrap();

        index_message(
            tmp.path(),
            "hero",
            "session-1",
            "assistant",
            "hello I am the character Aria",
            "2026-01-01T00:00:01Z",
            None,
        )
        .unwrap();

        let results = search(tmp.path(), "hero", "user", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role, "user");

        let results = search(tmp.path(), "hero", "Aria", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role, "assistant");
    }

    #[test]
    fn test_search_empty_db() {
        let tmp = tempdir().unwrap();
        let results = search(tmp.path(), "hero", "anything", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let tmp = tempdir().unwrap();
        index_message(
            tmp.path(),
            "hero",
            "s1",
            "user",
            "hello world",
            "2026-01-01T00:00:00Z",
            None,
        )
        .unwrap();
        let results = search(tmp.path(), "hero", "nonexistent", 10).unwrap();
        assert!(results.is_empty());
    }
}
