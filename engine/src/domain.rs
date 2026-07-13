//! Shared domain services used by HTTP, Tauri-facing pipelines, and Agent tools.
//!
//! Transport adapters must not implement their own persistence locking or
//! rollback semantics.  `ChatService` is the single boundary for chat/session
//! mutations and character deletion.

use std::collections::HashMap;
use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::adapter::ChatMessage;
use crate::chat_store::ChatLog;
use crate::data_dir;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId, UserId};
use crate::ulid;

type SessionLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;
type CharacterLockMap = Mutex<HashMap<String, Arc<RwLock<()>>>>;
type StateLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;
type PersonaLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;

static SESSION_LOCKS: OnceLock<SessionLockMap> = OnceLock::new();
static CHARACTER_LOCKS: OnceLock<CharacterLockMap> = OnceLock::new();
static STATE_LOCKS: OnceLock<StateLockMap> = OnceLock::new();
static PERSONA_LOCKS: OnceLock<PersonaLockMap> = OnceLock::new();

pub(crate) fn character_lock(character_id: &str) -> Arc<RwLock<()>> {
    let mut locks = CHARACTER_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("character lock map poisoned");
    locks
        .entry(character_id.to_string())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

fn session_lock(character_id: &str, session_id: Option<&SessionId>) -> Arc<Mutex<()>> {
    let key = match session_id {
        Some(session_id) => format!("{character_id}/{session_id}"),
        None => character_id.to_string(),
    };
    let mut locks = SESSION_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("session lock map poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn remove_deleted_session_lock(character_id: &str, session_id: &SessionId) {
    let Some(lock_map) = SESSION_LOCKS.get() else {
        return;
    };
    let key = format!("{character_id}/{session_id}");
    let mut locks = lock_map.lock().expect("session lock map poisoned");
    // The tombstone is durable before this runs, so every waiter or future
    // caller will fail closed even if it holds/creates a different lock Arc.
    locks.remove(&key);
}

fn state_lock(character_id: &str) -> Arc<Mutex<()>> {
    let mut locks = STATE_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("state lock map poisoned");
    locks
        .entry(character_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Per-user persona lock（串行化 persona 写入与 revision bump）。
fn persona_lock(user_id: &str) -> Arc<Mutex<()>> {
    let mut locks = PERSONA_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("persona lock map poisoned");
    locks
        .entry(user_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[derive(Clone, Debug)]
pub struct ChatService {
    data_root: PathBuf,
}

/// #37 cursor 分页窗口（`ChatService::history_window` 返回）。
///
/// `messages` / `message_ids` / `message_timestamps` 等长，按时间正序排列，
/// 是原 session 的一个切片（更早的一段或最近的一段）。
///
/// - `has_more`：cursor 之前还有更早消息可加载。
/// - `oldest_id`：本窗口最老消息的 durable ID，前端下次作 `before` cursor。
/// - `total`：session 消息总数（含未加载），前端显示 "X / N"。
/// - `scope_session_id`：#85 O1——当前 window 所属的 scope session id（None = legacy），
///   前端用它关联 session 列表。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryWindow {
    pub messages: Vec<ChatMessage>,
    pub message_ids: Vec<String>,
    pub message_timestamps: Vec<Option<String>>,
    pub has_more: bool,
    pub oldest_id: Option<String>,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_session_id: Option<String>,
}

impl ChatService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    fn with_session<R>(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        operation: impl FnOnce() -> Result<R, AirpError>,
    ) -> Result<R, AirpError> {
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().expect("character lock poisoned");
        let session = session_lock(character_id.as_str(), session_id);
        let _session_guard = session.lock().expect("session lock poisoned");
        // A never-seen named ID retains the legacy lazy-create behavior. Only
        // an explicitly deleted ID is rejected, using a tombstone so it cannot
        // be silently revived by load_or_create_for_session.
        if let Some(sid) = session_id {
            if data_dir::session_was_deleted(&self.data_root, character_id.as_str(), sid) {
                return Err(AirpError::NotFound(format!(
                    "session {sid} for character {character_id} not found"
                )));
            }
        }
        operation()
    }

    pub fn history(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            ChatLog::load_or_create_for_session(&self.data_root, character_id.as_str(), session_id)
        })
    }

    /// #37 cursor 分页窗口：返回 `before` ID 严格之前（更早）的消息，limit 上界。
    ///
    /// **cursor 语义**：`before` = 某条消息的 durable ID，返回该 ID **严格之前**的消息
    /// （更早的），按时间正序排列。`before` 必须命中当前 session 的某条 durable ID
    /// （含 legacy 派生 ID），否则 `BadRequest`——**cursor 不能跨 character/session 使用**。
    ///
    /// 不传 `before` → 返回最近 `limit` 条（时间正序）。
    /// 不传 `limit` → 默认 50；上界 200，超过 clamp。
    ///
    /// `has_more` = cursor 之前还有更早消息。`oldest_id` = 本窗口里最老消息的 ID，
    /// 供前端下次作 `before`。`total` = session 消息总数（含未加载）。
    pub fn history_window(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<HistoryWindow, AirpError> {
        let limit = limit.unwrap_or(50).clamp(1, 200);
        let log = self.history(character_id, session_id)?;
        let total = log.messages.len();

        // 找 cursor 切点：before ID 在 message_ids 里的位置；返回该位置严格之前。
        let cut = match before {
            Some(id) => {
                if !ulid::is_valid_id(id) {
                    return Err(AirpError::BadRequest(format!(
                        "cursor is not a valid durable message id: {id}"
                    )));
                }
                let idx = log
                    .message_ids
                    .iter()
                    .position(|x| ulid::matches(x, id))
                    .ok_or_else(|| {
                        AirpError::BadRequest(format!(
                            "cursor {id} not in this session (cursor cannot cross character/session)"
                        ))
                    })?;
                idx // 返回 [0, idx) 即更早的
            }
            None => total, // 无 cursor → 取最近 limit 条 = 尾部
        };

        // 窗口 = [start, end)，按时间正序。
        let end = cut.min(total);
        let start = end.saturating_sub(limit);
        let window_messages = log.messages[start..end].to_vec();
        let window_ids = log.message_ids[start..end].to_vec();
        let window_ts = log.message_timestamps[start..end].to_vec();

        // has_more = 切点之前还有消息（start > 0）。
        let has_more = start > 0;
        // oldest_id = 本窗口最老消息的 ID（窗口首条）。
        let oldest_id = window_ids.first().cloned();

        Ok(HistoryWindow {
            messages: window_messages,
            message_ids: window_ids,
            message_timestamps: window_ts,
            has_more,
            oldest_id,
            total,
            scope_session_id: log.scope_session_id().map(|s| s.to_string()),
        })
    }

    /// #37 rollback-by-ID：找到 `message_id` 在 `messages` 里的位置，调 `rollback_to(index)`。
    ///
    /// ID 不存在 → `BadRequest`。ID 寻址仍走 `with_session` 串行化，与并发 append 不产生半态。
    /// 同 `rollback`，返回 `(ChatLog, dropped_count)`。
    pub fn rollback_to_id(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message_id: &str,
    ) -> Result<(ChatLog, usize), AirpError> {
        if !ulid::is_valid_id(message_id) {
            return Err(AirpError::BadRequest(format!(
                "message_id is not a valid durable message id: {message_id}"
            )));
        }
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total = log.messages.len();
            if total == 0 {
                return Err(AirpError::BadRequest(format!(
                    "message_id {message_id} not in this empty session"
                )));
            }
            let idx = log
                .message_ids
                .iter()
                .position(|x| ulid::matches(x, message_id))
                .ok_or_else(|| {
                    AirpError::BadRequest(format!("message_id {message_id} not in this session"))
                })?;
            let dropped = total - idx - 1;
            log.rollback_to(&self.data_root, idx)?;
            Ok((log, dropped))
        })
    }

    pub fn recent(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, AirpError> {
        self.history(character_id, session_id)
            .map(|log| log.recent(limit))
    }

    pub fn append(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message: ChatMessage,
    ) -> Result<(ChatLog, usize), AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total_before = log.messages.len();
            log.append(&self.data_root, message)?;
            Ok((log, total_before))
        })
    }

    pub fn rollback(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        index: usize,
    ) -> Result<(ChatLog, usize), AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total = log.messages.len();
            if total == 0 && index == 0 {
                return Ok((log, 0));
            }
            if index >= total {
                return Err(AirpError::BadRequest(format!(
                    "index {index} out of range (total {total})"
                )));
            }
            let dropped = total - index - 1;
            log.rollback_to(&self.data_root, index)?;
            Ok((log, dropped))
        })
    }

    pub fn rollback_preview(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        index: usize,
    ) -> Result<usize, AirpError> {
        self.with_session(character_id, session_id, || {
            let log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total = log.messages.len();
            if total == 0 && index == 0 {
                return Ok(0);
            }
            if index >= total {
                return Err(AirpError::BadRequest(format!(
                    "index {index} out of range (total {total})"
                )));
            }
            Ok(total - index - 1)
        })
    }

    pub fn regen(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            if !log.messages.is_empty() {
                log.delete_last_n(&self.data_root, 1)?;
            }
            Ok(log)
        })
    }

    pub fn list_sessions(&self, character_id: &CharacterId) -> Result<Vec<SessionId>, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().expect("character lock poisoned");
        data_dir::list_sessions(&self.data_root, character_id.as_str())
    }

    pub fn create_session(&self, character_id: &CharacterId) -> Result<SessionId, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().expect("character lock poisoned");
        data_dir::create_session(&self.data_root, character_id.as_str())
    }

    pub fn delete_character(&self, character_id: &CharacterId) -> Result<(), AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.write().expect("character lock poisoned");
        data_dir::delete_character(&self.data_root, character_id)
    }

    /// #35：删除一个命名会话目录。走 character read lock + session lock，与 append/
    /// rollback/regen 同边界串行化，避免并发写期间删到半态。
    ///
    /// 会话不存在 → `NotFound`。destructive：调用方负责确认。
    pub fn delete_session(
        &self,
        character_id: &CharacterId,
        session_id: &SessionId,
    ) -> Result<(), AirpError> {
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().expect("character lock poisoned");
        let session = session_lock(character_id.as_str(), Some(session_id));
        let _session_guard = session.lock().expect("session lock poisoned");
        // A previous attempt may have written the fail-closed tombstone but
        // failed to remove the directory. Deletion must bypass `with_session`'s
        // tombstone rejection so a retry can finish that cleanup.
        let result = data_dir::delete_session(&self.data_root, character_id.as_str(), session_id);
        if result.is_ok() {
            remove_deleted_session_lock(character_id.as_str(), session_id);
        }
        result
    }
}

#[derive(Clone, Debug)]
pub struct StateService {
    data_root: PathBuf,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct StateSnapshot {
    pub revision: u64,
    pub timestamp: String,
    pub state: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct LorebookService {
    data_root: PathBuf,
}

impl LorebookService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    pub fn read(
        &self,
        character_id: &CharacterId,
    ) -> Result<crate::orchestrator::Lorebook, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().expect("character lock poisoned");
        let resource = state_lock(character_id.as_str());
        let _resource_guard = resource.lock().expect("resource lock poisoned");
        let path = data_dir::char_world_lorebook_path(&self.data_root, character_id.as_str());
        if !path.exists() {
            return Err(AirpError::NotFound(format!(
                "lorebook for character {character_id} not found"
            )));
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    pub fn write(
        &self,
        character_id: &CharacterId,
        lorebook: &crate::orchestrator::Lorebook,
    ) -> Result<(), AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().expect("character lock poisoned");
        let resource = state_lock(character_id.as_str());
        let _resource_guard = resource.lock().expect("resource lock poisoned");
        data_dir::char_world_dir(&self.data_root, character_id.as_str())?;
        let path = data_dir::char_world_lorebook_path(&self.data_root, character_id.as_str());
        data_dir::replace_file(&path, &serde_json::to_vec_pretty(lorebook)?)
    }
}

impl StateService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    pub fn write(
        &self,
        character_id: &CharacterId,
        state: &serde_json::Value,
    ) -> Result<StateSnapshot, AirpError> {
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().expect("character lock poisoned");
        let state_boundary = state_lock(character_id.as_str());
        let _state_guard = state_boundary.lock().expect("state lock poisoned");

        let state_dir = data_dir::char_state_dir(&self.data_root, character_id.as_str());
        fs::create_dir_all(&state_dir)?;
        let schema_path = state_dir.join("schema.json");
        if schema_path.exists() {
            let schema: serde_json::Value = serde_json::from_slice(&fs::read(&schema_path)?)?;
            validate_state(&schema, state)?;
        }

        let history_path =
            data_dir::char_state_history_path(&self.data_root, character_id.as_str());
        let revision = latest_revision(&history_path)? + 1;
        let snapshot = StateSnapshot {
            revision,
            timestamp: chrono::Utc::now().to_rfc3339(),
            state: state.clone(),
        };

        data_dir::replace_file(
            &state_dir.join("live.json"),
            &serde_json::to_vec_pretty(state)?,
        )?;
        let mut history = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(history_path)?;
        serde_json::to_writer(&mut history, &snapshot)?;
        history.write_all(b"\n")?;
        history.sync_data()?;
        Ok(snapshot)
    }
}

fn latest_revision(path: &Path) -> Result<u64, AirpError> {
    if !path.exists() {
        return Ok(0);
    }
    let mut file = fs::File::open(path)?;
    let mut position = file.metadata()?.len();
    let mut suffix = Vec::new();
    while position > 0 {
        let start = position.saturating_sub(4096);
        let mut block = vec![0; (position - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut block)?;
        block.extend_from_slice(&suffix);
        let first_newline = block.iter().position(|byte| *byte == b'\n');
        let complete_lines = first_newline.map_or(&[][..], |index| &block[index + 1..]);
        if let Some(revision) = complete_lines
            .split(|byte| *byte == b'\n')
            .rev()
            .filter(|line| !line.is_empty())
            .find_map(|line| serde_json::from_slice::<StateSnapshot>(line).ok())
            .map(|entry| entry.revision)
        {
            return Ok(revision);
        }
        suffix = match first_newline {
            Some(index) => block[..index].to_vec(),
            None => block,
        };
        position = start;
    }
    Ok(serde_json::from_slice::<StateSnapshot>(&suffix).map_or(0, |entry| entry.revision))
}

fn validate_state(schema: &serde_json::Value, state: &serde_json::Value) -> Result<(), AirpError> {
    if let Some(fields) = schema.get("fields").and_then(serde_json::Value::as_array) {
        let object = state
            .as_object()
            .ok_or_else(|| AirpError::BadRequest("state schema requires an object".to_string()))?;
        for field in fields {
            let Some(key) = field.get("key").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let value = object.get(key);
            if field.get("required").and_then(serde_json::Value::as_bool) == Some(true)
                && value.is_none()
            {
                return Err(AirpError::BadRequest(format!(
                    "state schema: missing required field {key}"
                )));
            }
            if let Some(value) = value {
                validate_schema_value(field, value, key)?;
            }
        }
        return Ok(());
    }
    validate_schema_value(schema, state, "$")
}

fn validate_schema_value(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), AirpError> {
    if let Some(expected) = schema.get("type").and_then(serde_json::Value::as_str) {
        let valid = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        if !valid {
            return Err(AirpError::BadRequest(format!(
                "state schema: {path} must be {expected}"
            )));
        }
    }

    let minimum = schema.get("minimum").or_else(|| schema.get("min"));
    let maximum = schema.get("maximum").or_else(|| schema.get("max"));
    if let (Some(number), Some(minimum)) = (value.as_f64(), minimum.and_then(|v| v.as_f64())) {
        if number < minimum {
            return Err(AirpError::BadRequest(format!(
                "state schema: {path} is below minimum {minimum}"
            )));
        }
    }
    if let (Some(number), Some(maximum)) = (value.as_f64(), maximum.and_then(|v| v.as_f64())) {
        if number > maximum {
            return Err(AirpError::BadRequest(format!(
                "state schema: {path} exceeds maximum {maximum}"
            )));
        }
    }

    if let Some(allowed) = schema.get("enum").and_then(serde_json::Value::as_array) {
        if !allowed.contains(value) {
            return Err(AirpError::BadRequest(format!(
                "state schema: {path} is not an allowed value"
            )));
        }
    }

    if let Some(object) = value.as_object() {
        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str);
        for key in required {
            if !object.contains_key(key) {
                return Err(AirpError::BadRequest(format!(
                    "state schema: {path}.{key} is required"
                )));
            }
        }
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object);
        if let Some(properties) = properties {
            for (key, property_schema) in properties {
                if let Some(property) = object.get(key) {
                    validate_schema_value(property_schema, property, &format!("{path}.{key}"))?;
                }
            }
        }
        if schema
            .get("additionalProperties")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            if let Some(extra) = object
                .keys()
                .find(|key| properties.is_none_or(|properties| !properties.contains_key(*key)))
            {
                return Err(AirpError::BadRequest(format!(
                    "state schema: unexpected field {path}.{extra}"
                )));
            }
        }
    }
    Ok(())
}

// ── PersonaService（#114，每个用户一个默认 Persona）────────────────────────────
//
// WEBUI-MVP-PLAN §3.1：先只实现"每用户一个默认 Persona"，最小字段 name / description
// / variables / revision。写入走 PersonaService（串行化 persona lock + 原子替换 +
// revision bump + history.jsonl），与 ChatService / StateService 同边界。
//
// persona.json 是元设定（不可变 base），state/live.json 是变量漂移覆盖（MVP 不做），
// state/history.jsonl 是 timeline（MVP 不做）。本 service 当前只管 persona.json 的
// 读/写/revision——多 Persona、头像、角色/会话绑定、drift/history/rollback 全留 #114
// 后续阶段。

/// 持久化的 Persona（每用户一份，#114 MVP；#115 扩多份与绑定）。
///
/// 历史只有一个默认 Persona（`users/{uid}/persona.json`）。#115 起支持每用户多份
/// Persona（`users/{uid}/personas/{pid}.json`），原默认那份迁移到 `personas/default.json`
/// 并保留兼容兜底（无多份时 `get_default` 仍读旧路径）。`bindings` 记录该 Persona 绑定
/// 的角色/会话，让 UI 在选角色时自动激活对应 Persona。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Persona {
    /// Persona schema 版本；当前固定 `2`（#115 加 `id` / `bindings`），未来字段迁移用。
    pub schema: u32,
    /// 递增 revision；PUT 携带 expected_revision 校验，冲突返回 `AirpError::BadRequest`。
    pub revision: u64,
    /// 上次写入的 RFC3339 时间戳，便于 UI 显示"已保存"。
    pub updated_at: String,
    /// 用户显示名（对应 `{{user}}` 占位符）。
    pub name: String,
    /// 自由描述，参与 prompt 装配（MVP 不做模板插值，原样透给 orchestrator）。
    pub description: String,
    /// 自定义变量表，键名对应 prompt 中 `{{key}}` 占位符。
    pub variables: HashMap<String, String>,
    /// #115：Persona 自己的 ID（多份 Persona 寿名）；schema=1 时默认 `"default"`。
    /// serde `default` 让旧 persona.json（无此字段）反序列化不破。
    #[serde(default = "Persona::default_id")]
    pub id: String,
    /// #115：该 Persona 绑定的角色/会话列表；UI 选角色时自动激活匹配的 Persona。
    /// 元素 `{character_id, session_id?}`；session_id 缺省表示全会话通用。
    #[serde(default)]
    pub bindings: Vec<PersonaBinding>,
}

impl Persona {
    /// 当前 schema 版本。#115 升到 2（加 `id` / `bindings`）；旧 schema=1 自动迁移。
    pub const SCHEMA: u32 = 2;
    /// schema=1 兼容默认 id。
    fn default_id() -> String {
        "default".to_string()
    }

    /// 构造一份初始 Persona（revision=0，name=default，id=default）。
    pub fn initial(default_name: &str) -> Self {
        Self {
            schema: Self::SCHEMA,
            revision: 0,
            updated_at: chrono::Utc::now().to_rfc3339(),
            name: default_name.to_string(),
            description: String::new(),
            variables: HashMap::new(),
            id: Self::default_id(),
            bindings: Vec::new(),
        }
    }
}

/// #115：Persona 与角色/会话的绑定。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersonaBinding {
    pub character_id: String,
    /// `None` = 该角色下所有会话通用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Persona 原子写入时的冲突 payload：返回当前服务端 revision，让客户端 merge 后重试。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersonaRevisionConflict {
    pub current_revision: u64,
}

/// User Persona shared service（读 / 原子写 / revision 校验 / 多份 / 绑定）。
///
/// 与 `ChatService` / `StateService` 同构：`data_root` 持一份，`new()` 廉价；
/// 写入走 `persona_lock` 串行化 + `replace_file` 原子替换 + history.jsonl append。
///
/// #115 起支持每用户多份 Persona（`users/{uid}/personas/{pid}.json`），原默认那份
/// (`persona.json`) 保留兜底：`get_default` / `save_default` 维护兼容路径，
/// `list` / `get` / `save` / `delete` 操作多份集合。
#[derive(Clone, Debug)]
pub struct PersonaService {
    data_root: PathBuf,
}

impl PersonaService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    // ── 默认 Persona（兼容老路径）────────────────────────────────────────────

    /// 读取当前默认 Persona；不存在时返回 `Persona::initial(default_name)` 的拷贝（不写盘）。
    ///
    /// `default_name` 仅用于未初始化时的 UI 显示兜底；调用方应随后 `save_default` 持久化。
    pub fn get_default(&self, user_id: &UserId, default_name: &str) -> Result<Persona, AirpError> {
        self.get(user_id, "default", default_name)
    }

    /// 原子写入默认 Persona；`expected_revision` 不匹配当前服务端 revision 时返回
    /// `AirpError::BadRequest`，message 携带 `PersonaRevisionConflict` JSON，
    /// 让 UI 解析出 `current_revision` 后 merge 重试（而非裸 409 文本）。
    pub fn save_default(
        &self,
        user_id: &UserId,
        expected_revision: u64,
        persona: Persona,
    ) -> Result<Persona, AirpError> {
        self.save(user_id, "default", expected_revision, persona)
    }

    // ── 多份 Persona（#115）────────────────────────────────────────────────────

    /// 列出该用户的所有 Persona id（含 `default`）。无多份目录时返回 `["default"]`。
    pub fn list(&self, user_id: &UserId) -> Result<Vec<String>, AirpError> {
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().expect("persona lock poisoned");
        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        let mut ids: Vec<String> = Vec::new();
        if dir.exists() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(stem) = name.strip_suffix(".json") {
                    if data_dir::validate_id_segment(stem).is_ok() {
                        ids.push(stem.to_string());
                    }
                }
            }
        }
        // 兜底：若多份目录不在但旧 persona.json 在，补 `default`。
        if !ids.iter().any(|i| i == "default")
            && data_dir::user_persona_path(&self.data_root, user_id).exists()
        {
            ids.push("default".to_string());
        }
        if ids.is_empty() {
            ids.push("default".to_string());
        }
        ids.sort();
        Ok(ids)
    }

    /// 读取指定 id 的 Persona；不存在时返回 `Persona::initial(default_name)` 并设 `id`。
    pub fn get(
        &self,
        user_id: &UserId,
        persona_id: &str,
        default_name: &str,
    ) -> Result<Persona, AirpError> {
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().expect("persona lock poisoned");
        let path = data_dir::user_persona_multi_path(&self.data_root, user_id, persona_id)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // 兜底：`default` 也读旧路径。
                if persona_id == "default" {
                    let legacy = data_dir::user_persona_path(&self.data_root, user_id);
                    match fs::read(&legacy) {
                        Ok(b) => b,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            let mut p = Persona::initial(default_name);
                            p.id = persona_id.to_string();
                            return Ok(p);
                        }
                        Err(e) => return Err(e.into()),
                    }
                } else {
                    let mut p = Persona::initial(default_name);
                    p.id = persona_id.to_string();
                    return Ok(p);
                }
            }
            Err(error) => return Err(error.into()),
        };
        let mut persona = self.parse_persona_bytes(&bytes)?;
        persona.id = persona_id.to_string();
        Ok(persona)
    }

    /// 原子写入指定 id 的 Persona（多份）；`expected_revision` 校验同 `save_default`。
    /// 写入到 `users/{uid}/personas/{pid}.json`；若 pid == "default" 同时回写兼容老路径。
    pub fn save(
        &self,
        user_id: &UserId,
        persona_id: &str,
        expected_revision: u64,
        mut persona: Persona,
    ) -> Result<Persona, AirpError> {
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().expect("persona lock poisoned");
        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        fs::create_dir_all(&dir)?;
        let path = data_dir::user_persona_multi_path(&self.data_root, user_id, persona_id)?;

        let current_revision = if persona_id == "default" && !path.exists() {
            self.current_revision_at(&data_dir::user_persona_path(&self.data_root, user_id))?
        } else {
            self.current_revision_at(&path)?
        };
        if expected_revision != current_revision {
            let conflict = PersonaRevisionConflict { current_revision };
            return Err(AirpError::BadRequest(serde_json::to_string(&conflict)?));
        }

        Self::validate_bindings(&persona.bindings)?;
        persona.schema = Persona::SCHEMA;
        persona.id = persona_id.to_string();
        persona.revision = current_revision + 1;
        persona.updated_at = chrono::Utc::now().to_rfc3339();
        data_dir::replace_file(&path, &serde_json::to_vec_pretty(&persona)?)?;

        // `default` 同步回写兼容老路径，避免旧读链断裂。
        if persona_id == "default" {
            let legacy = data_dir::user_persona_path(&self.data_root, user_id);
            data_dir::replace_file(&legacy, &serde_json::to_vec_pretty(&persona)?)?;
        }
        Ok(persona)
    }

    /// 删除指定 id 的 Persona；`default` 不允许删（返 BadRequest）。删除文件不可逆。
    pub fn delete(&self, user_id: &UserId, persona_id: &str) -> Result<(), AirpError> {
        if persona_id == "default" {
            return Err(AirpError::BadRequest(
                "default persona 不可删除；可用 save 重置内容".to_string(),
            ));
        }
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().expect("persona lock poisoned");
        let path = data_dir::user_persona_multi_path(&self.data_root, user_id, persona_id)?;
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    // ── 绑定（#115）────────────────────────────────────────────────────────────

    /// 给 Persona 加一条绑定；幂等（同 character_id+session_id 不重复追加）。
    pub fn bind(
        &self,
        user_id: &UserId,
        persona_id: &str,
        binding: PersonaBinding,
    ) -> Result<Persona, AirpError> {
        CharacterId::new(&binding.character_id)?;
        if let Some(session_id) = &binding.session_id {
            SessionId::parse(session_id)?;
        }
        let mut persona = self.get(user_id, persona_id, "User")?;
        if persona
            .bindings
            .iter()
            .any(|b| b.character_id == binding.character_id && b.session_id == binding.session_id)
        {
            return Ok(persona);
        }
        persona.bindings.push(binding);
        let rev = persona.revision;
        self.save(user_id, persona_id, rev, persona)
    }

    /// 移除一条绑定；幂等。返回更新后的 Persona。
    pub fn unbind(
        &self,
        user_id: &UserId,
        persona_id: &str,
        character_id: &str,
        session_id: Option<&str>,
    ) -> Result<Persona, AirpError> {
        CharacterId::new(character_id)?;
        if let Some(session_id) = session_id {
            SessionId::parse(session_id)?;
        }
        let mut persona = self.get(user_id, persona_id, "User")?;
        let previous_len = persona.bindings.len();
        persona
            .bindings
            .retain(|b| !(b.character_id == character_id && b.session_id.as_deref() == session_id));
        if persona.bindings.len() == previous_len {
            return Ok(persona);
        }
        let rev = persona.revision;
        self.save(user_id, persona_id, rev, persona)
    }

    /// 查找该用户下绑定到指定角色/会话的 Persona id（首个匹配）。
    /// 优先匹配带 session_id 的精确绑定，再匹配全会话通用绑定。
    pub fn find_for_character(
        &self,
        user_id: &UserId,
        character_id: &str,
        session_id: Option<&str>,
    ) -> Result<Option<String>, AirpError> {
        CharacterId::new(character_id)?;
        if let Some(session_id) = session_id {
            SessionId::parse(session_id)?;
        }
        let ids = self.list(user_id)?;
        // 精确 session 绑定优先。
        let mut generic: Option<String> = None;
        for pid in &ids {
            let persona = self.get(user_id, pid, "User")?;
            for b in &persona.bindings {
                if b.character_id != character_id {
                    continue;
                }
                match (&b.session_id, session_id) {
                    (Some(b_sid), Some(q_sid)) if b_sid == q_sid => return Ok(Some(pid.clone())),
                    (None, _) => {
                        generic = Some(pid.clone());
                    }
                    _ => {}
                }
            }
        }
        Ok(generic)
    }

    // ── 内部────────────────────────────────────────────────────────────────────

    fn parse_persona_bytes(&self, bytes: &[u8]) -> Result<Persona, AirpError> {
        let mut persona: Persona = serde_json::from_slice(bytes)?;
        // schema=1（无 id/bindings）靠 serde default 升到 2；若 schema>2 拒。
        if persona.schema > Persona::SCHEMA {
            return Err(AirpError::Internal(format!(
                "persona schema {} unsupported (expected <= {})",
                persona.schema,
                Persona::SCHEMA
            )));
        }
        if persona.schema < Persona::SCHEMA {
            persona.schema = Persona::SCHEMA;
        }
        Ok(persona)
    }

    fn current_revision_at(&self, path: &Path) -> Result<u64, AirpError> {
        if !path.exists() {
            return Ok(0);
        }
        let bytes = fs::read(path)?;
        Ok(self.parse_persona_bytes(&bytes)?.revision)
    }

    fn validate_bindings(bindings: &[PersonaBinding]) -> Result<(), AirpError> {
        let mut seen = std::collections::HashSet::new();
        for binding in bindings {
            CharacterId::new(&binding.character_id)?;
            if let Some(session_id) = &binding.session_id {
                SessionId::parse(session_id)?;
            }
            if !seen.insert((&binding.character_id, &binding.session_id)) {
                return Err(AirpError::BadRequest(
                    "duplicate persona binding".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::MessageRole;

    #[test]
    fn append_and_rollback_share_one_session_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();

        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "one".into(),
                },
            )
            .unwrap();
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::Assistant,
                    content: "two".into(),
                },
            )
            .unwrap();

        let (log, dropped) = service.rollback(&character, None, 0).unwrap();
        assert_eq!(dropped, 1);
        assert_eq!(log.messages.len(), 1);
    }

    #[test]
    fn concurrent_appends_do_not_lose_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let service = Arc::new(ChatService::new(tmp.path()));
        let character = CharacterId::new("concurrent").unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let mut workers = Vec::new();

        for index in 0..8 {
            let service = service.clone();
            let character = character.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                service
                    .append(
                        &character,
                        None,
                        ChatMessage {
                            role: MessageRole::User,
                            content: format!("message-{index}"),
                        },
                    )
                    .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let log = service.history(&character, None).unwrap();
        assert_eq!(log.messages.len(), 8);
        let unique: std::collections::HashSet<_> = log
            .messages
            .iter()
            .map(|message| &message.content)
            .collect();
        assert_eq!(unique.len(), 8);
    }

    #[test]
    fn state_service_validates_schema_and_assigns_revisions() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("stateful").unwrap();
        let state_dir = data_dir::char_state_dir(tmp.path(), character.as_str());
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(
            state_dir.join("schema.json"),
            serde_json::to_vec(&serde_json::json!({
                "type": "object",
                "required": ["hp"],
                "additionalProperties": false,
                "properties": {"hp": {"type": "integer", "minimum": 0, "maximum": 100}}
            }))
            .unwrap(),
        )
        .unwrap();
        let service = StateService::new(tmp.path());

        let first = service
            .write(&character, &serde_json::json!({"hp": 80}))
            .unwrap();
        let second = service
            .write(&character, &serde_json::json!({"hp": 60}))
            .unwrap();
        assert_eq!((first.revision, second.revision), (1, 2));
        assert!(service
            .write(&character, &serde_json::json!({"hp": 101}))
            .is_err());
        let live: serde_json::Value =
            serde_json::from_slice(&fs::read(state_dir.join("live.json")).unwrap()).unwrap();
        assert_eq!(live["hp"], 60);
    }

    #[test]
    fn state_schema_without_properties_rejects_all_additional_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("closed").unwrap();
        let state_dir = data_dir::char_state_dir(tmp.path(), character.as_str());
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(
            state_dir.join("schema.json"),
            serde_json::to_vec(&serde_json::json!({
                "type": "object",
                "additionalProperties": false
            }))
            .unwrap(),
        )
        .unwrap();

        let error = StateService::new(tmp.path())
            .write(&character, &serde_json::json!({"unexpected": true}))
            .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
    }

    #[test]
    fn latest_revision_skips_a_large_invalid_trailing_line() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("history.jsonl");
        let snapshot = StateSnapshot {
            revision: 7,
            timestamp: "2026-07-10T00:00:00Z".to_string(),
            state: serde_json::json!({"hp": 50}),
        };
        let mut bytes = serde_json::to_vec(&snapshot).unwrap();
        bytes.push(b'\n');
        bytes.extend(std::iter::repeat_n(b'x', 12_000));
        fs::write(&history, bytes).unwrap();

        assert_eq!(super::latest_revision(&history).unwrap(), 7);
    }

    // ── PersonaService（#114）─────────────────────────────────────────────────────

    #[test]
    fn persona_get_returns_initial_when_not_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let persona = service.get_default(&uid, "User").unwrap();
        assert_eq!(
            persona.revision, 0,
            "non-existent persona returns revision 0"
        );
        assert_eq!(persona.name, "User", "default name fallback");
        assert!(persona.variables.is_empty());
        // 不写盘：persona.json 不应存在
        assert!(!crate::data_dir::user_persona_path(tmp.path(), &uid).exists());
    }

    #[test]
    fn persona_save_bumps_revision_and_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        let persona = Persona {
            schema: Persona::SCHEMA,
            revision: 0, // save 内 bump
            updated_at: String::new(),
            name: "Alice".to_string(),
            description: "a curious librarian".to_string(),
            variables: HashMap::from([("mood".to_string(), "curious".to_string())]),
            id: "default".to_string(),
            bindings: Vec::new(),
        };
        let saved = service.save_default(&uid, 0, persona).unwrap();
        assert_eq!(saved.revision, 1, "first save bumps 0 -> 1");
        assert_eq!(saved.name, "Alice");
        assert_eq!(saved.variables.get("mood").unwrap(), "curious");

        // 持久化：重新 get 应读回同一份
        let reread = service.get_default(&uid, "User").unwrap();
        assert_eq!(reread.revision, 1);
        assert_eq!(reread.name, "Alice");
        assert_eq!(reread.description, "a curious librarian");
    }

    #[test]
    fn persona_save_rejects_revision_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        let p1 = Persona::initial("Alice");
        service.save_default(&uid, 0, p1).unwrap(); // revision -> 1

        // 客户端仍持有 revision=0，服务端已 1 → 必须拒绝
        let p2 = Persona::initial("Alice-updated");
        let err = service.save_default(&uid, 0, p2).unwrap_err();
        let conflict: PersonaRevisionConflict = serde_json::from_str(match &err {
            AirpError::BadRequest(s) => s,
            _ => panic!("expected BadRequest with PersonaRevisionConflict JSON, got {err:?}"),
        })
        .unwrap();
        assert_eq!(
            conflict.current_revision, 1,
            "conflict payload must report server-side revision"
        );
    }

    #[test]
    fn persona_save_rejects_unsupported_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        // 手动写一份 schema=999 的 persona.json
        let dir = crate::data_dir::user_dir(tmp.path(), &uid);
        fs::create_dir_all(&dir).unwrap();
        let bad = serde_json::json!({
            "schema": 999,
            "revision": 5,
            "updated_at": "2026-07-11T00:00:00Z",
            "name": "bad",
            "description": "",
            "variables": {}
        });
        fs::write(
            crate::data_dir::user_persona_path(tmp.path(), &uid),
            serde_json::to_vec_pretty(&bad).unwrap(),
        )
        .unwrap();

        let err = service.get_default(&uid, "User").unwrap_err();
        assert!(
            matches!(err, AirpError::Internal(_)),
            "unsupported schema must be Internal, got {err:?}"
        );
    }

    #[test]
    fn persona_save_does_not_overwrite_corrupt_existing_data() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let path = crate::data_dir::user_persona_multi_path(tmp.path(), &uid, "default").unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not-json").unwrap();

        assert!(service
            .save_default(&uid, 0, Persona::initial("Alice"))
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not-json");
    }

    #[test]
    fn persona_multi_storage_rejects_traversal_and_preserves_legacy_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        let legacy = service
            .save_default(&uid, 0, Persona::initial("Legacy"))
            .unwrap();
        assert_eq!(legacy.revision, 1);

        let migrated = service.save(&uid, "default", 1, legacy.clone()).unwrap();
        assert_eq!(migrated.revision, 2);
        assert_eq!(service.get_default(&uid, "User").unwrap().revision, 2);
        assert!(service.get(&uid, "../escape", "User").is_err());
        assert!(service
            .save(&uid, "..\\escape", 0, Persona::initial("Bad"))
            .is_err());
    }

    #[test]
    fn persona_binding_prefers_session_and_idempotent_bind_does_not_bump_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let session = SessionId::new().to_string();

        service
            .save(&uid, "generic", 0, Persona::initial("Generic"))
            .unwrap();
        service
            .save(&uid, "specific", 0, Persona::initial("Specific"))
            .unwrap();
        let generic = service
            .bind(
                &uid,
                "generic",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: None,
                },
            )
            .unwrap();
        let unchanged = service
            .bind(
                &uid,
                "generic",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: None,
                },
            )
            .unwrap();
        assert_eq!(unchanged.revision, generic.revision);

        service
            .bind(
                &uid,
                "specific",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: Some(session.clone()),
                },
            )
            .unwrap();
        assert_eq!(
            service
                .find_for_character(&uid, "char-a", Some(&session))
                .unwrap(),
            Some("specific".to_string())
        );
        assert_eq!(
            service.find_for_character(&uid, "char-a", None).unwrap(),
            Some("generic".to_string())
        );
    }

    // ── delete_session + session-scoped lifecycle（#35/#37）──────────────────────

    #[test]
    fn delete_session_removes_directory_and_is_not_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let sid = service.create_session(&character).unwrap();

        // append 一条消息到命名会话，确认目录非空
        service
            .append(
                &character,
                Some(&sid),
                ChatMessage {
                    role: MessageRole::User,
                    content: "hi".to_string(),
                },
            )
            .unwrap();
        let sessions_dir = tmp
            .path()
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(sid.to_string());
        assert!(
            sessions_dir.is_dir(),
            "session dir must exist before delete"
        );

        service.delete_session(&character, &sid).unwrap();
        assert!(
            !sessions_dir.exists(),
            "session dir must be gone after delete"
        );
        let listed = service.list_sessions(&character).unwrap();
        assert!(
            !listed.contains(&sid),
            "deleted session must not appear in list_sessions"
        );
    }

    #[test]
    fn delete_session_returns_not_found_for_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let unknown = SessionId::new();
        let err = service.delete_session(&character, &unknown).unwrap_err();
        assert!(
            matches!(err, AirpError::NotFound(_)),
            "unknown session delete must be NotFound, got {err:?}"
        );
    }

    #[test]
    fn delete_session_retries_cleanup_after_tombstone_was_written() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let sid = service.create_session(&character).unwrap();
        let marker = tmp
            .path()
            .join("characters/alice/deleted_sessions")
            .join(sid.to_string());
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, []).unwrap();

        service.delete_session(&character, &sid).unwrap();

        assert!(marker.is_file());
        assert!(!tmp
            .path()
            .join("characters/alice/sessions")
            .join(sid.to_string())
            .exists());
    }

    /// #35/#37：命名会话与默认会话隔离——append 到命名会话不污染默认会话 history，
    /// 删除命名会话不影响默认会话。这是 WEBUI-MVP-PLAN §3.2"切换后不串流、串历史"
    /// 的最小可自动验收子集。
    #[test]
    fn named_session_isolated_from_default_and_delete_does_not_leak() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();

        // default session：2 条
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "default-1".to_string(),
                },
            )
            .unwrap();
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "default-2".to_string(),
                },
            )
            .unwrap();

        // named session A：3 条
        let sid_a = service.create_session(&character).unwrap();
        for content in ["a-1", "a-2", "a-3"] {
            service
                .append(
                    &character,
                    Some(&sid_a),
                    ChatMessage {
                        role: MessageRole::User,
                        content: content.to_string(),
                    },
                )
                .unwrap();
        }

        // 隔离断言：default history 不含 named 的消息
        let default_log = service.history(&character, None).unwrap();
        assert_eq!(
            default_log.messages.len(),
            2,
            "default session must keep its own 2 messages"
        );
        assert!(
            default_log
                .messages
                .iter()
                .all(|m| m.content.starts_with("default-")),
            "default session must not leak named session messages"
        );

        let named_log = service.history(&character, Some(&sid_a)).unwrap();
        assert_eq!(
            named_log.messages.len(),
            3,
            "named session A must keep its own 3 messages"
        );
        assert!(
            named_log
                .messages
                .iter()
                .all(|m| m.content.starts_with("a-")),
            "named session A must not leak default session messages"
        );

        // delete named A → default 不受影响
        service.delete_session(&character, &sid_a).unwrap();
        let default_log_after = service.history(&character, None).unwrap();
        assert_eq!(
            default_log_after.messages.len(),
            2,
            "default session must survive named session delete"
        );
        assert!(
            !service.list_sessions(&character).unwrap().contains(&sid_a),
            "deleted named session A must not appear in list_sessions"
        );
    }

    /// #35：delete_session 与 append 同时起跑。共享 session lock 必须保证每个 append
    /// 要么完整落盘，要么在 delete 的 tombstone 后返回 NotFound，不能半写或复活目录。
    #[test]
    fn delete_session_serializes_with_concurrent_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let service = Arc::new(ChatService::new(tmp.path()));
        let character = CharacterId::new("concurrent").unwrap();
        let sid = service.create_session(&character).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let mut workers = Vec::new();

        for index in 0..8 {
            let service = service.clone();
            let character = character.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                service.append(
                    &character,
                    Some(&sid),
                    ChatMessage {
                        role: MessageRole::User,
                        content: format!("message-{index}"),
                    },
                )
            }));
        }
        let delete_service = service.clone();
        let delete_character = character.clone();
        let delete_barrier = barrier.clone();
        let delete_worker = std::thread::spawn(move || {
            delete_barrier.wait();
            delete_service.delete_session(&delete_character, &sid)
        });
        for worker in workers {
            let result = worker.join().unwrap();
            assert!(
                result.is_ok() || matches!(result, Err(AirpError::NotFound(_))),
                "append racing delete must either commit or return NotFound, got {result:?}"
            );
        }
        delete_worker.join().unwrap().unwrap();
        assert!(
            !service.list_sessions(&character).unwrap().contains(&sid),
            "deleted concurrent session must not appear in list_sessions"
        );
        // delete 后再 append 到同一命名会话 → NotFound（目录被删，load_or_create 不复活命名会话）
        let err = service
            .append(
                &character,
                Some(&sid),
                ChatMessage {
                    role: MessageRole::User,
                    content: "post-delete".to_string(),
                },
            )
            .unwrap_err();
        assert!(
            matches!(err, AirpError::NotFound(_)),
            "append to deleted named session must be NotFound, got {err:?}"
        );
    }

    #[test]
    fn deleting_unknown_session_does_not_create_character() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("missing-character").unwrap();
        let sid = SessionId::new();

        let err = service.delete_session(&character, &sid).unwrap_err();
        assert!(matches!(err, AirpError::NotFound(_)));
        assert!(
            !tmp.path().join("characters/missing-character").exists(),
            "a failed delete must not create an empty character"
        );
    }

    // ── #37 durable message-id contract：cursor / rollback-by-ID 不变式 ──────

    fn seed_session_with_n(
        root: &Path,
        cid: &str,
        sid: Option<SessionId>,
        n: usize,
    ) -> (ChatService, CharacterId, Option<SessionId>) {
        let character = CharacterId::new(cid).unwrap();
        let session_id = sid;
        let service = ChatService::new(root);
        for i in 0..n {
            service
                .append(
                    &character,
                    session_id.as_ref(),
                    ChatMessage {
                        role: if i % 2 == 0 {
                            crate::adapter::MessageRole::User
                        } else {
                            crate::adapter::MessageRole::Assistant
                        },
                        content: format!("msg-{i}"),
                    },
                )
                .unwrap();
        }
        (service, character, session_id)
    }

    fn parse_sid(s: &str) -> SessionId {
        // 用固定 UUID 字符串做测试 sid，避免 SessionId::new() 的非确定性。
        SessionId::parse(s).unwrap()
    }

    #[test]
    fn history_window_limit_returns_tail_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "win_char", None, 10);

        // 取最近 4 条 → 应是 msg-6..msg-9，时间正序。
        let win = service
            .history_window(&character, session_id.as_ref(), Some(4), None)
            .unwrap();
        assert_eq!(win.messages.len(), 4);
        assert_eq!(win.messages[0].content, "msg-6");
        assert_eq!(win.messages[3].content, "msg-9");
        assert_eq!(win.total, 10);
        assert!(
            win.has_more,
            "loading tail of 10 with limit 4 must have more"
        );
        assert!(win.oldest_id.is_some());
    }

    #[test]
    fn history_window_before_cursor_returns_strictly_earlier() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "cursor_char", None, 10);

        // 取最近 4 条拿到 oldest_id 当 cursor。
        let tail = service
            .history_window(&character, session_id.as_ref(), Some(4), None)
            .unwrap();
        let cursor = tail.oldest_id.unwrap().to_ascii_lowercase();

        // before=cursor → 返回 cursor 严格之前（更早）的消息，limit 3。
        let earlier = service
            .history_window(&character, session_id.as_ref(), Some(3), Some(&cursor))
            .unwrap();
        assert_eq!(earlier.messages.len(), 3);
        // cursor 是 msg-6，更早 3 条 = msg-3..msg-5。
        assert_eq!(earlier.messages[0].content, "msg-3");
        assert_eq!(earlier.messages[2].content, "msg-5");
        assert!(earlier.has_more, "there are still earlier messages");
    }

    #[test]
    fn cursor_rejects_id_from_other_session() {
        let tmp = tempfile::tempdir().unwrap();
        // session A 拿一个真实 ID。
        let (svc_a, char_a, sess_a) = seed_session_with_n(
            tmp.path(),
            "cross_a",
            Some(parse_sid("550e8400-e29b-41d4-a716-446655440001")),
            3,
        );
        let log_a = svc_a.history(&char_a, sess_a.as_ref()).unwrap();
        let id_a = log_a.message_ids[0].clone();

        // session B 用 A 的 ID 当 cursor → BadRequest（cursor 不能跨 session）。
        let (svc_b, char_b, sess_b) = seed_session_with_n(
            tmp.path(),
            "cross_b",
            Some(parse_sid("550e8400-e29b-41d4-a716-446655440002")),
            3,
        );
        let err = svc_b
            .history_window(&char_b, sess_b.as_ref(), Some(2), Some(&id_a))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref msg) if msg.contains("not in this session")),
            "cross-session cursor must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn cursor_rejects_malformed_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) = seed_session_with_n(tmp.path(), "mal_char", None, 3);
        let err = service
            .history_window(&character, session_id.as_ref(), Some(2), Some("not-a-ulid"))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not a valid durable message id")),
            "malformed cursor must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn rollback_by_id_equivalent_to_by_index() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "rbid_char", None, 5);

        // index 2 的 ID → rollback_to_id(id_at_2) 应等价 rollback(2)：保留 0..=2 = 3 条。
        let log = service.history(&character, session_id.as_ref()).unwrap();
        let id_at_2 = log.message_ids[2].clone();

        let (log_after, dropped) = service
            .rollback_to_id(&character, session_id.as_ref(), &id_at_2)
            .unwrap();
        assert_eq!(dropped, 2, "rollback to index 2 drops 2 (total 5, kept 3)");
        assert_eq!(log_after.messages.len(), 3);
        assert_eq!(log_after.messages[2].content, "msg-2");

        // 不变量 6：同位置等价。
        let log_check = service.history(&character, session_id.as_ref()).unwrap();
        assert_eq!(log_check.messages.len(), 3);
    }

    #[test]
    fn rollback_by_id_rejects_unknown_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "rbid_unknown", None, 3);
        // 合形但不在 session 的 ID（派生一个不命中的）。
        let fake = crate::ulid::derive_legacy_id("some-other-scope", 99);
        let err = service
            .rollback_to_id(&character, session_id.as_ref(), &fake)
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not in this session")),
            "unknown message_id must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn rollback_by_id_rejects_malformed_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) = seed_session_with_n(tmp.path(), "rbid_mal", None, 3);
        let err = service
            .rollback_to_id(&character, session_id.as_ref(), "not-a-ulid")
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not a valid durable message id")),
            "malformed message_id must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn rollback_validation_rejects_both_and_neither() {
        // 不变量 7 的 HTTP 入口校验：RollbackRequest.validate_rollback_target。
        use crate::daemon::RollbackRequest;
        use crate::types::CharacterId;
        let cid = CharacterId::new("vchar").unwrap();

        let both = RollbackRequest {
            character_id: cid.clone(),
            message_index: Some(2),
            message_id: Some("m0abc".to_string()),
            session_id: None,
        };
        assert!(both.validate_rollback_target().is_err());

        let neither = RollbackRequest {
            character_id: cid,
            message_index: None,
            message_id: None,
            session_id: None,
        };
        assert!(neither.validate_rollback_target().is_err());

        let ok_id = RollbackRequest {
            character_id: CharacterId::new("v2").unwrap(),
            message_index: None,
            message_id: Some("m0abc".to_string()),
            session_id: None,
        };
        assert!(ok_id.validate_rollback_target().is_ok());

        let ok_idx = RollbackRequest {
            character_id: CharacterId::new("v3").unwrap(),
            message_index: Some(2),
            message_id: None,
            session_id: None,
        };
        assert!(ok_idx.validate_rollback_target().is_ok());
    }

    #[test]
    fn concurrent_append_and_rollback_no_half_state() {
        // 不变量 7：with_session 串行化 → 并发 append/rollback 不产生半态。
        let tmp = tempfile::tempdir().unwrap();
        let cid = CharacterId::new("conc_char").unwrap();
        let sid = parse_sid("550e8400-e29b-41d4-a716-446655440010");
        let svc = ChatService::new(tmp.path());
        // 先种 5 条。
        for _ in 0..5 {
            svc.append(
                &cid,
                Some(&sid),
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: "seed".to_string(),
                },
            )
            .unwrap();
        }

        let svc_arc = std::sync::Arc::new(svc);
        let mut handles = Vec::new();
        for i in 0..10 {
            let s = svc_arc.clone();
            let cidc = cid.clone();
            let sidc = sid;
            handles.push(std::thread::spawn(move || {
                if i % 2 == 0 {
                    s.append(
                        &cidc,
                        Some(&sidc),
                        ChatMessage {
                            role: crate::adapter::MessageRole::Assistant,
                            content: format!("concurrent-{i}"),
                        },
                    )
                } else {
                    // rollback 到 index 2（保留前 3）。
                    s.rollback(&cidc, Some(&sidc), 2)
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        // 不变量：最终态自洽——messages/ids/timestamps 等长，无半态。
        let final_log = svc_arc.history(&cid, Some(&sid)).unwrap();
        assert_eq!(
            final_log.messages.len(),
            final_log.message_ids.len(),
            "concurrent mutations must keep messages/ids equal length"
        );
        assert_eq!(
            final_log.messages.len(),
            final_log.message_timestamps.len(),
            "concurrent mutations must keep messages/timestamps equal length"
        );
    }
}
