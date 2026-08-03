//! Character live-state domain service: read/mutate/write `state/live.json`
//! with schema validation, history.jsonl append, and unified revision contract.
//!
//! Extracted from `domain/mod.rs` (E-P1-1 slice 3). Zero behavior change.
//!
//! Locking contract: `character_lock` (read) + `state_lock` (mutex) held for
//! the entire read-modify-write critical section. See
//! `docs/LOCK-ORDER-CONTRACT.md` §2.2 / §3 R1 / §3 R2.

use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use crate::data_dir;
use crate::error::AirpError;
use crate::revision::atomic::{commit_revision, CommitOptions, StagedRevision};
use crate::revision::manifest::{AssetKind, AssetSource};
use crate::types::CharacterId;

use super::lock_order;
use super::locks::{character_lock, state_lock};

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

impl StateService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    /// Read a character's current live state with proper locking.
    ///
    /// Returns `Value::Object(Default::default())` when `live.json` does not
    /// exist yet (fresh character, no state committed). JSON parse failures
    /// are propagated as `AirpError::Internal` rather than silently swallowed
    /// — a corrupt `live.json` must not be overwritten with an empty object
    /// by a subsequent write.
    pub fn read(&self, character_id: &CharacterId) -> Result<serde_json::Value, AirpError> {
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
        let state_boundary = state_lock(character_id.as_str());
        let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
        let _state_track = lock_order::track_state();

        let state_dir = data_dir::char_state_dir(&self.data_root, character_id.as_str());
        Self::load_live_value(character_id, &state_dir)
    }

    /// Atomically mutate a character's live state under the state lock.
    ///
    /// The closure receives the current state (or an empty object if
    /// `live.json` does not exist yet) and may modify it in place. After the
    /// closure returns `Ok(())`, the new state is validated against
    /// `state/schema.json` (if present), written via `data_dir::replace_file`,
    /// appended to `state/history.jsonl`, and a revision snapshot is
    /// committed under `state/revisions/{revision}/` — exactly matching
    /// [`StateService::write`] semantics. Locking is identical to `write`:
    /// `character_lock` read guard + `state_lock` mutex guard held for the
    /// entire read-modify-write critical section, so concurrent tool calls
    /// (e.g. `update_relationship` + `advance_plot`) cannot lose updates.
    ///
    /// Callers that already hold `character_lock.read()` externally (e.g.
    /// `advance_plot` after #437 fix path 4) must use [`Self::mutate_locked`]
    /// to avoid a re-entrant `RwLock::read` on `character_lock` — std
    /// `RwLock` recursive read breaks exclusivity semantics on Windows
    /// SRWLOCK and may deadlock on some pthread implementations.
    pub fn mutate<F>(
        &self,
        character_id: &CharacterId,
        mutate: F,
    ) -> Result<StateSnapshot, AirpError>
    where
        F: FnOnce(&mut serde_json::Value) -> Result<(), AirpError>,
    {
        // LOCK-ORDER: character.read → state.lock（§2.2）。
        // 若调用方已持 session_lock（如 advance_plot），构成唯一合法 session→state 嵌套（§2.3 / R2）。
        // 合同：docs/LOCK-ORDER-CONTRACT.md §2.2 / §2.3 / §3 R1 / §3 R2 / §4 A1。
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
        self.mutate_locked(character_id, mutate)
    }

    /// Same as [`Self::mutate`] but **does not** acquire `character_lock.read()`.
    ///
    /// Caller must already hold `character_lock.read()` (or `.write()`) for
    /// `character_id` before calling this method — typically as the outermost
    /// R1 gate before acquiring `session_lock`. This variant exists to break
    /// the `StateService::mutate` re-entrant `RwLock::read` cycle that
    /// prevented `advance_plot` from acquiring `character_lock.read()`
    /// externally (PR #436 §2.3 R1 exception; closed by #437 fix path 4).
    ///
    /// Only acquires `state_lock` internally. R2 still applies: if the caller
    /// holds `session_lock`, the nesting direction is `session → state`
    /// (legal, only `advance_plot`).
    ///
    /// LOCK-ORDER: caller-held character.read → [caller-held session?] → state.lock。
    /// 合同：docs/LOCK-ORDER-CONTRACT.md §2.2 / §2.3 / §3 R1 / §3 R2 / §4 A1。
    pub fn mutate_locked<F>(
        &self,
        character_id: &CharacterId,
        mutate: F,
    ) -> Result<StateSnapshot, AirpError>
    where
        F: FnOnce(&mut serde_json::Value) -> Result<(), AirpError>,
    {
        let state_boundary = state_lock(character_id.as_str());
        let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
        let _state_track = lock_order::track_state();

        let state_dir = data_dir::char_state_dir(&self.data_root, character_id.as_str());
        fs::create_dir_all(&state_dir)?;

        let mut value: serde_json::Value = Self::load_live_value(character_id, &state_dir)?;

        mutate(&mut value)?;
        self.commit_state_under_lock(character_id, &state_dir, &value)
    }

    pub fn write(
        &self,
        character_id: &CharacterId,
        state: &serde_json::Value,
    ) -> Result<StateSnapshot, AirpError> {
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
        let state_boundary = state_lock(character_id.as_str());
        let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
        let _state_track = lock_order::track_state();

        let state_dir = data_dir::char_state_dir(&self.data_root, character_id.as_str());
        fs::create_dir_all(&state_dir)?;
        self.commit_state_under_lock(character_id, &state_dir, state)
    }

    /// Load + parse `live.json` for a character. Shared by `read` and `mutate`
    /// so the "missing file → empty object / corrupt file → `Internal` error"
    /// contract can't drift between the two entry points.
    ///
    /// Must be called with both `character_lock` (read) and `state_lock`
    /// (mutex) already held by the caller — both callers acquire them before
    /// invoking this helper.
    fn load_live_value(
        character_id: &CharacterId,
        state_dir: &Path,
    ) -> Result<serde_json::Value, AirpError> {
        let live_path = state_dir.join("live.json");
        if !live_path.exists() {
            return Ok(serde_json::Value::Object(Default::default()));
        }
        let bytes = fs::read(&live_path)?;
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| {
            AirpError::Internal(format!(
                "failed to parse live.json for {}: {e}",
                character_id.as_str()
            ))
        })
    }

    /// Validate + atomically write + history.jsonl append + revision snapshot.
    ///
    /// Must be called with both `character_lock` (read) and `state_lock`
    /// (mutex) already held by the caller. Extracted from `write` so that
    /// `mutate` can reuse the exact same commit semantics after applying
    /// its in-place mutation.
    fn commit_state_under_lock(
        &self,
        character_id: &CharacterId,
        state_dir: &Path,
        state: &serde_json::Value,
    ) -> Result<StateSnapshot, AirpError> {
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

        let state_bytes = serde_json::to_vec_pretty(state)?;
        data_dir::replace_file(&state_dir.join("live.json"), &state_bytes)?;
        let mut history = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(history_path)?;
        serde_json::to_writer(&mut history, &snapshot)?;
        history.write_all(b"\n")?;
        history.sync_data()?;

        // #115 Phase 2e：State 接入统一 revision 合同。
        // `live.json` + `history.jsonl` 已写入；下面在 `characters/{id}/state/` 下
        // 创建 `revisions/{content_revision}/` + `current_revision` 不可变快照。
        // State 已有 `revision`（从 history.jsonl 派生），直接复用为 content_revision，
        // 不需要 lazy migration。
        // 批准文件 `state.json` 内容 = state Value 序列化（与 live.json 对齐，
        // 只含 state 字段，不含 revision/timestamp）。
        let source_hash_hex = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&state_bytes);
            format!("{:x}", hasher.finalize())
        };
        let staged = StagedRevision {
            content_revision: revision,
            asset_kind: AssetKind::State,
            asset_id: character_id.to_string(),
            created_at: snapshot.timestamp.clone(),
            source: AssetSource {
                source_kind: "derived".to_string(),
                source_hash: Some(source_hash_hex),
                source_filename: None,
                converter_version: None,
                imported_at: Some(snapshot.timestamp.clone()),
                parent_revision: if revision > 1 {
                    Some(revision - 1)
                } else {
                    None
                },
            },
            files: vec![("state.json".to_string(), state_bytes)],
        };
        let commit_opts = CommitOptions::new(state_dir);
        commit_revision(&staged, &commit_opts)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: a trailing invalid line larger than the 4096-byte read
    /// window must not prevent `latest_revision` from finding the last valid
    /// snapshot. Moved here from `domain/mod.rs` (E-P1-1 slice 3) because it
    /// tests the now-private `latest_revision` helper.
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

        assert_eq!(latest_revision(&history).unwrap(), 7);
    }
}
