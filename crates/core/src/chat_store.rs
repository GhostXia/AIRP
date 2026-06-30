use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::adapter::ChatMessage;
use crate::error::AirpError;

/// ChatLog 滚动上限：超过此值时丢弃最早的消息。
///
/// 设计动机：ChatLog 仅用于短期上下文与 UI 回放；长期记忆由卷系统
/// （`current.md` + `vol_XXX.md` + `index.md`）承担。无上限增长会导致
/// 单次 load_or_create 反序列化变慢且占用大量内存。
pub const MAX_MESSAGES: usize = 1000;

/// A complete chat log for one character session.
///
/// **CF-2 持久化模型**：消息列表写入 `history/chat_log.jsonl`（每行一条 JSON 消息），
/// 元数据（session_id / 时间戳）写入 `history/chat_log_meta.json`。
/// `append` 走 `OpenOptions::append` 实现 O(1) 追加；只有在滚动截断
/// （`MAX_MESSAGES`）/ delete_last_n / rollback_to 等需要重写历史的路径才会
/// 触发整体重写。
///
/// 迁移链：`chat_log.json`（<6.0e）→ `chat_log.jsonl`（6.0e，根目录）
/// → `history/chat_log.jsonl`（CF-2，history/ 子目录）。
/// `load_or_create` 自动处理全部迁移步骤。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLog {
    /// Unique session identifier
    pub session_id: String,
    /// Character folder name
    pub character_id: String,
    /// Ordered list of messages (user + assistant interleaved)
    pub messages: Vec<ChatMessage>,
    /// ISO 8601 creation timestamp
    pub created_at: String,
    /// ISO 8601 last update timestamp
    pub updated_at: String,
}

/// 持久化在 `chat_log_meta.json` 中的小型元数据 (无 messages 字段)。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatLogMeta {
    session_id: String,
    character_id: String,
    created_at: String,
    updated_at: String,
}

impl ChatLog {
    /// Creates a new empty chat log for a character.
    pub fn new(character_id: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            character_id: character_id.to_string(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// 角色目录下消息 JSONL 文件路径（CF-2 位置：`history/` 子目录）。
    fn jsonl_path(data_root: &Path, character_id: &str) -> PathBuf {
        data_root
            .join("characters")
            .join(character_id)
            .join("history")
            .join("chat_log.jsonl")
    }

    /// 角色目录下元数据 JSON 文件路径（CF-2 位置：`history/` 子目录）。
    fn meta_path(data_root: &Path, character_id: &str) -> PathBuf {
        data_root
            .join("characters")
            .join(character_id)
            .join("history")
            .join("chat_log_meta.json")
    }

    /// Pre-CF2 JSONL 路径（6.0e 旧位置：字符根目录，迁移用）。
    fn pre_cf2_jsonl_path(data_root: &Path, character_id: &str) -> PathBuf {
        data_root
            .join("characters")
            .join(character_id)
            .join("chat_log.jsonl")
    }

    /// Pre-CF2 meta 路径（6.0e 旧位置：字符根目录，迁移用）。
    fn pre_cf2_meta_path(data_root: &Path, character_id: &str) -> PathBuf {
        data_root
            .join("characters")
            .join(character_id)
            .join("chat_log_meta.json")
    }

    /// Legacy 单文件 JSON 路径（6.0e 之前的格式，迁移用）。
    fn legacy_path(data_root: &Path, character_id: &str) -> PathBuf {
        data_root
            .join("characters")
            .join(character_id)
            .join("chat_log.json")
    }

    /// Loads an existing chat log from disk, or creates a new one.
    ///
    /// 加载顺序（完整迁移链）：
    ///   1. `history/chat_log.jsonl` 存在 → 直接加载（CF-2 当前格式）；
    ///   2. 根目录 `chat_log.jsonl` 存在（6.0e pre-CF2）→ 迁移到 `history/` 后加载；
    ///   3. 根目录 `chat_log.json` 存在（<6.0e legacy）→ 迁移到 `history/` 后加载；
    ///   4. 均不存在 → 新建空 log，写入 `history/`。
    pub fn load_or_create(data_root: &Path, character_id: &str) -> Result<Self, AirpError> {
        let jsonl = Self::jsonl_path(data_root, character_id);
        let meta_p = Self::meta_path(data_root, character_id);
        let pre_cf2_jsonl = Self::pre_cf2_jsonl_path(data_root, character_id);
        let pre_cf2_meta = Self::pre_cf2_meta_path(data_root, character_id);
        let legacy = Self::legacy_path(data_root, character_id);

        // ── 1. CF-2 新位置 ────────────────────────────────────────────────────
        if jsonl.exists() {
            let messages = Self::read_messages_jsonl(&jsonl)?;
            let m: ChatLogMeta = if meta_p.exists() {
                serde_json::from_str(&fs::read_to_string(&meta_p)?)?
            } else {
                // meta 丢失 → 重建最小元数据
                let now = Utc::now().to_rfc3339();
                ChatLogMeta {
                    session_id: uuid::Uuid::new_v4().to_string(),
                    character_id: character_id.to_string(),
                    created_at: now.clone(),
                    updated_at: now,
                }
            };
            return Ok(Self {
                session_id: m.session_id,
                character_id: m.character_id,
                messages,
                created_at: m.created_at,
                updated_at: m.updated_at,
            });
        }

        // ── 2. pre-CF2 迁移：根目录 chat_log.jsonl → history/ ─────────────────
        if pre_cf2_jsonl.exists() {
            tracing::info!(char = character_id, "CF-2 迁移: chat_log.jsonl → history/");
            let messages = Self::read_messages_jsonl(&pre_cf2_jsonl)?;
            let m: ChatLogMeta = if pre_cf2_meta.exists() {
                serde_json::from_str(&fs::read_to_string(&pre_cf2_meta)?)?
            } else {
                let now = Utc::now().to_rfc3339();
                ChatLogMeta {
                    session_id: uuid::Uuid::new_v4().to_string(),
                    character_id: character_id.to_string(),
                    created_at: now.clone(),
                    updated_at: now,
                }
            };
            let log = Self {
                session_id: m.session_id,
                character_id: m.character_id,
                messages,
                created_at: m.created_at,
                updated_at: m.updated_at,
            };
            log.save(data_root)?;
            // 删除旧文件，失败不阻塞
            if let Err(e) = fs::remove_file(&pre_cf2_jsonl) {
                tracing::warn!(path = ?pre_cf2_jsonl, err = %e, "CF-2 迁移: 删除旧 chat_log.jsonl 失败");
            }
            if pre_cf2_meta.exists() {
                if let Err(e) = fs::remove_file(&pre_cf2_meta) {
                    tracing::warn!(path = ?pre_cf2_meta, err = %e, "CF-2 迁移: 删除旧 chat_log_meta.json 失败");
                }
            }
            return Ok(log);
        }

        // ── 3. legacy JSON 迁移：chat_log.json → history/ ────────────────────
        if legacy.exists() {
            tracing::info!(
                char = character_id,
                "CF-2 迁移: chat_log.json (legacy) → history/"
            );
            let content = fs::read_to_string(&legacy)?;
            let log: ChatLog = serde_json::from_str(&content)?;
            log.save(data_root)?;
            if let Err(e) = fs::remove_file(&legacy) {
                tracing::warn!(path = ?legacy, err = %e, "迁移完成但删除旧 chat_log.json 失败");
            }
            return Ok(log);
        }

        // ── 4. 全新 ───────────────────────────────────────────────────────────
        crate::data_dir::character_dir(data_root, character_id)?;
        let log = ChatLog::new(character_id);
        log.save(data_root)?;
        Ok(log)
    }

    /// 整体重写 jsonl + meta（用于 delete/rollback/滚动截断）。
    pub fn save(&self, data_root: &Path) -> Result<(), AirpError> {
        let jsonl = Self::jsonl_path(data_root, &self.character_id);
        let meta = Self::meta_path(data_root, &self.character_id);

        // 确保 history/ 目录存在
        if let Some(parent) = jsonl.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        // 写 jsonl：一行一条 message
        let mut buf = String::new();
        for m in &self.messages {
            buf.push_str(&serde_json::to_string(m)?);
            buf.push('\n');
        }
        fs::write(&jsonl, buf)?;

        // 写 meta（小文件）
        let m = ChatLogMeta {
            session_id: self.session_id.clone(),
            character_id: self.character_id.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        };
        let meta_content = serde_json::to_string_pretty(&m)?;
        fs::write(&meta, meta_content)?;
        Ok(())
    }

    /// Appends a message.
    ///
    /// 常规路径 O(1)：以 `OpenOptions::append` 在 jsonl 末尾追加一行，
    /// 然后用 ~小常数大小的 meta 文件刷新 `updated_at`。
    /// 仅当超过 `MAX_MESSAGES` 触发 FIFO 滚动时才会发生整体重写。
    pub fn append(&mut self, data_root: &Path, msg: ChatMessage) -> Result<(), AirpError> {
        self.messages.push(msg.clone());

        if self.messages.len() > MAX_MESSAGES {
            // 滚动截断 → 走整体重写。
            let drop = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(..drop);
            self.updated_at = Utc::now().to_rfc3339();
            return self.save(data_root);
        }

        // 常规追加：jsonl O(1) 写入 + meta 小文件刷新。
        let jsonl = Self::jsonl_path(data_root, &self.character_id);
        // 文件可能首次创建（迁移路径已 ensure，但保底处理）
        if let Some(parent) = jsonl.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl)?;
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;

        // meta 刷新
        self.updated_at = Utc::now().to_rfc3339();
        let m = ChatLogMeta {
            session_id: self.session_id.clone(),
            character_id: self.character_id.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        };
        let meta_path = Self::meta_path(data_root, &self.character_id);
        fs::write(&meta_path, serde_json::to_string_pretty(&m)?)?;
        Ok(())
    }

    /// Deletes the last N messages (for regen: delete last assistant message).
    pub fn delete_last_n(&mut self, data_root: &Path, n: usize) -> Result<(), AirpError> {
        let len = self.messages.len();
        if n > len {
            self.messages.clear();
        } else {
            self.messages.truncate(len - n);
        }
        self.updated_at = Utc::now().to_rfc3339();
        self.save(data_root)
    }

    /// Rolls back to a specific message index (keeps messages 0..=index).
    pub fn rollback_to(&mut self, data_root: &Path, index: usize) -> Result<(), AirpError> {
        if index < self.messages.len() {
            self.messages.truncate(index + 1);
            self.updated_at = Utc::now().to_rfc3339();
            self.save(data_root)?;
        }
        Ok(())
    }

    /// Returns the N most recent messages for context building.
    pub fn recent(&self, n: usize) -> Vec<ChatMessage> {
        let len = self.messages.len();
        if n >= len {
            self.messages.clone()
        } else {
            self.messages[len - n..].to_vec()
        }
    }

    /// 逐行解析 jsonl。空行忽略；非法行返回错误（不静默吞掉，避免历史丢失）。
    fn read_messages_jsonl(path: &Path) -> Result<Vec<ChatMessage>, AirpError> {
        let content = fs::read_to_string(path)?;
        let mut out = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let m: ChatMessage = serde_json::from_str(line).map_err(|e| {
                AirpError::Internal(format!("chat_log.jsonl 第 {} 行解析失败: {}", i + 1, e))
            })?;
            out.push(m);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_chat_log_crud() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        // Create character dir structure
        fs::create_dir_all(root.join("characters").join("test_char")).unwrap();

        let mut log = ChatLog::new("test_char");
        assert!(log.messages.is_empty());

        // Append messages
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "Hello".to_string(),
            },
        )
        .unwrap();
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content: "Hi there".to_string(),
            },
        )
        .unwrap();
        assert_eq!(log.messages.len(), 2);

        // Reload from disk
        let reloaded = ChatLog::load_or_create(root, "test_char").unwrap();
        assert_eq!(reloaded.messages.len(), 2);
        assert_eq!(reloaded.messages[0].content, "Hello");

        // Delete last (regen)
        let mut log2 = reloaded;
        log2.delete_last_n(root, 1).unwrap();
        assert_eq!(log2.messages.len(), 1);

        // Rollback
        log2.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content: "New reply".to_string(),
            },
        )
        .unwrap();
        log2.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "Follow up".to_string(),
            },
        )
        .unwrap();
        log2.rollback_to(root, 0).unwrap();
        assert_eq!(log2.messages.len(), 1);
        assert_eq!(log2.messages[0].content, "Hello");
    }

    #[test]
    fn test_jsonl_persistence_layout() {
        // CF-2：验证新格式以 history/jsonl + history/meta 两文件落盘
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("characters").join("alice")).unwrap();

        let mut log = ChatLog::new("alice");
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "hi".to_string(),
            },
        )
        .unwrap();
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content: "hello".to_string(),
            },
        )
        .unwrap();

        let jsonl_path = root
            .join("characters")
            .join("alice")
            .join("history")
            .join("chat_log.jsonl");
        let meta_path = root
            .join("characters")
            .join("alice")
            .join("history")
            .join("chat_log_meta.json");
        assert!(jsonl_path.exists(), "history/chat_log.jsonl 应存在");
        assert!(meta_path.exists(), "history/chat_log_meta.json 应存在");

        let jsonl_content = fs::read_to_string(&jsonl_path).unwrap();
        // 两行 + 末尾换行
        let lines: Vec<&str> = jsonl_content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"role\":\"user\""));
        assert!(lines[1].contains("\"role\":\"assistant\""));
    }

    #[test]
    fn test_legacy_json_migration() {
        // CF-2：旧 chat_log.json 在 load_or_create 时应被迁移到 history/ 下
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("bob");
        fs::create_dir_all(&char_dir).unwrap();

        // 手工写入 legacy 文件
        let legacy = char_dir.join("chat_log.json");
        let legacy_json = r#"{
            "session_id": "legacy-session",
            "character_id": "bob",
            "messages": [
                {"role": "user", "content": "old1"},
                {"role": "assistant", "content": "old2"}
            ],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z"
        }"#;
        fs::write(&legacy, legacy_json).unwrap();

        let loaded = ChatLog::load_or_create(root, "bob").unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.session_id, "legacy-session");

        // legacy 文件应被删除
        assert!(!legacy.exists(), "迁移后旧 chat_log.json 应被删除");
        // 新格式文件位于 history/ 子目录
        assert!(char_dir.join("history").join("chat_log.jsonl").exists());
        assert!(char_dir.join("history").join("chat_log_meta.json").exists());

        // 再次 load 不重复迁移
        let reload = ChatLog::load_or_create(root, "bob").unwrap();
        assert_eq!(reload.messages.len(), 2);
    }

    #[test]
    fn test_pre_cf2_jsonl_migration() {
        // CF-2：pre-CF2 根目录 chat_log.jsonl 应被迁移到 history/ 下
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("carol");
        fs::create_dir_all(&char_dir).unwrap();

        // 手工写入 pre-CF2 jsonl（模拟 6.0e 旧数据）
        let old_jsonl = char_dir.join("chat_log.jsonl");
        let old_meta = char_dir.join("chat_log_meta.json");
        fs::write(
            &old_jsonl,
            "{\"role\":\"user\",\"content\":\"hello\"}\n\
             {\"role\":\"assistant\",\"content\":\"hi\"}\n",
        )
        .unwrap();
        let meta_json = serde_json::json!({
            "session_id": "pre-cf2-session",
            "character_id": "carol",
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-02T00:00:00Z"
        });
        fs::write(&old_meta, serde_json::to_string(&meta_json).unwrap()).unwrap();

        let loaded = ChatLog::load_or_create(root, "carol").unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.session_id, "pre-cf2-session");
        assert_eq!(loaded.messages[0].content, "hello");

        // 旧文件应被删除
        assert!(
            !old_jsonl.exists(),
            "迁移后旧根目录 chat_log.jsonl 应被删除"
        );
        assert!(
            !old_meta.exists(),
            "迁移后旧根目录 chat_log_meta.json 应被删除"
        );

        // 新文件位于 history/
        assert!(char_dir.join("history").join("chat_log.jsonl").exists());
        assert!(char_dir.join("history").join("chat_log_meta.json").exists());

        // 再次 load 不重复迁移
        let reload = ChatLog::load_or_create(root, "carol").unwrap();
        assert_eq!(reload.messages.len(), 2);
        assert_eq!(reload.session_id, "pre-cf2-session");
    }

    #[test]
    fn test_chat_log_rolling_truncation() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("characters").join("roller")).unwrap();

        let mut log = ChatLog::new("roller");
        // 写入 MAX_MESSAGES + 50 条；前 50 条应被丢弃，留下最后 MAX_MESSAGES 条。
        let total = MAX_MESSAGES + 50;
        for i in 0..total {
            log.append(
                root,
                ChatMessage {
                    role: if i % 2 == 0 {
                        crate::adapter::MessageRole::User
                    } else {
                        crate::adapter::MessageRole::Assistant
                    },
                    content: format!("msg-{}", i),
                },
            )
            .unwrap();
        }

        assert_eq!(log.messages.len(), MAX_MESSAGES);
        // 第一条应是 msg-50（前 50 条被滚动丢弃）
        assert_eq!(
            log.messages[0].content,
            format!("msg-{}", total - MAX_MESSAGES)
        );
        // 最后一条应是 msg-(total-1)
        assert_eq!(
            log.messages.last().unwrap().content,
            format!("msg-{}", total - 1)
        );
    }
}
