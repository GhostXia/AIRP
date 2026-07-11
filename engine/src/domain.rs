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
        replace_file(&path, &serde_json::to_vec_pretty(lorebook)?)
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

        replace_file(
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

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), AirpError> {
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
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

/// 持久化的默认 Persona（每用户一份）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Persona {
    /// Persona schema 版本；当前固定 `1`，未来字段迁移用。
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
}

impl Persona {
    /// 当前 schema 版本。
    pub const SCHEMA: u32 = 1;

    /// 构造一份初始 Persona（revision=0，name=default）。
    pub fn initial(default_name: &str) -> Self {
        Self {
            schema: Self::SCHEMA,
            revision: 0,
            updated_at: chrono::Utc::now().to_rfc3339(),
            name: default_name.to_string(),
            description: String::new(),
            variables: HashMap::new(),
        }
    }
}

/// Persona 原子写入时的冲突 payload：返回当前服务端 revision，让客户端 merge 后重试。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersonaRevisionConflict {
    pub current_revision: u64,
}

/// User Persona shared service（读 / 原子写 / revision 校验）。
///
/// 与 `ChatService` / `StateService` 同构：`data_root` 持一份，`new()` 廉价；
/// 写入走 `persona_lock` 串行化 + `replace_file` 原子替换 + history.jsonl append。
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

    /// 读取当前 Persona；不存在时返回 `Persona::initial(default_name)` 的拷贝（不写盘）。
    ///
    /// `default_name` 仅用于未初始化时的 UI 显示兜底；调用方应随后 `save` 持久化。
    pub fn get(&self, user_id: &UserId, default_name: &str) -> Result<Persona, AirpError> {
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().expect("persona lock poisoned");
        let path = data_dir::user_persona_path(&self.data_root, user_id);
        if !path.exists() {
            return Ok(Persona::initial(default_name));
        }
        let bytes = fs::read(&path)?;
        let persona: Persona = serde_json::from_slice(&bytes)?;
        if persona.schema != Persona::SCHEMA {
            return Err(AirpError::Internal(format!(
                "persona schema {} unsupported (expected {})",
                persona.schema,
                Persona::SCHEMA
            )));
        }
        Ok(persona)
    }

    /// 原子写入 Persona；`expected_revision` 不匹配当前服务端 revision 时返回
    /// `AirpError::BadRequest`，message 携带 `PersonaRevisionConflict` JSON，
    /// 让 UI 解析出 `current_revision` 后 merge 重试（而非裸 409 文本）。
    pub fn save(
        &self,
        user_id: &UserId,
        expected_revision: u64,
        mut persona: Persona,
    ) -> Result<Persona, AirpError> {
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().expect("persona lock poisoned");
        let dir = data_dir::user_dir(&self.data_root, user_id);
        fs::create_dir_all(&dir)?;
        let path = data_dir::user_persona_path(&self.data_root, user_id);

        // revision 校验：current = 读取现存 revision（不存在则 0）。
        let current_revision = if path.exists() {
            serde_json::from_slice::<Persona>(&fs::read(&path)?)
                .map(|p| p.revision)
                .unwrap_or(0)
        } else {
            0
        };
        if expected_revision != current_revision {
            let conflict = PersonaRevisionConflict {
                current_revision,
            };
            return Err(AirpError::BadRequest(serde_json::to_string(&conflict)?));
        }

        persona.schema = Persona::SCHEMA;
        persona.revision = current_revision + 1;
        persona.updated_at = chrono::Utc::now().to_rfc3339();
        replace_file(&path, &serde_json::to_vec_pretty(&persona)?)?;
        Ok(persona)
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
        let persona = service.get(&uid, "User").unwrap();
        assert_eq!(persona.revision, 0, "non-existent persona returns revision 0");
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
        };
        let saved = service.save(&uid, 0, persona).unwrap();
        assert_eq!(saved.revision, 1, "first save bumps 0 -> 1");
        assert_eq!(saved.name, "Alice");
        assert_eq!(saved.variables.get("mood").unwrap(), "curious");

        // 持久化：重新 get 应读回同一份
        let reread = service.get(&uid, "User").unwrap();
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
        service.save(&uid, 0, p1).unwrap(); // revision -> 1

        // 客户端仍持有 revision=0，服务端已 1 → 必须拒绝
        let p2 = Persona::initial("Alice-updated");
        let err = service.save(&uid, 0, p2).unwrap_err();
        let conflict: PersonaRevisionConflict =
            serde_json::from_str(match &err {
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

        let err = service.get(&uid, "User").unwrap_err();
        assert!(matches!(err, AirpError::Internal(_)), "unsupported schema must be Internal, got {err:?}");
    }
}
