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
use crate::types::{CharacterId, SessionId};

type SessionLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;
type CharacterLockMap = Mutex<HashMap<String, Arc<RwLock<()>>>>;
type StateLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;

static SESSION_LOCKS: OnceLock<SessionLockMap> = OnceLock::new();
static CHARACTER_LOCKS: OnceLock<CharacterLockMap> = OnceLock::new();
static STATE_LOCKS: OnceLock<StateLockMap> = OnceLock::new();

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
}
