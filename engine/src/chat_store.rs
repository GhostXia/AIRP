use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::adapter::ChatMessage;
use crate::error::AirpError;
use crate::types::SessionId;
use crate::ulid;

/// 默认上下文读取上限（#37 / #122 长会话合同）。
///
/// **语义（2026-07-12 重定义）**：这是调用方选择近期上下文时可使用的默认上限，
/// **不是持久化删除阈值**。完整历史必须留在 `ChatLog` 和 jsonl 中；模型上下文由
/// `recent` 在读取边界裁剪。后续可引入流式分页读取来降低全量反序列化成本。
pub const MAX_MESSAGES: usize = 1000;

/// A complete chat log for one character session.
///
/// **CF-2 持久化模型**：消息列表写入 `history/chat_log.jsonl`（每行一条 JSON 消息），
/// 元数据（session_id / 时间戳）写入 `history/chat_log_meta.json`。
/// `append` 走 `OpenOptions::append` 实现 O(1) 追加；只有在迁移、delete_last_n、
/// rollback_to 等需要改变持久化真相的路径才会
/// 触发整体重写。
///
/// 迁移链：`chat_log.json`（<6.0e）→ `chat_log.jsonl`（6.0e，根目录）
/// → `history/chat_log.jsonl`（CF-2，history/ 子目录）。
/// `load_or_create` 自动处理全部迁移步骤。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLog {
    /// Canonical session identifier. Named sessions use the UUID from
    /// `characters/{id}/sessions/{session_id}/`; legacy per-character logs keep
    /// their historical chat-log UUID.
    pub session_id: String,
    /// Character folder name
    pub character_id: String,
    /// Ordered list of messages (user + assistant interleaved)
    pub messages: Vec<ChatMessage>,
    /// #37 durable message-id contract：每条消息的稳定 durable ID，与 `messages` 一一对应。
    ///
    /// - 新写入 → `ulid::new_id()`（UUIDv4-backed opaque ID，JSON Pointer 安全）。
    /// - 旧 jsonl 无 id → 加载时**确定性派生**（`ulid::derive_legacy_id(scope_salt, index)`），
    ///   同一 fixture 多次加载产生相同 ID（不写回 jsonl，lazy 兼容）。
    /// - 长度始终等于 `messages.len()`（save/append/delete/rollback 同步维护）。
    #[serde(default)]
    pub message_ids: Vec<String>,
    /// #73 方案 B：消息级时间戳（ISO 8601），与 `messages` 一一对应。
    ///
    /// 旧 jsonl 无 ts → 对应位置为 `None`（向后兼容，不强制迁移）。
    /// 新写入 → `Some(now)`。
    ///
    /// 长度始终等于 `messages.len()`（save/append/delete/rollback 同步维护）。
    #[serde(default)]
    pub message_timestamps: Vec<Option<String>>,
    /// ISO 8601 creation timestamp
    pub created_at: String,
    /// ISO 8601 last update timestamp
    pub updated_at: String,
    /// #85 O1：当前 ChatLog 所属的 scope session_id（由 `POST /v1/sessions/:character_id`
    /// 返回的 UUID）。`None` 表示 legacy per-character log。
    ///
    /// HTTP 响应时序列化（`Some` 才出现，`None` skip），让前端能把它与 session 列表
    /// 中的 id 关联。命名 session 中该值与 `ChatLog.session_id` 相同；保留此字段是为了
    /// 兼容既有响应形状并区分不带命名 session 的 legacy per-character log。
    /// 持久化时不写入（jsonl 用 `StoredMessage`，meta 用 `ChatLogMeta`，均不含此字段）；
    /// 反序列化时 `#[serde(default)]` 给 `None`，legacy JSON 迁移安全。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    scope_session_id: Option<String>,
}

/// 持久化在 `chat_log_meta.json` 中的小型元数据 (无 messages 字段)。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatLogMeta {
    session_id: String,
    character_id: String,
    created_at: String,
    updated_at: String,
}

/// #73 方案 B / #37 durable id：jsonl 行的持久化结构。
///
/// 用 `#[serde(flatten)]` 平铺 `ChatMessage`（role/content，OpenAI 协议兼容），
/// 额外存 `ts`（消息写入时间，ISO 8601）与 `id`（durable message ID）。
///
/// - 旧 jsonl（无 ts / id）→ deserialize 时 `ts: None` / `id: None`（向后兼容，不强制迁移）
/// - 新写入 → `ts: Some(now)` / `id: Some(ulid::new_id())`
///
/// `ChatLog.messages` 仍是 `Vec<ChatMessage>`（保持 OpenAI 协议兼容，durable id 不进
/// OpenAI 协议类型），`ts` / `id` 单独存在 `ChatLog.message_timestamps` /
/// `ChatLog.message_ids` 中，与 messages 一一对应。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMessage {
    #[serde(flatten)]
    msg: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
}

impl ChatLog {
    /// Creates a new empty chat log for a character.
    pub fn new(character_id: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            character_id: character_id.to_string(),
            messages: Vec::new(),
            message_ids: Vec::new(),
            message_timestamps: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            scope_session_id: None,
        }
    }

    /// Creates a new empty chat log for a named session scope.
    fn new_for_session(character_id: &str, session_id: &SessionId) -> Self {
        let mut log = Self::new(character_id);
        let session_id = session_id.to_string();
        log.session_id = session_id.clone();
        log.scope_session_id = Some(session_id);
        log
    }

    /// #85 O1：暴露 scope session id 给 HTTP 响应（`HistoryWindow.scope_session_id`），
    /// 让前端能把它与 session 列表关联。`None` = legacy per-character log。
    pub fn scope_session_id(&self) -> Option<&str> {
        self.scope_session_id.as_deref()
    }

    fn history_dir(
        data_root: &Path,
        character_id: &str,
        scope_session_id: Option<&str>,
    ) -> PathBuf {
        let character_dir = data_root.join("characters").join(character_id);
        match scope_session_id {
            Some(session_id) => character_dir
                .join("sessions")
                .join(session_id)
                .join("history"),
            None => character_dir.join("history"),
        }
    }

    fn scoped_jsonl_path(
        data_root: &Path,
        character_id: &str,
        scope_session_id: Option<&str>,
    ) -> PathBuf {
        Self::history_dir(data_root, character_id, scope_session_id).join("chat_log.jsonl")
    }

    fn scoped_meta_path(
        data_root: &Path,
        character_id: &str,
        scope_session_id: Option<&str>,
    ) -> PathBuf {
        Self::history_dir(data_root, character_id, scope_session_id).join("chat_log_meta.json")
    }

    /// 角色目录下消息 JSONL 文件路径（CF-2 位置：`history/` 子目录）。
    fn jsonl_path(data_root: &Path, character_id: &str) -> PathBuf {
        Self::scoped_jsonl_path(data_root, character_id, None)
    }

    /// 角色目录下元数据 JSON 文件路径（CF-2 位置：`history/` 子目录）。
    fn meta_path(data_root: &Path, character_id: &str) -> PathBuf {
        Self::scoped_meta_path(data_root, character_id, None)
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
        Self::load_or_create_for_session(data_root, character_id, None)
    }

    /// Loads or creates a chat log scoped to a named session when `session_id` is provided.
    ///
    /// `None` preserves the historical per-character log and migration behavior. Named sessions
    /// use `characters/{id}/sessions/{session_id}/history/` and intentionally do not import the
    /// legacy per-character chat log.
    ///
    /// #37 durable message-id contract:
    /// - 旧 jsonl 行无 `id` → 加载时**确定性派生**（`ulid::derive_legacy_id(scope_salt, index)`），
    ///   同一 fixture 多次加载产生相同 ID。派生 ID 只在内存 `ChatLog.message_ids` 里补，**不写回 jsonl**
    ///   （守 lazy + 不删旧文件原则，避免迁移半态）。
    /// - meta 丢失重建改**确定性派生**（hash `character_id` + scope 或 "legacy"），不再用 `Uuid::new_v4()`——
    ///   保证多次加载同一 fixture 产生同一 `session_id`（坐实"legacy fixture 多次加载产生相同 ID"验收）。
    pub fn load_or_create_for_session(
        data_root: &Path,
        character_id: &str,
        session_id: Option<&SessionId>,
    ) -> Result<Self, AirpError> {
        if let Some(session_id) = session_id {
            let scope_session_id = session_id.to_string();
            crate::data_dir::resolve_session_dir(data_root, character_id, Some(session_id))?;
            let jsonl = Self::scoped_jsonl_path(data_root, character_id, Some(&scope_session_id));
            let meta_p = Self::scoped_meta_path(data_root, character_id, Some(&scope_session_id));
            if jsonl.exists() {
                let salt = Self::legacy_scope_salt(character_id, Some(&scope_session_id));
                let parsed = Self::read_messages_jsonl(&jsonl, &salt)?;
                let meta_existed = meta_p.exists();
                let mut m: ChatLogMeta = if meta_existed {
                    serde_json::from_str(&fs::read_to_string(&meta_p)?)?
                } else {
                    // 命名 session 的规范 ID 就是目录 UUID；meta 丢失时从 scope 恢复。
                    Self::derive_meta(character_id, Some(&scope_session_id), &jsonl)
                };
                if !meta_existed || m.session_id != scope_session_id {
                    // 旧版本为 ChatLog 额外生成内部 UUID。加载时只归一化身份字段，
                    // 保留原 created_at / updated_at 和聊天内容。持久化迁移不能阻断
                    // 已存在历史的读取；失败时由下一次写操作再次保存规范 ID。
                    m.session_id = scope_session_id.clone();
                    let bytes = serde_json::to_vec_pretty(&m)?;
                    if let Err(error) = crate::data_dir::replace_file(&meta_p, &bytes) {
                        tracing::warn!(path = ?meta_p, err = %error, "命名 session metadata ID 归一化写入失败，继续读取历史");
                    }
                }
                let log = Self {
                    session_id: scope_session_id.clone(),
                    character_id: m.character_id,
                    messages: parsed.messages,
                    message_ids: parsed.message_ids,
                    message_timestamps: parsed.message_timestamps,
                    created_at: m.created_at,
                    updated_at: m.updated_at,
                    scope_session_id: Some(scope_session_id),
                };
                return Ok(log);
            }

            let log = ChatLog::new_for_session(character_id, session_id);
            log.save(data_root)?;
            return Ok(log);
        }

        let jsonl = Self::jsonl_path(data_root, character_id);
        let meta_p = Self::meta_path(data_root, character_id);
        let pre_cf2_jsonl = Self::pre_cf2_jsonl_path(data_root, character_id);
        let pre_cf2_meta = Self::pre_cf2_meta_path(data_root, character_id);
        let legacy = Self::legacy_path(data_root, character_id);

        // ── 1. CF-2 新位置 ────────────────────────────────────────────────────
        if jsonl.exists() {
            let salt = Self::legacy_scope_salt(character_id, None);
            let parsed = Self::read_messages_jsonl(&jsonl, &salt)?;
            let m: ChatLogMeta = if meta_p.exists() {
                serde_json::from_str(&fs::read_to_string(&meta_p)?)?
            } else {
                // meta 丢失 → 确定性派生（不再随机 UUID）。
                Self::derive_meta(character_id, None, &jsonl)
            };
            let log = Self {
                session_id: m.session_id,
                character_id: m.character_id,
                messages: parsed.messages,
                message_ids: parsed.message_ids,
                message_timestamps: parsed.message_timestamps,
                created_at: m.created_at,
                updated_at: m.updated_at,
                scope_session_id: None,
            };
            return Ok(log);
        }

        // ── 2. pre-CF2 迁移：根目录 chat_log.jsonl → history/ ─────────────────
        if pre_cf2_jsonl.exists() {
            tracing::info!(char = character_id, "CF-2 迁移: chat_log.jsonl → history/");
            let salt = Self::legacy_scope_salt(character_id, None);
            let parsed = Self::read_messages_jsonl(&pre_cf2_jsonl, &salt)?;
            let m: ChatLogMeta = if pre_cf2_meta.exists() {
                serde_json::from_str(&fs::read_to_string(&pre_cf2_meta)?)?
            } else {
                Self::derive_meta(character_id, None, &pre_cf2_jsonl)
            };
            let log = Self {
                session_id: m.session_id,
                character_id: m.character_id,
                messages: parsed.messages,
                message_ids: parsed.message_ids,
                message_timestamps: parsed.message_timestamps,
                created_at: m.created_at,
                updated_at: m.updated_at,
                scope_session_id: None,
            };
            log.save(data_root)?;
            // 删除旧文件，失败不阻塞（新位置已写；下次加载走新位置）
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
            let mut log: ChatLog = serde_json::from_str(&content)?;
            log.scope_session_id = None;
            // #73 方案 B：旧 ChatLog JSON 无 message_timestamps 字段（#[serde(default)]
            // 给空 Vec），但 messages 有内容 → 长度不匹配。补齐为全 None。
            if log.message_timestamps.len() != log.messages.len() {
                log.message_timestamps = log.messages.iter().map(|_| None).collect();
            }
            // #37：旧 ChatLog JSON 无 message_ids 字段（#[serde(default)] 给空 Vec），
            // 但 messages 有内容 → 长度不匹配。确定性派生补齐（legacy fixture 多次加载同 ID）。
            if log.message_ids.len() != log.messages.len() {
                let salt = Self::legacy_scope_salt(character_id, None);
                log.message_ids = log
                    .messages
                    .iter()
                    .enumerate()
                    .map(|(i, _)| ulid::derive_legacy_id(&salt, i))
                    .collect();
            }
            // A-2：迁移后验证等长不变量
            debug_assert_eq!(
                log.message_timestamps.len(),
                log.messages.len(),
                "legacy JSON 迁移后 message_timestamps.len() != messages.len()"
            );
            debug_assert_eq!(
                log.message_ids.len(),
                log.messages.len(),
                "legacy JSON 迁移后 message_ids.len() != messages.len()"
            );
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

    /// 确定性派生 `ChatLogMeta`（用于 meta 丢失重建）。
    ///
    /// `session_id` 用 `character_id` + scope 的稳定 hash 派生（不再随机 UUID），
    /// 保证同一 fixture 多次加载产生同一 `session_id`。`created_at` / `updated_at`
    /// 取 jsonl 文件的 mtime（若读不到则 fallback 到 epoch）——meta 丢失本身是边缘场景，
    /// 时间精度损失可接受，关键是 session_id 稳定。
    fn derive_meta(character_id: &str, scope: Option<&str>, jsonl_path: &Path) -> ChatLogMeta {
        // 稳定 hash：FNV-1a over (character_id ++ scope_or_"legacy")，输出格式化成 UUIDv4 形。
        let salt = scope.unwrap_or("legacy");
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in character_id.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for &b in salt.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        // 把 64-bit hash 展成 UUIDv4 形字符串（8-4-4-4-12），够稳定且形如旧 meta。
        let session_id = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (h & 0xFFFF_FFFF) as u32,
            ((h >> 32) & 0xFFFF) as u16,
            ((h >> 48) & 0xFFFF) as u16,
            (h.wrapping_mul(31) & 0xFFFF) as u16,
            h.wrapping_mul(97) & 0xFFFF_FFFF_FFFF
        );
        // meta 丢失时用 history 文件 mtime 恢复可排序时间；仅 metadata 不可读时回退 epoch。
        let recovered_at = fs::metadata(jsonl_path)
            .and_then(|metadata| metadata.modified())
            .map(chrono::DateTime::<Utc>::from)
            .map(|timestamp| timestamp.to_rfc3339())
            .unwrap_or_else(|_| "1970-01-01T00:00:00+00:00".to_string());
        ChatLogMeta {
            session_id,
            character_id: character_id.to_string(),
            created_at: recovered_at.clone(),
            updated_at: recovered_at,
        }
    }

    /// 用于 legacy 派生 ID 的 scope salt：`character_id` 或 `character_id/sessions/{sid}`。
    fn legacy_scope_salt(character_id: &str, scope: Option<&str>) -> String {
        match scope {
            Some(sid) => format!("{character_id}/sessions/{sid}"),
            None => format!("{character_id}/legacy"),
        }
    }

    /// 整体重写 jsonl + meta（用于 delete/rollback）。
    ///
    /// **#37 注意**：`save` 永远写**全量** `messages`。上下文窗口由 `recent` 在读取时
    /// 裁剪，不能改变持久化历史。`save` 只被迁移、delete、rollback 等路径调用。
    pub fn save(&self, data_root: &Path) -> Result<(), AirpError> {
        let scope = self.scope_session_id.as_deref();
        let jsonl = Self::scoped_jsonl_path(data_root, &self.character_id, scope);
        let meta = Self::scoped_meta_path(data_root, &self.character_id, scope);

        // 确保 history/ 目录存在
        if let Some(parent) = jsonl.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        // 写 jsonl：一行一条 StoredMessage（含 id + ts）
        let mut buf = String::new();
        for (i, m) in self.messages.iter().enumerate() {
            let id = self.message_ids.get(i).cloned();
            let ts = self.message_timestamps.get(i).cloned().flatten();
            let stored = StoredMessage {
                msg: m.clone(),
                id,
                ts,
            };
            buf.push_str(&serde_json::to_string(&stored)?);
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
    ///
    /// #37 durable message-id contract：
    /// - 每条新消息生成 UUIDv4-backed ID，写入 jsonl 行 + 内存 `message_ids`。
    /// - **`MAX_MESSAGES` 不再物理删除消息**：jsonl 和内存 `ChatLog` 都保留完整历史；
    ///   orchestrator 通过 `recent(MAX_MESSAGES)` 获取有界上下文。这样分页、回滚和保存
    ///   始终基于同一份完整历史，避免窗口态覆盖持久化真相。
    ///
    /// #73 方案 B：同时写入消息级 `ts`（ISO 8601 now），并同步 push 到
    /// `message_timestamps` 保持与 `messages` 等长。
    pub fn append(&mut self, data_root: &Path, msg: ChatMessage) -> Result<(), AirpError> {
        let now = Utc::now().to_rfc3339();
        let id = ulid::new_id();
        self.messages.push(msg.clone());
        self.message_ids.push(id.clone());
        self.message_timestamps.push(Some(now.clone()));

        // 常规追加：jsonl O(1) 写入 + meta 小文件刷新。
        let scope = self.scope_session_id.as_deref();
        let jsonl = Self::scoped_jsonl_path(data_root, &self.character_id, scope);
        // 文件可能首次创建（迁移路径已 ensure，但保底处理）
        if let Some(parent) = jsonl.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let stored = StoredMessage {
            msg,
            id: Some(id),
            ts: Some(now),
        };
        let mut line = serde_json::to_string(&stored)?;
        line.push('\n');
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl)?;
        f.write_all(line.as_bytes())?;

        // meta 刷新
        self.updated_at = Utc::now().to_rfc3339();
        let m = ChatLogMeta {
            session_id: self.session_id.clone(),
            character_id: self.character_id.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        };
        let meta_path = Self::scoped_meta_path(data_root, &self.character_id, scope);
        fs::write(&meta_path, serde_json::to_string_pretty(&m)?)?;

        Ok(())
    }

    /// Deletes the last N messages (for regen: delete last assistant message).
    ///
    /// #73 方案 B / #37：同步截断 `message_timestamps` / `message_ids` 保持等长。
    pub fn delete_last_n(&mut self, data_root: &Path, n: usize) -> Result<(), AirpError> {
        let len = self.messages.len();
        if n > len {
            self.messages.clear();
            self.message_ids.clear();
            self.message_timestamps.clear();
        } else {
            self.messages.truncate(len - n);
            self.message_ids.truncate(len - n);
            self.message_timestamps.truncate(len - n);
        }
        self.updated_at = Utc::now().to_rfc3339();
        self.save(data_root)
    }

    /// Rolls back to a specific message index (keeps messages 0..=index).
    ///
    /// #73 方案 B / #37：同步截断 `message_timestamps` / `message_ids` 保持等长。
    pub fn rollback_to(&mut self, data_root: &Path, index: usize) -> Result<(), AirpError> {
        let len = self.messages.len();
        if (len == 0 && index != 0) || (len > 0 && index >= len) {
            return Err(AirpError::BadRequest(format!(
                "rollback index {index} out of range (total messages: {len})"
            )));
        }
        if len > 0 {
            self.messages.truncate(index + 1);
            self.message_ids.truncate(index + 1);
            self.message_timestamps.truncate(index + 1);
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
    ///
    /// #37 durable id：返回 `(messages, message_ids, timestamps)`。
    /// - `StoredMessage.id` 为 `Some` → 直接用（新写入路径）。
    /// - `StoredMessage.id` 为 `None` → **确定性派生**（`ulid::derive_legacy_id(scope_salt, index)`），
    ///   同一 legacy fixture 多次加载产生相同 ID。派生 ID 不写回 jsonl（lazy 兼容）。
    ///
    /// `scope_salt` 来自 character/session 的逻辑身份，而不是绝对路径，因此移动数据根目录
    /// 或恢复备份不会改变 legacy 消息 ID。
    fn read_messages_jsonl(path: &Path, scope_salt: &str) -> Result<JsonlParseResult, AirpError> {
        let content = fs::read_to_string(path)?;
        let mut msgs = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        let mut tss = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let stored: StoredMessage = serde_json::from_str(line).map_err(|e| {
                AirpError::Internal(format!("chat_log.jsonl 第 {} 行解析失败: {}", i + 1, e))
            })?;
            // #37：无 id 的 legacy 行 → 确定性派生（同 fixture 多次加载同 ID）。
            let id = stored
                .id
                .unwrap_or_else(|| ulid::derive_legacy_id(scope_salt, msgs.len()));
            msgs.push(stored.msg);
            ids.push(id);
            tss.push(stored.ts);
        }
        // A-2：等长不变量防御。三 Vec 在同一循环中 push，理论上永远等长。
        debug_assert_eq!(
            msgs.len(),
            tss.len(),
            "read_messages_jsonl: msgs.len() != tss.len()"
        );
        debug_assert_eq!(
            msgs.len(),
            ids.len(),
            "read_messages_jsonl: msgs.len() != ids.len()"
        );
        Ok(JsonlParseResult {
            messages: msgs,
            message_ids: ids,
            message_timestamps: tss,
        })
    }
}

/// `read_messages_jsonl` 的返回聚合（避免 clippy::type_complexity 巨元 tuple）。
struct JsonlParseResult {
    messages: Vec<ChatMessage>,
    message_ids: Vec<String>,
    message_timestamps: Vec<Option<String>>,
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
    fn named_session_uses_one_canonical_session_id() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sid = SessionId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let log = ChatLog::load_or_create_for_session(root, "alice", Some(&sid)).unwrap();

        assert_eq!(log.session_id, sid.to_string());
        assert_eq!(log.scope_session_id(), Some(sid.to_string().as_str()));

        let meta_path = root
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(sid.to_string())
            .join("history")
            .join("chat_log_meta.json");
        let meta: ChatLogMeta =
            serde_json::from_str(&fs::read_to_string(meta_path).unwrap()).unwrap();
        assert_eq!(meta.session_id, sid.to_string());
    }

    #[test]
    fn named_session_normalizes_legacy_internal_id() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sid = SessionId::parse("550e8400-e29b-41d4-a716-446655440001").unwrap();
        let history_dir = root
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(sid.to_string())
            .join("history");
        fs::create_dir_all(&history_dir).unwrap();
        fs::write(
            history_dir.join("chat_log.jsonl"),
            "{\"role\":\"user\",\"content\":\"hello\"}\n",
        )
        .unwrap();
        fs::write(
            history_dir.join("chat_log_meta.json"),
            serde_json::to_string_pretty(&ChatLogMeta {
                session_id: "legacy-internal-id".to_string(),
                character_id: "alice".to_string(),
                created_at: "2025-01-01T00:00:00Z".to_string(),
                updated_at: "2025-01-02T00:00:00Z".to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        let log = ChatLog::load_or_create_for_session(root, "alice", Some(&sid)).unwrap();
        assert_eq!(log.session_id, sid.to_string());

        let meta: ChatLogMeta = serde_json::from_str(
            &fs::read_to_string(history_dir.join("chat_log_meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta.session_id, sid.to_string());
        assert_eq!(meta.created_at, "2025-01-01T00:00:00Z");
    }

    #[test]
    fn named_session_read_survives_metadata_repair_failure() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sid = SessionId::parse("550e8400-e29b-41d4-a716-446655440002").unwrap();
        let history_dir = root
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(sid.to_string())
            .join("history");
        fs::create_dir_all(&history_dir).unwrap();
        fs::write(
            history_dir.join("chat_log.jsonl"),
            "{\"role\":\"user\",\"content\":\"recoverable\"}\n",
        )
        .unwrap();

        let meta_path = history_dir.join("chat_log_meta.json");
        fs::write(
            &meta_path,
            serde_json::to_string_pretty(&ChatLogMeta {
                session_id: "legacy-internal-id".to_string(),
                character_id: "alice".to_string(),
                created_at: "2025-01-01T00:00:00Z".to_string(),
                updated_at: "2025-01-02T00:00:00Z".to_string(),
            })
            .unwrap(),
        )
        .unwrap();
        let temporary = meta_path.with_extension("json.tmp");
        fs::create_dir(&temporary).unwrap();

        let meta_permissions = fs::metadata(&meta_path).unwrap().permissions();
        let mut readonly_meta_permissions = meta_permissions.clone();
        readonly_meta_permissions.set_readonly(true);
        fs::set_permissions(&meta_path, readonly_meta_permissions).unwrap();

        let result = ChatLog::load_or_create_for_session(root, "alice", Some(&sid));

        fs::set_permissions(&meta_path, meta_permissions).unwrap();

        let log = result.expect("metadata repair failure must not block history reads");
        assert_eq!(log.session_id, sid.to_string());
        assert_eq!(log.messages[0].content, "recoverable");
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
        // 写入 MAX_MESSAGES + 50 条；完整历史不得因上下文上限而丢弃。
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

        assert_eq!(log.messages.len(), total);
        assert_eq!(log.messages[0].content, "msg-0");
        // 最后一条应是 msg-(total-1)
        assert_eq!(
            log.messages.last().unwrap().content,
            format!("msg-{}", total - 1)
        );
    }

    // ── #73 方案 B：消息级时间戳回归测试 ──────────────────────────────────

    #[test]
    fn test_message_timestamps_persisted_after_append() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("characters").join("ts_char")).unwrap();

        let mut log = ChatLog::new("ts_char");
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "msg1".to_string(),
            },
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content: "msg2".to_string(),
            },
        )
        .unwrap();

        // 内存状态：timestamps 等长，每条都有 ts
        assert_eq!(log.message_timestamps.len(), 2);
        assert!(log.message_timestamps[0].is_some());
        assert!(log.message_timestamps[1].is_some());

        // 重新加载：ts 应持久化
        let reloaded = ChatLog::load_or_create(root, "ts_char").unwrap();
        assert_eq!(reloaded.message_timestamps.len(), 2);
        assert!(reloaded.message_timestamps[0].is_some());
        assert!(reloaded.message_timestamps[1].is_some());
        // ts 应能 parse 为有效时间
        let ts0 = reloaded.message_timestamps[0].as_ref().unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(ts0).is_ok());
    }

    #[test]
    fn test_message_timestamps_back_compat_old_jsonl() {
        // 模拟旧格式 jsonl（无 ts 字段）→ 加载时应回退为 None
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("old_char");
        let history_dir = char_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();

        // 写旧格式 jsonl：纯 {"role":"user","content":"legacy"}
        let old_jsonl = history_dir.join("chat_log.jsonl");
        fs::write(
            &old_jsonl,
            r#"{"role":"user","content":"legacy1"}
{"role":"assistant","content":"legacy2"}
"#,
        )
        .unwrap();
        // 写最小 meta
        let now = Utc::now().to_rfc3339();
        fs::write(
            history_dir.join("chat_log_meta.json"),
            serde_json::to_string_pretty(&ChatLogMeta {
                session_id: "test-session".to_string(),
                character_id: "old_char".to_string(),
                created_at: now.clone(),
                updated_at: now,
            })
            .unwrap(),
        )
        .unwrap();

        let mut log = ChatLog::load_or_create(root, "old_char").unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(log.message_timestamps.len(), 2);
        // 旧消息无 ts → None
        assert!(log.message_timestamps[0].is_none());
        assert!(log.message_timestamps[1].is_none());

        // append 新消息 → 新消息有 ts，旧消息仍 None
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "new".to_string(),
            },
        )
        .unwrap();
        // 但 append 会触发 save 重写 jsonl — 这里只测内存状态
    }

    #[test]
    fn test_message_timestamps_mixed_old_new_jsonl() {
        // W-01：旧 jsonl（无 ts）+ append 新消息（有 ts）→ save 重写后 reload，
        //       验证旧行 ts 仍 None，新行 ts 有值，混合场景下对应关系正确。
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("mixed_char");
        let history_dir = char_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();

        // 写旧格式 jsonl：2 行无 ts
        let old_jsonl = history_dir.join("chat_log.jsonl");
        fs::write(
            &old_jsonl,
            r#"{"role":"user","content":"legacy1"}
{"role":"assistant","content":"legacy2"}
"#,
        )
        .unwrap();
        let now = Utc::now().to_rfc3339();
        fs::write(
            history_dir.join("chat_log_meta.json"),
            serde_json::to_string_pretty(&ChatLogMeta {
                session_id: "mixed-session".to_string(),
                character_id: "mixed_char".to_string(),
                created_at: now.clone(),
                updated_at: now,
            })
            .unwrap(),
        )
        .unwrap();

        // load → 旧 2 行 ts=None
        let mut log = ChatLog::load_or_create(root, "mixed_char").unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(log.message_timestamps.len(), 2);
        assert!(log.message_timestamps[0].is_none());
        assert!(log.message_timestamps[1].is_none());

        // append 1 条新消息 → 触发 save 重写 jsonl（旧行无 ts + 新行有 ts 混合）
        std::thread::sleep(std::time::Duration::from_millis(10));
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "new_msg".to_string(),
            },
        )
        .unwrap();

        // reload → 3 行：旧 2 行 ts=None，新 1 行 ts=Some
        let reloaded = ChatLog::load_or_create(root, "mixed_char").unwrap();
        assert_eq!(reloaded.messages.len(), 3);
        assert_eq!(reloaded.message_timestamps.len(), 3);
        assert!(
            reloaded.message_timestamps[0].is_none(),
            "旧行 ts 应为 None"
        );
        assert!(
            reloaded.message_timestamps[1].is_none(),
            "旧行 ts 应为 None"
        );
        assert!(reloaded.message_timestamps[2].is_some(), "新行 ts 应有值");
        assert_eq!(reloaded.messages[2].content, "new_msg");
        // 新行 ts 能 parse 为有效时间
        let ts_new = reloaded.message_timestamps[2].as_ref().unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(ts_new).is_ok());
    }

    #[test]
    fn test_message_timestamps_delete_last_n_keeps_sync() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("characters").join("del_char")).unwrap();

        let mut log = ChatLog::new("del_char");
        for i in 0..5 {
            log.append(
                root,
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: format!("msg{i}"),
                },
            )
            .unwrap();
        }
        assert_eq!(log.message_timestamps.len(), 5);

        // 删 2 条
        log.delete_last_n(root, 2).unwrap();
        assert_eq!(log.messages.len(), 3);
        assert_eq!(log.message_timestamps.len(), 3);
        // 重载验证持久化
        let reloaded = ChatLog::load_or_create(root, "del_char").unwrap();
        assert_eq!(reloaded.message_timestamps.len(), 3);
    }

    #[test]
    fn test_message_timestamps_rollback_keeps_sync() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("characters").join("rb_char")).unwrap();

        let mut log = ChatLog::new("rb_char");
        for i in 0..5 {
            log.append(
                root,
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: format!("msg{i}"),
                },
            )
            .unwrap();
        }
        // rollback 到 index 1（保留 0..=1）
        log.rollback_to(root, 1).unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(log.message_timestamps.len(), 2);
        // 重载验证
        let reloaded = ChatLog::load_or_create(root, "rb_char").unwrap();
        assert_eq!(reloaded.message_timestamps.len(), 2);
        assert_eq!(reloaded.messages[0].content, "msg0");
        assert_eq!(reloaded.messages[1].content, "msg1");
    }

    #[test]
    fn rollback_to_rejects_out_of_range_without_mutation() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        make_char_dir(root, "rb_range_char");

        let mut log = ChatLog::new("rb_range_char");
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "msg0".into(),
            },
        )
        .unwrap();

        let err = log.rollback_to(root, 1).unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));
        assert_eq!(log.messages.len(), 1);
        let reloaded = ChatLog::load_or_create(root, "rb_range_char").unwrap();
        assert_eq!(reloaded.messages.len(), 1);
    }

    #[test]
    fn rollback_to_preserves_empty_log_compatibility() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        make_char_dir(root, "rb_empty_char");

        let mut log = ChatLog::new("rb_empty_char");
        log.rollback_to(root, 0).unwrap();
        let err = log.rollback_to(root, 1).unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));
    }

    // ── #37 durable message-id contract 不变式 ──────────────────────────────

    fn make_char_dir(root: &Path, cid: &str) {
        fs::create_dir_all(root.join("characters").join(cid)).unwrap();
    }

    #[test]
    fn durable_id_unique_within_session() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        make_char_dir(root, "uniq_char");
        let mut log = ChatLog::new("uniq_char");
        for _ in 0..5 {
            log.append(
                root,
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: "x".to_string(),
                },
            )
            .unwrap();
        }
        // 不变量 1：同 session 内任意两条 durable ID 不同。
        let mut seen = std::collections::HashSet::new();
        for id in &log.message_ids {
            assert!(seen.insert(id.clone()), "duplicate durable id: {id}");
        }
    }

    #[test]
    fn durable_id_stable_across_reload() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        make_char_dir(root, "stable_char");
        let mut log = ChatLog::new("stable_char");
        log.append(
            root,
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "first".to_string(),
            },
        )
        .unwrap();
        let id_before = log.message_ids[0].clone();

        // 重启模拟：reload 同 fixture。
        let reloaded = ChatLog::load_or_create(root, "stable_char").unwrap();
        // 不变量 2：消息落盘后，重启 / 多次 load → 同一消息同一 ID。
        assert_eq!(reloaded.message_ids.len(), 1);
        assert_eq!(reloaded.message_ids[0], id_before);
    }

    #[test]
    fn legacy_derived_id_stable_across_loads() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let cid = "legacy_char";
        make_char_dir(root, cid);

        // 手写一行无 id 的 legacy jsonl（模拟旧 fixture）。
        let jsonl = root
            .join("characters")
            .join(cid)
            .join("history")
            .join("chat_log.jsonl");
        fs::create_dir_all(jsonl.parent().unwrap()).unwrap();
        let legacy_line = r#"{"role":"user","content":"hello"}"#;
        fs::write(&jsonl, format!("{legacy_line}\n")).unwrap();

        // 不变量 3：同一 legacy fixture 多次加载 → 同一派生 ID。
        let a = ChatLog::load_or_create(root, cid).unwrap();
        let b = ChatLog::load_or_create(root, cid).unwrap();
        assert_eq!(a.message_ids.len(), 1);
        assert_eq!(
            a.message_ids[0], b.message_ids[0],
            "legacy derive must be stable"
        );
        // 派生 ID 形如 m0…（legacy marker）。
        assert!(
            a.message_ids[0].starts_with("m0"),
            "derived id carries zero-ts marker"
        );
    }

    #[test]
    fn legacy_derived_ids_ignore_blank_jsonl_lines() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let cid = "legacy_blank_lines";
        make_char_dir(root, cid);
        let jsonl = root
            .join("characters")
            .join(cid)
            .join("history")
            .join("chat_log.jsonl");
        fs::create_dir_all(jsonl.parent().unwrap()).unwrap();
        let first = r#"{"role":"user","content":"first"}"#;
        let second = r#"{"role":"assistant","content":"second"}"#;
        fs::write(&jsonl, format!("{first}\n\n{second}\n")).unwrap();
        let with_blank = ChatLog::load_or_create(root, cid).unwrap().message_ids;

        fs::write(&jsonl, format!("{first}\n{second}\n")).unwrap();
        let without_blank = ChatLog::load_or_create(root, cid).unwrap().message_ids;
        assert_eq!(with_blank, without_blank);
    }

    #[test]
    fn legacy_meta_loss_rebuild_is_deterministic() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let cid = "meta_loss_char";
        make_char_dir(root, cid);

        // 写 jsonl 但不写 meta → meta 丢失路径。
        let jsonl = root
            .join("characters")
            .join(cid)
            .join("history")
            .join("chat_log.jsonl");
        fs::create_dir_all(jsonl.parent().unwrap()).unwrap();
        fs::write(
            &jsonl,
            r#"{"role":"user","content":"a"}
"#,
        )
        .unwrap();

        // 多次加载 → derive_meta 产出同一 session_id（不再随机 UUID）。
        let a = ChatLog::load_or_create(root, cid).unwrap();
        let b = ChatLog::load_or_create(root, cid).unwrap();
        assert_eq!(
            a.session_id, b.session_id,
            "meta-loss derive must be deterministic"
        );
        assert_ne!(a.updated_at, "1970-01-01T00:00:00+00:00");
        assert_eq!(a.updated_at, b.updated_at);
    }

    #[test]
    fn ids_timestamps_messages_equal_length_after_mutations() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let cid = "eq_char";
        make_char_dir(root, cid);
        let mut log = ChatLog::new(cid);
        for _ in 0..3 {
            log.append(
                root,
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: "x".to_string(),
                },
            )
            .unwrap();
        }
        // 不变量 4：三 Vec 等长。
        assert_eq!(
            log.messages.len(),
            log.message_ids.len(),
            "messages.len() != message_ids.len()"
        );
        assert_eq!(
            log.messages.len(),
            log.message_timestamps.len(),
            "messages.len() != message_timestamps.len()"
        );

        // rollback 后仍等长。
        log.rollback_to(root, 0).unwrap();
        assert_eq!(log.messages.len(), 1);
        assert_eq!(log.messages.len(), log.message_ids.len());
        assert_eq!(log.messages.len(), log.message_timestamps.len());

        // delete_last_n 后仍等长。
        log.delete_last_n(root, 1).unwrap();
        assert_eq!(log.messages.len(), 0);
        assert_eq!(log.messages.len(), log.message_ids.len());
        assert_eq!(log.messages.len(), log.message_timestamps.len());
    }

    #[test]
    fn max_messages_does_not_delete_persistence() {
        // 不变量 8：append 超量后，jsonl 与 ChatLog 都保留全部消息。
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let cid = "cap_char";
        make_char_dir(root, cid);
        let mut log = ChatLog::new(cid);
        // 写 MAX_MESSAGES + 5 条。
        for i in 0..(MAX_MESSAGES + 5) {
            log.append(
                root,
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: format!("msg{i}"),
                },
            )
            .unwrap();
        }
        // 内存态与持久化态都保留完整历史；上下文限制只在 recent() 读取时应用。
        assert_eq!(
            log.messages.len(),
            MAX_MESSAGES + 5,
            "ChatLog must retain the complete history"
        );
        assert_eq!(log.message_ids.len(), MAX_MESSAGES + 5);
        assert_eq!(log.message_timestamps.len(), MAX_MESSAGES + 5);
        assert_eq!(log.recent(MAX_MESSAGES).len(), MAX_MESSAGES);

        // jsonl 物理全留：reload 走 read_messages_jsonl 读全量行数。
        let jsonl = root
            .join("characters")
            .join(cid)
            .join("history")
            .join("chat_log.jsonl");
        let raw = fs::read_to_string(&jsonl).unwrap();
        let line_count = raw.lines().filter(|l| !l.trim().is_empty()).count();
        assert_eq!(
            line_count,
            MAX_MESSAGES + 5,
            "jsonl must retain all messages (no physical delete)"
        );

        // reload 仍可访问最早消息，供 cursor 分页和按 ID 回滚。
        let reloaded = ChatLog::load_or_create(root, cid).unwrap();
        assert_eq!(reloaded.messages.len(), MAX_MESSAGES + 5);
        assert_eq!(reloaded.messages[0].content, "msg0");
    }
}
