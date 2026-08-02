//! User Persona domain service: read/write/delete multi-persona profiles with
//! revision checking, character/session bindings, and unified revision contract.
//!
//! Extracted from `domain/mod.rs` (E-P1-1 slice 4). Zero behavior change.
//!
//! WEBUI-MVP-PLAN §3.1：先只实现"每用户一个默认 Persona"，最小字段 name / description
//! / variables / revision。写入走 PersonaService（串行化 persona lock + 原子替换 +
//! revision bump + history.jsonl），与 ChatService / StateService 同边界。
//!
//! persona.json 是元设定（不可变 base），state/live.json 是变量漂移覆盖（MVP 不做），
//! state/history.jsonl 是 timeline（MVP 不做）。本 service 当前只管 persona.json 的
//! 读/写/revision——多 Persona、头像、角色/会话绑定、drift/history/rollback 全留 #114
//! 后续阶段。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::data_dir;
use crate::error::AirpError;
use crate::revision::atomic::{commit_revision, CommitOptions, StagedRevision};
use crate::revision::manifest::{AssetKind, AssetSource};
use crate::types::{CharacterId, SessionId, UserId};

use super::locks::persona_lock;

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

/// Persona 生效来源（binding→default 解析后的命中 scope）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectivePersonaSource {
    SessionBinding,
    CharacterBinding,
    Default,
}

/// `PersonaService::resolve_effective_persona` 的结构化结果。
///
/// `effective_persona_id` 是最终生效的 Persona id（session scope 优先，回退
/// character scope，再回退 default——但 default 不在此处填入，由调用方补 `get_default`）。
/// `session_persona_id` / `character_persona_id` 分别是两个 scope 的 owner，供
/// HTTP 端点与 UI 按钮分别决策；没有对应绑定时为 `None`。
#[derive(Debug, Clone)]
pub struct EffectivePersonaResolution {
    pub effective_persona_id: Option<String>,
    pub source: EffectivePersonaSource,
    pub session_persona_id: Option<String>,
    pub character_persona_id: Option<String>,
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
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        self.reject_case_variant_default_file(user_id)?;
        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        let mut ids: Vec<String> = Vec::new();
        if dir.is_dir() {
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
        // `default` is a virtual profile even before its first save.
        if !ids.iter().any(|i| i == "default") {
            ids.push("default".to_string());
        }
        ids.sort();
        Ok(ids)
    }

    /// 读取指定 id 的 Persona。虚拟 `default` 不存在时返回
    /// `Persona::initial(default_name)`；不存在的自定义 id 返回 `NotFound`。
    pub fn get(
        &self,
        user_id: &UserId,
        persona_id: &str,
        default_name: &str,
    ) -> Result<Persona, AirpError> {
        let persona_id = Self::canonical_persona_id(persona_id);
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        if persona_id == "default" {
            self.reject_case_variant_default_file(user_id)?;
        }
        let path = data_dir::user_persona_multi_path(&self.data_root, user_id, persona_id)?;
        if persona_id == "default" {
            let legacy = data_dir::user_persona_path(&self.data_root, user_id);
            let mut persona = self
                .newest_default_copy(&path, &legacy)?
                .unwrap_or_else(|| Persona::initial(default_name));
            persona.id = "default".to_string();
            return Ok(persona);
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(AirpError::NotFound(format!(
                    "persona {persona_id} does not exist"
                )));
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
        let persona_id = Self::canonical_persona_id(persona_id);
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        if persona_id == "default" {
            self.reject_case_variant_default_file(user_id)?;
        }
        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        fs::create_dir_all(&dir)?;
        let path = data_dir::user_persona_multi_path(&self.data_root, user_id, persona_id)?;

        let current_revision = if persona_id == "default" {
            self.newest_default_copy(
                &path,
                &data_dir::user_persona_path(&self.data_root, user_id),
            )?
            .map_or(0, |persona| persona.revision)
        } else {
            self.current_revision_at(&path)?
        };
        if expected_revision != current_revision {
            let conflict = PersonaRevisionConflict { current_revision };
            return Err(AirpError::BadRequest(serde_json::to_string(&conflict)?));
        }

        Self::validate_bindings(&persona.bindings)?;
        self.validate_binding_ownership(user_id, persona_id, &persona.bindings)?;
        persona.schema = Persona::SCHEMA;
        persona.id = persona_id.to_string();
        persona.revision = current_revision + 1;
        persona.updated_at = chrono::Utc::now().to_rfc3339();
        let serialized = serde_json::to_vec_pretty(&persona)?;
        if persona_id == "default" {
            let legacy = data_dir::user_persona_path(&self.data_root, user_id);
            self.replace_default_pair(&path, &legacy, &serialized)?;
        } else {
            data_dir::replace_file(&path, &serialized)?;
        }

        // #115 Phase 2g：Persona 接入统一 revision 合同。
        // 工作副本 `personas/{pid}.json` 已原子写入（含 default 的 legacy 镜像）；
        // 下面在 `users/{uid}/personas/{pid}/` 下创建 `revisions/{content_revision}/`
        // + `current_revision` 不可变快照。文件形态 `{pid}.json` 与目录形态 `{pid}/`
        // 在同一 `personas/` 父目录下共存，互不冲突。
        // Persona 已有 `revision`（自增），直接复用为 content_revision，不需要 lazy migration。
        // 批准文件 `persona.json` 内容 = serialized persona bytes（与 `personas/{pid}.json` 相同）。
        let persona_asset_dir =
            data_dir::user_personas_dir(&self.data_root, user_id).join(persona_id);
        fs::create_dir_all(&persona_asset_dir)?;
        let source_hash_hex = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&serialized);
            format!("{:x}", hasher.finalize())
        };
        let now = persona.updated_at.clone();
        let content_revision = persona.revision;
        let staged = StagedRevision {
            content_revision,
            asset_kind: AssetKind::Persona,
            asset_id: persona_id.to_string(),
            created_at: now.clone(),
            source: AssetSource {
                source_kind: "manual_edit".to_string(),
                source_hash: Some(source_hash_hex),
                source_filename: None,
                converter_version: None,
                imported_at: Some(now),
                parent_revision: if content_revision > 1 {
                    Some(content_revision - 1)
                } else {
                    None
                },
            },
            files: vec![("persona.json".to_string(), serialized)],
        };
        let commit_opts = CommitOptions::new(&persona_asset_dir);
        commit_revision(&staged, &commit_opts)?;
        Ok(persona)
    }

    /// 删除指定 id 的 Persona；`default` 不允许删（返 BadRequest）。删除文件不可逆。
    ///
    /// Gemini #2：除工作副本 `personas/{pid}.json` 外，同时删除 revision 目录
    /// `users/{uid}/personas/{pid}/`，避免后续以同 id 重建 Persona 时
    /// `commit_revision` 因 `revisions/1` 已存在而失败。
    pub fn delete(&self, user_id: &UserId, persona_id: &str) -> Result<(), AirpError> {
        let persona_id = Self::canonical_persona_id(persona_id);
        if persona_id == "default" {
            return Err(AirpError::BadRequest(
                "default persona 不可删除；可用 save 重置内容".to_string(),
            ));
        }
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        // Validate the untrusted ID before constructing either destructive path.
        let path = data_dir::user_persona_multi_path(&self.data_root, user_id, persona_id)?;
        let persona_asset_dir = path.with_extension("");
        match fs::remove_dir_all(&persona_asset_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // Fail closed: retain the working copy when revision cleanup fails.
            Err(error) => return Err(error.into()),
        }
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
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

    /// 查找该用户下绑定到指定角色/会话的 Persona id。
    /// 优先匹配带 session_id 的精确绑定，再匹配全会话通用绑定；同一优先级
    /// 命中多份 Persona 时 fail closed，避免按文件名字典序静默切换 Persona。
    ///
    /// 复用 `resolve_effective_persona` 的结构化解析，保证与 HTTP effective 端点
    /// 使用同一真相。
    pub fn find_for_character(
        &self,
        user_id: &UserId,
        character_id: &str,
        session_id: Option<&str>,
    ) -> Result<Option<String>, AirpError> {
        Ok(self
            .resolve_effective_persona(user_id, character_id, session_id)?
            .effective_persona_id)
    }

    /// 结构化解析某角色/会话下的 Persona binding ownership。
    ///
    /// 返回 effective owner（session scope 优先，回退 character scope）、
    /// 命中 scope、以及两个 scope 各自的 owner（供 HTTP 端点与 UI 分别决策按钮）。
    /// 任一 scope 有多个 owner 时 fail closed，返回 `AirpError::BadRequest`，
    /// 响应指出冲突 scope 与 Persona IDs；不挑文件名最靠前者，不静默回退 default。
    ///
    /// `find_for_character` 与 chat pipeline 的 binding 层都复用本方法，保证
    /// HTTP 可观察结果与聊天激活使用同一真相。
    pub fn resolve_effective_persona(
        &self,
        user_id: &UserId,
        character_id: &str,
        session_id: Option<&str>,
    ) -> Result<EffectivePersonaResolution, AirpError> {
        CharacterId::new(character_id)?;
        if let Some(session_id) = session_id {
            SessionId::parse(session_id)?;
        }
        let mut session_owners: Vec<String> = Vec::new();
        let mut character_owners: Vec<String> = Vec::new();
        for (pid, persona) in self.persona_snapshot(user_id)? {
            for b in &persona.bindings {
                if b.character_id != character_id {
                    continue;
                }
                match &b.session_id {
                    Some(b_sid) if Some(b_sid.as_str()) == session_id => {
                        session_owners.push(pid.clone());
                    }
                    None => {
                        character_owners.push(pid.clone());
                    }
                    _ => {}
                }
            }
        }
        // 任一 scope 多 owner → fail closed。
        if session_owners.len() > 1 {
            return Err(AirpError::BadRequest(format!(
                "ambiguous session-scoped persona binding for character {character_id} session {}: {}",
                session_id.unwrap_or(""),
                session_owners.join(", ")
            )));
        }
        if character_owners.len() > 1 {
            return Err(AirpError::BadRequest(format!(
                "ambiguous character-scoped persona binding for character {character_id}: {}",
                character_owners.join(", ")
            )));
        }
        let session_owner = session_owners.into_iter().next();
        let character_owner = character_owners.into_iter().next();
        // effective = session scope 优先，回退 character scope。
        let (effective, source) = match &session_owner {
            Some(pid) => (Some(pid.clone()), EffectivePersonaSource::SessionBinding),
            None => match &character_owner {
                Some(pid) => (Some(pid.clone()), EffectivePersonaSource::CharacterBinding),
                None => (None, EffectivePersonaSource::Default),
            },
        };
        Ok(EffectivePersonaResolution {
            effective_persona_id: effective,
            source,
            session_persona_id: session_owner,
            character_persona_id: character_owner,
        })
    }

    // ── 内部────────────────────────────────────────────────────────────────────

    /// `default` is a reserved cross-platform storage name. Canonicalizing at
    /// the service boundary prevents case variants from addressing the same
    /// file with different semantics on case-insensitive filesystems.
    fn canonical_persona_id(persona_id: &str) -> &str {
        if persona_id.eq_ignore_ascii_case("default") {
            "default"
        } else {
            persona_id
        }
    }

    /// Older callers could create `Default.json` on case-sensitive filesystems.
    /// Fail closed instead of silently hiding or overwriting that data. Recovery
    /// requires an explicit operator rename, which also forces conflicts with an
    /// existing `default.json` to be resolved deliberately.
    fn reject_case_variant_default_file(&self, user_id: &UserId) -> Result<(), AirpError> {
        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name != "default.json" && name.eq_ignore_ascii_case("default.json") {
                return Err(AirpError::BadRequest(
                    "non-canonical default persona file found; rename it to default.json after resolving any conflict"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Read all Persona files while holding the per-user lock so binding
    /// resolution observes one committed ownership snapshot.
    fn persona_snapshot(&self, user_id: &UserId) -> Result<Vec<(String, Persona)>, AirpError> {
        let lock = persona_lock(user_id.as_str());
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        self.reject_case_variant_default_file(user_id)?;

        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        let mut ids = Vec::new();
        if dir.is_dir() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if let Some(id) = name.strip_suffix(".json") {
                    if data_dir::validate_id_segment(id).is_ok() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        if !ids.iter().any(|id| id == "default") {
            ids.push("default".to_string());
        }
        ids.sort();

        let mut snapshot = Vec::with_capacity(ids.len());
        for id in ids {
            let mut persona = if id == "default" {
                let canonical =
                    data_dir::user_persona_multi_path(&self.data_root, user_id, "default")?;
                let legacy = data_dir::user_persona_path(&self.data_root, user_id);
                self.newest_default_copy(&canonical, &legacy)?
                    .unwrap_or_else(|| Persona::initial("User"))
            } else {
                let path =
                    data_dir::user_persona_multi_path(&self.data_root, user_id, id.as_str())?;
                self.parse_persona_bytes(&fs::read(path)?)?
            };
            persona.id = id.clone();
            snapshot.push((id, persona));
        }
        Ok(snapshot)
    }

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

    fn newest_default_copy(
        &self,
        canonical: &Path,
        legacy: &Path,
    ) -> Result<Option<Persona>, AirpError> {
        let read = |path: &Path| -> Result<Option<(Persona, std::time::SystemTime)>, AirpError> {
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let persona = self.parse_persona_bytes(&bytes)?;
            let modified = fs::metadata(path)?
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            Ok(Some((persona, modified)))
        };

        match (read(canonical)?, read(legacy)?) {
            (None, None) => Ok(None),
            (Some((persona, _)), None) | (None, Some((persona, _))) => Ok(Some(persona)),
            (Some((canonical, canonical_time)), Some((legacy, legacy_time))) => {
                if legacy.revision > canonical.revision
                    || (legacy.revision == canonical.revision && legacy_time > canonical_time)
                {
                    Ok(Some(legacy))
                } else {
                    Ok(Some(canonical))
                }
            }
        }
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

    /// Enforce one owner per character/session scope while the per-user save
    /// lock is held. Concurrent bind requests can race before `save`, but only
    /// one can pass this check and persist.
    fn validate_binding_ownership(
        &self,
        user_id: &UserId,
        persona_id: &str,
        bindings: &[PersonaBinding],
    ) -> Result<(), AirpError> {
        self.reject_case_variant_default_file(user_id)?;

        let check_owner = |owner_id: &str, owner: &Persona| -> Result<(), AirpError> {
            for binding in bindings {
                if owner.bindings.iter().any(|existing| {
                    existing.character_id == binding.character_id
                        && existing.session_id == binding.session_id
                }) {
                    let scope = binding.session_id.as_deref().map_or_else(
                        || format!("character {}", binding.character_id),
                        |session_id| {
                            format!("character {} session {session_id}", binding.character_id)
                        },
                    );
                    return Err(AirpError::BadRequest(format!(
                        "persona binding scope {scope} is already owned by {owner_id}"
                    )));
                }
            }
            Ok(())
        };

        let dir = data_dir::user_personas_dir(&self.data_root, user_id);
        if dir.is_dir() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Some(owner_id) = name.strip_suffix(".json") else {
                    continue;
                };
                if data_dir::validate_id_segment(owner_id).is_err()
                    || owner_id == persona_id
                    || owner_id == "default"
                {
                    continue;
                }
                let owner = self.parse_persona_bytes(&fs::read(entry.path())?)?;
                check_owner(owner_id, &owner)?;
            }
        }

        if persona_id != "default" {
            let canonical = data_dir::user_persona_multi_path(&self.data_root, user_id, "default")?;
            let legacy = data_dir::user_persona_path(&self.data_root, user_id);
            if let Some(owner) = self.newest_default_copy(&canonical, &legacy)? {
                check_owner("default", &owner)?;
            }
        }

        Ok(())
    }

    fn replace_default_pair(
        &self,
        canonical: &Path,
        legacy: &Path,
        bytes: &[u8],
    ) -> Result<(), AirpError> {
        let previous_canonical = fs::read(canonical).ok();
        data_dir::replace_file(canonical, bytes)?;
        if let Err(write_error) = data_dir::replace_file(legacy, bytes) {
            let rollback = match previous_canonical {
                Some(previous) => data_dir::replace_file(canonical, &previous),
                None => match fs::remove_file(canonical) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(rollback_error) = rollback {
                return Err(AirpError::Internal(format!(
                    "legacy persona mirror failed ({write_error}); canonical rollback failed ({rollback_error})"
                )));
            }
            return Err(write_error);
        }
        Ok(())
    }
}
