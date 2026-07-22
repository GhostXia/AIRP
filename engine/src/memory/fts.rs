//! FTS5 历史检索：SQLite 全文搜索（4.3）。
//!
//! 存储路径：`data/characters/{id}/search.db`（每角色一份 SQLite）
//! 索引时机：搜索前从持久化 ChatLog 幂等同步（含历史回填与删除清理）

use crate::error::AirpError;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const DEFAULT_CONNECTION_CACHE_CAPACITY: usize = 16;

struct CachedConnection {
    connection: Arc<Mutex<Connection>>,
    last_used: u64,
}

#[derive(Default)]
struct ConnectionCache {
    entries: HashMap<PathBuf, CachedConnection>,
    clock: u64,
}

/// Daemon-scoped, bounded cache of per-character SQLite connections.
pub struct FtsStore {
    cache: Mutex<ConnectionCache>,
    capacity: usize,
}

impl Default for FtsStore {
    fn default() -> Self {
        Self {
            cache: Mutex::new(ConnectionCache::default()),
            capacity: DEFAULT_CONNECTION_CACHE_CAPACITY,
        }
    }
}

impl FtsStore {
    #[cfg(test)]
    fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            cache: Mutex::new(ConnectionCache::default()),
            capacity,
        }
    }

    fn connection(
        &self,
        data_root: &Path,
        character_id: &str,
    ) -> Result<Arc<Mutex<Connection>>, AirpError> {
        let path = search_db_path(data_root, character_id);
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| AirpError::Internal(format!("FTS connection cache poisoned: {e}")))?;
        cache.clock = cache.clock.wrapping_add(1);
        let now = cache.clock;

        if let Some(entry) = cache.entries.get_mut(&path) {
            entry.last_used = now;
            return Ok(entry.connection.clone());
        }

        if cache.entries.len() >= self.capacity {
            let lru_idle = cache
                .entries
                .iter()
                .filter(|(_, entry)| Arc::strong_count(&entry.connection) == 1)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone());
            if let Some(path) = lru_idle {
                cache.entries.remove(&path);
            } else {
                // All cached connections are in use. Serve this operation with
                // an uncached connection rather than exceeding the hard cap.
                return Ok(Arc::new(Mutex::new(open_db(&path)?)));
            }
        }

        let connection = Arc::new(Mutex::new(open_db(&path)?));
        cache.entries.insert(
            path,
            CachedConnection {
                connection: connection.clone(),
                last_used: now,
            },
        );
        Ok(connection)
    }

    /// Insert one message into the character's FTS index.
    #[allow(clippy::too_many_arguments)]
    pub fn index_message(
        &self,
        data_root: &Path,
        character_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        timestamp: &str,
        message_id: Option<&str>,
    ) -> Result<(), AirpError> {
        let connection = self.connection(data_root, character_id)?;
        let conn = connection
            .lock()
            .map_err(|e| AirpError::Internal(format!("FTS connection poisoned: {e}")))?;
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp, message_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, role, content, timestamp, message_id],
        )?;
        Ok(())
    }

    /// Search a character's indexed conversation history.
    pub fn search(
        &self,
        data_root: &Path,
        character_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AirpError> {
        let connection = self.connection(data_root, character_id)?;
        let conn = connection
            .lock()
            .map_err(|e| AirpError::Internal(format!("FTS connection poisoned: {e}")))?;
        search_connection(&conn, query, limit)
    }

    /// Reconcile the derived SQLite index with every persisted chat session,
    /// then search it. This provides automatic historical backfill and keeps
    /// edits, deletes, rollbacks, swipes, and continuations consistent without
    /// making chat persistence depend on SQLite availability.
    pub fn search_history(
        &self,
        data_root: &Path,
        character_id: &crate::types::CharacterId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, AirpError> {
        self.synchronize_history(data_root, character_id)?;
        self.search(data_root, character_id.as_str(), query, limit)
    }

    fn synchronize_history(
        &self,
        data_root: &Path,
        character_id: &crate::types::CharacterId,
    ) -> Result<(), AirpError> {
        let service = crate::domain::ChatService::new(data_root);
        let connection = self.connection(data_root, character_id.as_str())?;
        let indexed: HashMap<String, String> = {
            let conn = connection
                .lock()
                .map_err(|e| AirpError::Internal(format!("FTS connection poisoned: {e}")))?;
            let mut stmt =
                conn.prepare("SELECT session_id, source_revision FROM indexed_sessions")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            rows
        };

        let mut scopes = vec![None];
        scopes.extend(service.list_sessions(character_id)?.into_iter().map(Some));
        let mut seen = HashSet::new();
        let mut changed = Vec::new();

        for session_id in scopes {
            let key = session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "default".to_string());
            seen.insert(key.clone());
            let path = chat_log_path(data_root, character_id.as_str(), session_id.as_ref());
            let revision = file_revision(&path)?;
            if revision
                .as_ref()
                .is_some_and(|value| indexed.get(&key) == Some(value))
            {
                continue;
            }

            // First search also performs legacy migration/backfill through the
            // canonical ChatService loading contract.
            let log = service.history(character_id, session_id.as_ref())?;
            let revision = file_revision(&path)?.ok_or_else(|| {
                AirpError::Internal(format!(
                    "chat history missing after load: {}",
                    path.display()
                ))
            })?;
            changed.push((key, revision, log));
        }

        let mut conn = connection
            .lock()
            .map_err(|e| AirpError::Internal(format!("FTS connection poisoned: {e}")))?;
        let transaction = conn.transaction()?;

        for (session_id, source_revision, log) in &changed {
            transaction.execute(
                "DELETE FROM messages WHERE session_id = ?1",
                params![session_id],
            )?;
            {
                let mut insert = transaction.prepare(
                    "INSERT INTO messages (session_id, role, content, timestamp, message_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for (index, message) in log.messages.iter().enumerate() {
                    let timestamp = log
                        .message_timestamps
                        .get(index)
                        .and_then(Option::as_deref)
                        .unwrap_or(&log.created_at);
                    insert.execute(params![
                        session_id,
                        message_role(message.role),
                        message.content,
                        timestamp,
                        log.message_ids.get(index).map(String::as_str),
                    ])?;
                }
            }
            transaction.execute(
                "INSERT INTO indexed_sessions (session_id, source_revision) VALUES (?1, ?2) \
                 ON CONFLICT(session_id) DO UPDATE SET source_revision = excluded.source_revision",
                params![session_id, source_revision],
            )?;
        }

        // Remove rows for deleted sessions and any rows created by the old
        // insert-only path before synchronization metadata existed.
        let indexed_session_ids: Vec<String> = {
            let mut stmt = transaction.prepare("SELECT DISTINCT session_id FROM messages")?;
            let rows = stmt
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            rows
        };
        for session_id in indexed_session_ids {
            if !seen.contains(&session_id) {
                transaction.execute(
                    "DELETE FROM messages WHERE session_id = ?1",
                    params![session_id],
                )?;
            }
        }
        for session_id in indexed.keys() {
            if !seen.contains(session_id) {
                transaction.execute(
                    "DELETE FROM indexed_sessions WHERE session_id = ?1",
                    params![session_id],
                )?;
            }
        }

        transaction.commit()?;
        Ok(())
    }

    #[cfg(test)]
    fn cached_paths(&self) -> Vec<PathBuf> {
        self.cache.lock().unwrap().entries.keys().cloned().collect()
    }
}

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
fn open_db(path: &Path) -> Result<Connection, AirpError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// 初始化 FTS5 schema。
///
/// 审计修复：增加 UPDATE/DELETE 触发器，保证消息编辑/删除后 FTS 索引同步。
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
        CREATE TABLE IF NOT EXISTS indexed_sessions (
            session_id TEXT PRIMARY KEY,
            source_revision TEXT NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, content, role) VALUES (new.id, new.content, new.role);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, content, role) VALUES ('delete', old.id, old.content, old.role);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, content, role) VALUES ('delete', old.id, old.content, old.role);
            INSERT INTO messages_fts(rowid, content, role) VALUES (new.id, new.content, new.role);
        END;",
    )?;
    Ok(())
}

fn message_role(role: crate::adapter::MessageRole) -> &'static str {
    match role {
        crate::adapter::MessageRole::User => "user",
        crate::adapter::MessageRole::Assistant => "assistant",
        crate::adapter::MessageRole::System => "system",
    }
}

fn chat_log_path(
    data_root: &Path,
    character_id: &str,
    session_id: Option<&crate::types::SessionId>,
) -> PathBuf {
    let character_dir = data_root.join("characters").join(character_id);
    match session_id {
        Some(session_id) => character_dir
            .join("sessions")
            .join(session_id.to_string())
            .join("history")
            .join("chat_log.jsonl"),
        None => character_dir.join("history").join("chat_log.jsonl"),
    }
}

fn file_revision(path: &Path) -> Result<Option<String>, AirpError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let modified = metadata
        .modified()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_err(|error| AirpError::Internal(format!("invalid chat history mtime: {error}")))?;
    Ok(Some(format!("{}:{}", metadata.len(), modified.as_nanos())))
}

/// 插入消息到搜索索引。
/// 全文搜索。
///
/// 审计修复：
/// - limit 钳制到 i64::MAX，防止溢出为负数（SQLite 负 LIMIT = 无限制）
/// - 非法 FTS5 查询语法返回 BadRequest 而非 500
fn search_connection(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, AirpError> {
    let mut stmt = conn.prepare(
        "SELECT m.session_id, m.role, m.content, m.timestamp, rank
         FROM messages_fts f
         JOIN messages m ON f.rowid = m.id
         WHERE messages_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    // 钳制 limit，防止 usize -> i64 溢出为负数。
    let limit_i64 = limit.min(i64::MAX as usize) as i64;

    let results = stmt
        .query_map(params![query, limit_i64], |row| {
            Ok(SearchResult {
                session_id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                timestamp: row.get(3)?,
                rank: row.get(4)?,
            })
        })
        .map_err(|e| map_fts_error(e, query))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| map_fts_error(e, query))?;

    Ok(results)
}

/// 把 FTS5 语法错误映射为 BadRequest，其他 SQLite 错误保持 Sqlite 变体。
fn map_fts_error(e: rusqlite::Error, query: &str) -> AirpError {
    let msg = e.to_string();
    if msg.contains("fts5") || msg.contains("syntax error") || msg.contains("malformed") {
        AirpError::BadRequest(format!("invalid search query: {}", query))
    } else {
        AirpError::Sqlite(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{ChatMessage, MessageRole};
    use crate::domain::ChatService;
    use crate::types::CharacterId;
    use tempfile::tempdir;

    #[test]
    fn test_index_and_search() {
        let tmp = tempdir().unwrap();
        let store = FtsStore::default();

        store
            .index_message(
                tmp.path(),
                "hero",
                "session-1",
                "user",
                "hello world from user",
                "2026-01-01T00:00:00Z",
                None,
            )
            .unwrap();

        store
            .index_message(
                tmp.path(),
                "hero",
                "session-1",
                "assistant",
                "hello I am the character Aria",
                "2026-01-01T00:00:01Z",
                None,
            )
            .unwrap();

        let results = store.search(tmp.path(), "hero", "user", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role, "user");

        let results = store.search(tmp.path(), "hero", "Aria", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role, "assistant");
    }

    #[test]
    fn test_search_empty_db() {
        let tmp = tempdir().unwrap();
        let store = FtsStore::default();
        let results = store.search(tmp.path(), "hero", "anything", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let tmp = tempdir().unwrap();
        let store = FtsStore::default();
        store
            .index_message(
                tmp.path(),
                "hero",
                "s1",
                "user",
                "hello world",
                "2026-01-01T00:00:00Z",
                None,
            )
            .unwrap();
        let results = store.search(tmp.path(), "hero", "nonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn reuses_cached_connection_for_same_character() {
        let tmp = tempdir().unwrap();
        let store = FtsStore::default();
        store.search(tmp.path(), "hero", "anything", 10).unwrap();
        store.search(tmp.path(), "hero", "anything", 10).unwrap();

        assert_eq!(store.cached_paths().len(), 1);
    }

    #[test]
    fn evicts_least_recently_used_idle_connection() {
        let tmp = tempdir().unwrap();
        let store = FtsStore::with_capacity(2);
        store.search(tmp.path(), "one", "anything", 10).unwrap();
        store.search(tmp.path(), "two", "anything", 10).unwrap();
        store.search(tmp.path(), "one", "anything", 10).unwrap();
        store.search(tmp.path(), "three", "anything", 10).unwrap();

        let paths = store.cached_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|path| path.ends_with("one/search.db")));
        assert!(paths.iter().any(|path| path.ends_with("three/search.db")));
    }

    #[test]
    fn active_connections_do_not_expand_cache_past_capacity() {
        let tmp = tempdir().unwrap();
        let store = FtsStore::with_capacity(1);
        let active = store.connection(tmp.path(), "one").unwrap();
        let uncached = store.connection(tmp.path(), "two").unwrap();

        assert_eq!(store.cached_paths().len(), 1);
        assert!(store.cached_paths()[0].ends_with("one/search.db"));
        assert!(!Arc::ptr_eq(&active, &uncached));
    }

    #[test]
    fn search_history_backfills_and_tracks_message_mutations() {
        let tmp = tempdir().unwrap();
        let character = CharacterId::new("hero").unwrap();
        let service = ChatService::new(tmp.path());
        let store = FtsStore::default();

        let (log, _) = service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "historical sapphire phrase".into(),
                },
            )
            .unwrap();
        let message_id = log.message_ids[0].clone();

        let results = store
            .search_history(tmp.path(), &character, "sapphire", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "default");

        service
            .edit_message(&character, None, &message_id, "replacement emerald phrase")
            .unwrap();
        assert!(store
            .search_history(tmp.path(), &character, "sapphire", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .search_history(tmp.path(), &character, "emerald", 10)
                .unwrap()
                .len(),
            1
        );

        service
            .delete_message(&character, None, &message_id)
            .unwrap();
        assert!(store
            .search_history(tmp.path(), &character, "emerald", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_history_removes_deleted_named_session() {
        let tmp = tempdir().unwrap();
        let character = CharacterId::new("hero").unwrap();
        let service = ChatService::new(tmp.path());
        let store = FtsStore::default();
        let session_id = service.create_session(&character).unwrap();

        service
            .append(
                &character,
                Some(&session_id),
                ChatMessage {
                    role: MessageRole::Assistant,
                    content: "named-session topaz phrase".into(),
                },
            )
            .unwrap();
        let results = store
            .search_history(tmp.path(), &character, "topaz", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, session_id.to_string());

        service.delete_session(&character, &session_id).unwrap();
        assert!(store
            .search_history(tmp.path(), &character, "topaz", 10)
            .unwrap()
            .is_empty());
    }
}
