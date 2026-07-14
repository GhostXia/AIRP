//! HTTP handler functions for the daemon API.
use super::types::{ChatCompletionRequest, HistoryQuery, RegenRequest, RollbackRequest};
use super::{DaemonState, SettingsView};
use crate::chat_store::ChatLog;
use crate::domain::{ChatService, LorebookService, Persona, PersonaService};
use crate::error::AirpError;
use crate::types::{CharacterId, PresetId, SessionId, UserId};
use crate::{chat_pipeline, data_dir};
use axum::{
    http::{header, StatusCode},
    response::{sse::Sse, IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;

const MAX_DERIVED_CHARACTER_ID_BYTES: usize = 120;
const MODELS_PROXY_TIMEOUT_DEFAULT: Duration = Duration::from_secs(5);

// Preset imports are infrequent administrative writes. A single process-wide
// lock keeps the fixed temporary/backup names safe without retaining an
// unbounded per-preset lock map.
static PRESET_IMPORT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// #42 F-6：/v1/models 上游超时。默认 5s，可用 `AIRP_MODELS_PROXY_TIMEOUT_MS`
/// 覆盖（跨境 provider 偏慢时无需重编译；测试也借此走快速超时路径）。
fn models_proxy_timeout() -> Duration {
    std::env::var("AIRP_MODELS_PROXY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(MODELS_PROXY_TIMEOUT_DEFAULT)
}

// ── Private request/response types (handler-local) ────────────────────────────

/// R-01: Import a character card. Path-first (守不变式6)：优先 `card_path`
/// 让引擎读盘；`card_json`/`card_png_base64` 为 fallback（无真实路径时）。
/// `character_id` 可选——不传时引擎从 `card.name` slugify 派生并在响应返回。
#[derive(Debug, Deserialize)]
pub(super) struct ImportCharacterRequest {
    /// 落盘目录名。None → 引擎从卡内 name 派生；重名自动加后缀。
    pub character_id: Option<CharacterId>,
    /// 绝对路径——引擎 fs::read 后按内容嗅探（PNG 魔数 → png_parser，否则 JSON）。
    pub card_path: Option<std::path::PathBuf>,
    /// TavernCardV2 JSON string (fallback)。
    pub card_json: Option<String>,
    /// Raw PNG bytes, base64-encoded (fallback)。
    pub card_png_base64: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ImportCharacterResponse {
    /// 最终落盘用的 id（传入则原样；未传则为派生 id）。
    pub character_id: String,
    pub card_format: String,
}

#[derive(serde::Deserialize)]
pub(super) struct AddCharacterBody {
    character_id: String,
    #[serde(default)]
    role: crate::scene::CharacterRole,
    #[serde(default)]
    intro: String,
}

// ── Settings handlers ─────────────────────────────────────────────────────────

/// GET /v1/settings — 返回当前 daemon 运行时配置（api_key 脱敏）。
pub(super) async fn get_settings(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<SettingsView>, AirpError> {
    let cfg = state
        .config
        .read()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
    Ok(Json(SettingsView::from_config(&cfg, &state.data_root)))
}

/// POST /v1/settings — 用 `PartialAppConfig` 合并到当前运行时配置 + 落盘
/// `data/settings.json`。空字符串视为未设置，避免抹掉合法上层值。
pub(super) async fn update_settings(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(patch): Json<crate::config::PartialAppConfig>,
) -> Result<Json<SettingsView>, AirpError> {
    if patch
        .access_api_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
    {
        let cfg = state
            .config
            .read()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        if cfg.deployment_mode == crate::config::DeploymentMode::Production {
            return Err(AirpError::BadRequest(
                "AIRP_ACCESS_KEY cannot be changed through /v1/settings in production; rotate the gateway and engine secret together, then restart"
                    .to_string(),
            ));
        }
    }
    // 1) 合并到内存
    let merged = {
        let mut cfg = state
            .config
            .write()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        if let Some(p) = patch.provider {
            cfg.provider = p;
        }
        if let Some(e) = patch.endpoint.filter(|s| !s.is_empty()) {
            cfg.endpoint = e;
        }
        if let Some(k) = patch.api_key.filter(|s| !s.is_empty()) {
            cfg.api_key = Some(k);
        }
        if let Some(m) = patch.model.filter(|s| !s.is_empty()) {
            cfg.model = m;
        }
        if let Some(v) = patch.volume {
            v.validate()
                .map_err(|e| AirpError::BadRequest(format!("VolumeConfig 不合法: {}", e)))?;
            cfg.volume_config = v;
        }
        if let Some(k) = patch.access_api_key.filter(|s| !s.is_empty()) {
            cfg.access_api_key = Some(k);
        }
        if let Some(e) = patch.engine {
            cfg.engine = e;
        }
        if let Some(q) = patch.quota {
            cfg.quota = q;
        }
        cfg.clone()
    };

    // 2) Persist only non-secret settings. Provider and daemon credentials are
    // runtime-only and must be supplied through AIRP_* env or this request.
    let on_disk = serde_json::json!({
        "provider": merged.provider,
        "endpoint": merged.endpoint,
        "model": merged.model,
        "volume": merged.volume_config,
        "engine": merged.engine,
        "quota": merged.quota,
    });
    let path = state.data_root.join("settings.json");
    fs::write(&path, serde_json::to_string_pretty(&on_disk)?)?;

    Ok(Json(SettingsView::from_config(&merged, &state.data_root)))
}

// ── Session handlers ──────────────────────────────────────────────────────────

/// GET /v1/sessions/:character_id — list all named sessions for a character.
pub(super) async fn list_sessions_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<Vec<SessionId>>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sessions = ChatService::new(&state.data_root).list_sessions(&cid)?;
    Ok(Json(sessions))
}

/// POST /v1/sessions/:character_id — create a new named session, return its ID.
pub(super) async fn create_session_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<SessionId>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sid = ChatService::new(&state.data_root).create_session(&cid)?;
    Ok(Json(sid))
}

/// DELETE /v1/sessions/:character_id/:session_id — 删除一个命名会话目录。
/// #35：destructive，调用方负责确认。返回 `{deleted, status}`。会话不存在 → 404。
pub(super) async fn delete_session_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((character_id, session_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sid = SessionId::parse(&session_id)?;
    ChatService::new(&state.data_root).delete_session(&cid, &sid)?;
    Ok(Json(serde_json::json!({
        "deleted": sid.to_string(),
        "character_id": cid.as_str(),
        "status": "ok"
    })))
}

// ── Persona handlers（#114，每用户一个默认 Persona）────────────────────────────
//
// WEBUI-MVP-PLAN §3.1：GET 读当前 Persona（不存在返回兜底，不写盘）；
// PUT 原子写并 revision bump；expected_revision 不匹配返回 400 + PersonaRevisionConflict JSON。
// `user_id` 走路径参数，经 UserId::new 校验（拒绝路径遍历）；`default_name` 走 query string。

/// GET /v1/users/:user_id/persona — 读当前 Persona；不存在返回兜底（revision=0）。
pub(super) async fn get_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let persona = PersonaService::new(&state.data_root).get_default(&uid, "User")?;
    Ok(Json(persona))
}

/// PUT /v1/users/:user_id/persona — 原子写入 Persona；revision 不匹配返回 400。
pub(super) async fn update_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(payload): Json<UpdatePersonaRequest>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let service = PersonaService::new(&state.data_root);
    let current = service.get_default(&uid, "User")?;
    let persona = Persona {
        schema: Persona::SCHEMA,
        revision: 0, // save 内会 bump；payload 的 revision 不信，用 expected_revision 校验
        updated_at: String::new(),
        name: payload.name,
        description: payload.description,
        variables: payload.variables,
        id: current.id,
        // The legacy endpoint does not own schema-v2 binding fields. Preserve
        // them so editing a name or description cannot silently unbind chats.
        bindings: current.bindings,
    };
    let saved = service.save_default(&uid, payload.expected_revision, persona)?;
    Ok(Json(saved))
}

/// PUT /v1/users/:user_id/persona 的请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdatePersonaRequest {
    /// 客户端持有的当前 revision；不匹配服务端时返回 400 + PersonaRevisionConflict。
    pub expected_revision: u64,
    /// 用户显示名。
    pub name: String,
    /// 自由描述。
    #[serde(default)]
    pub description: String,
    /// 自定义变量表。
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

// ── Character handlers ────────────────────────────────────────────────────────

/// GET /v1/characters — list all available character folder names
pub(super) async fn list_characters(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<Vec<String>>, AirpError> {
    let chars = data_dir::list_characters(&state.data_root)?;
    Ok(Json(chars))
}

/// GET /v1/presets — list all available preset file names under data/presets/
pub(super) async fn list_presets_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<Vec<String>>, AirpError> {
    let presets = data_dir::list_presets(&state.data_root)?;
    Ok(Json(presets))
}

/// GET /v1/presets/:preset_id — get all prompts of a preset
pub(super) async fn get_preset_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(preset_id): axum::extract::Path<String>,
) -> Result<Json<Vec<crate::orchestrator::TavernPrompt>>, AirpError> {
    let preset_id = PresetId::new(preset_id)?;
    let normalized_path = data_dir::preset_json_path(&state.data_root, preset_id.as_str());
    let legacy_path = data_dir::legacy_preset_json_path(&state.data_root, preset_id.as_str());
    let preset_path = if normalized_path.exists() {
        normalized_path
    } else {
        legacy_path
    };
    if !preset_path.exists() {
        return Err(AirpError::NotFound(format!(
            "Preset {} not found",
            preset_id
        )));
    }
    let json_str = fs::read_to_string(&preset_path)?;
    let preset: crate::orchestrator::TavernPreset = serde_json::from_str(&json_str)
        .map_err(|e| AirpError::BadRequest(format!("Invalid preset JSON: {}", e)))?;

    Ok(Json(preset.prompts.unwrap_or_default()))
}

// ── Preset import（#114，WEBUI-MVP-PLAN §3.1：最小 JSON 导入）────────────────────
//
// 接收 `{preset_id, preset_json}`，校验 TavernPreset schema 后原子写盘到
// `presets/{id}/preset.json`。保留 raw sidecar（原样写盘，不解释 prompt 内容），
// 拒绝脚本执行和路径输入（preset_id 走 PresetId::new 校验，preset_json 走 serde
// 反序列化 TavernPreset）。rename/duplicate/export、字段级迁移报告、PromptAssemblyTrace
// 留 #115。

/// POST /v1/presets/import — 校验 + 落盘一份 preset JSON。
pub(super) async fn import_preset_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<ImportPresetRequest>,
) -> Result<Json<ImportPresetResponse>, AirpError> {
    let preset_id = PresetId::new(req.preset_id)?;
    // 校验 JSON 形状：必须是 TavernPreset（顶层 prompts[] + 模型参数）。
    // 反序列化失败 → BadRequest，不落盘，避免脏文件残留。
    let cleaned = data_dir::strip_utf8_bom(&req.preset_json).to_owned();
    let parsed: crate::orchestrator::TavernPreset =
        serde_json::from_str(&cleaned).map_err(|e| {
            AirpError::BadRequest(format!("preset_json 不是有效 TavernPreset JSON: {}", e))
        })?;
    // 再序列化为规范 pretty JSON 写盘（保留 raw sidecar，原样不解释 prompt）。
    let bytes = serde_json::to_vec_pretty(&parsed)
        .map_err(|e| AirpError::Internal(format!("preset 序列化失败: {}", e)))?;

    let _guard = PRESET_IMPORT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("preset import lock poisoned");
    let dir = state.data_root.join("presets").join(preset_id.as_str());
    let final_path = dir.join("preset.json");
    let legacy_path = data_dir::legacy_preset_json_path(&state.data_root, preset_id.as_str());
    if final_path.exists() || legacy_path.exists() {
        return Err(AirpError::BadRequest(format!(
            "preset {} already exists; explicit overwrite is not supported",
            preset_id.as_str()
        )));
    }
    fs::create_dir_all(&dir)?;
    data_dir::replace_file(&final_path, &bytes)?;

    Ok(Json(ImportPresetResponse {
        preset_id: preset_id.to_string(),
        prompts_count: parsed.prompts.map(|p| p.len()).unwrap_or(0),
    }))
}

/// POST /v1/presets/import 的请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportPresetRequest {
    /// 目标 preset ID；走 PresetId::new 校验，拒路径遍历。
    pub preset_id: String,
    /// TavernPreset 规范的 JSON 文本（原样 sidecar，不解释）。
    pub preset_json: String,
}

/// POST /v1/presets/import 的响应体。
#[derive(Debug, Serialize)]
pub(super) struct ImportPresetResponse {
    pub preset_id: String,
    pub prompts_count: usize,
}

// ── Chat handlers ─────────────────────────────────────────────────────────────

/// POST /v1/chat/history — get chat history for a character
pub(super) async fn get_chat_history(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(query): Json<HistoryQuery>,
) -> Result<Json<serde_json::Value>, AirpError> {
    // #37 cursor 分页：传 limit/before 走窗口；不传 → 全量（向后兼容旧客户端）。
    if query.limit.is_some() || query.before.is_some() {
        let window = ChatService::new(&state.data_root).history_window(
            &query.character_id,
            query.session_id.as_ref(),
            query.limit,
            query.before.as_deref(),
        )?;
        return Ok(Json(serde_json::to_value(window)?));
    }
    // legacy 全量返回必须保留 ChatLog 的既有响应形状。
    let log = ChatService::new(&state.data_root)
        .history(&query.character_id, query.session_id.as_ref())?;
    Ok(Json(serde_json::to_value(log)?))
}

/// POST /v1/chat/rollback — rollback to a specific message index
pub(super) async fn rollback_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RollbackRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    // #37：message_id / message_index 二选一校验。
    if let Err(msg) = req.validate_rollback_target() {
        return Err(AirpError::BadRequest(msg));
    }
    let service = ChatService::new(&state.data_root);
    let (log, _) = match (req.message_index, req.message_id.as_deref()) {
        (Some(idx), None) => service.rollback(&req.character_id, req.session_id.as_ref(), idx)?,
        (None, Some(id)) => {
            service.rollback_to_id(&req.character_id, req.session_id.as_ref(), id)?
        }
        // validate_rollback_target 已挡住二义与都空，这里不可达。
        _ => {
            return Err(AirpError::BadRequest(
                "rollback target invariant violated".into(),
            ))
        }
    };
    Ok(Json(log))
}

/// POST /v1/chat/regen — delete last assistant message for regeneration
pub(super) async fn regen_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RegenRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let log =
        ChatService::new(&state.data_root).regen(&req.character_id, req.session_id.as_ref())?;
    Ok(Json(log))
}

pub(super) async fn chat_completion(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    // DX-3: quota check (before any expensive work; resolves same effective_root as pipeline)
    let (quota_config, effective_root) = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        let quota = cfg.quota.clone();
        let root =
            crate::data_dir::resolve_effective_root(&state.data_root, payload.user_id.as_deref())?;
        (quota, root)
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    let pipeline = chat_pipeline::prepare_pipeline(&payload, &state)?;
    Ok(Sse::new(chat_pipeline::build_sse_stream(pipeline)))
}

/// M_AGENT-1: `POST /v1/agent/run` — 多步 loop 入口（SSE）。
///
/// 计划书 §4.3：`/v1/chat/completions` ≡ `max_steps=1` 的 `/v1/agent/run`。
/// 老客户端继续打 `/v1/chat/completions`（单回合）；要 agentic 的显式打此端点。
///
/// 复用 `AgentLoop::run`（协调器）；quota 检查与 chat_completion 同路径。
pub(super) async fn agent_run(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(payload): Json<crate::agent::AgentRunRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    // DX-3: quota check（与 chat_completion 同路径）
    let (quota_config, effective_root) = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        let quota = cfg.quota.clone();
        let root = crate::data_dir::resolve_effective_root(
            &state.data_root,
            payload.base.user_id.as_deref(),
        )?;
        (quota, root)
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    let cancel = tokio_util::sync::CancellationToken::new();
    // 客户端断连 → drop SSE 流 → 我们不显式取消（M_AGENT-1 骨架）；
    // M_AGENT-5 会接 SSE 连接生命周期到 cancel token。
    let looper = crate::agent::AgentLoop::new(state);
    Ok(Sse::new(looper.run(payload, cancel)))
}

pub(super) async fn list_agent_tools(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Json<Vec<crate::agent::tools::ToolMeta>> {
    Json(crate::agent::tools::default_registry(state).list())
}

// ── Character card import ─────────────────────────────────────────────────────

/// M_MCP MCP-2: 角色卡导入的内部实现（pub(crate) 供 daemon HTTP handler 与 MCP tool 共享）。
///
/// 三选一参数（按优先级）：`card_path`（path-first，引擎读盘）/ `card_json` / `card_png_base64`。
/// 全为 None → 400。
///
/// `character_id` 为 `None` 时，引擎解析卡后 slugify `data.name` 派生默认 id 并返回；
/// 重名（目标目录已存在）时加 `-2`/`-3` 后缀，不覆盖既有角色。
///
/// 副作用（CF-7 解包）：
/// - `card.json` 或 `card.png` 写入角色根目录（向后兼容）
/// - `card/raw.json` — 完整 TavernV2 JSON（最小 sidecar，守 ASSET-SPEC 规则2 存储永不丢）
/// - `card/greetings/00.md` — first_mes（其他 `0x.md` 为 alternate_greetings）
/// - `world/lorebook.json` — 角色卡内嵌 character_book 转 Lorebook 格式
///
/// 返回 `(character_id, card_format, json_str)`。`character_id` 是最终落盘用的 id
/// （可能与传入不同——传入 None 时为派生 id）。
pub(crate) fn import_card_to_disk(
    data_root: &std::path::Path,
    character_id: Option<&str>,
    card_path: Option<&std::path::Path>,
    card_json: Option<String>,
    card_png_base64: Option<String>,
) -> Result<(String, String, String), AirpError> {
    // 阶段 1：提取 + 校验 JSON（path/PNG 先读入内存，暂不落盘）。
    // 写盘推迟到形状校验之后，避免被拒的预设残留脏文件。
    let (card_format, json_str, png_bytes): (String, String, Option<Vec<u8>>) = if let Some(path) =
        card_path
    {
        // path-first 主路径：引擎读盘（守不变式6——大 blob 不经线协议）。
        // ⚠️ RR-001 / 审计 2026-07-04：card_path = 服务端任意绝对路径读。
        // 门控：仅当 engine 启动时设了 AIRP_ALLOW_LOCAL_PATH=1 才开放此分支。
        // Tauri sidecar 启动脚本带此变量；对外暴露的 engine 不带 → Web/远端
        // 调用方即使伪造 JSON body 带 card_path 也被拒。不可伪造（env 在进程
        // 启动时定，非请求头）。审计裁定：Web 永不走 card_path，即使持 bearer。
        let allow_local_path = crate::config::local_path_import_enabled();
        if !allow_local_path {
            return Err(AirpError::BadRequest(
                    "card_path 任意路径读已禁用（AIRP_ALLOW_LOCAL_PATH 未设）。Web/远端调用方请用 card_png_base64 或 card_json 字段（JSON body，非 multipart）。".to_string(),
                ));
        }
        let bytes = fs::read(path)
            .map_err(|e| AirpError::BadRequest(format!("读取 card_path 失败: {}", e)))?;
        // 按内容嗅探：PNG 魔数 → png_parser；否则当 JSON 文本。
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            let json = crate::png_parser::parse_png_character_card_bytes(&bytes)?;
            ("png".to_string(), json, Some(bytes))
        } else {
            let clean = data_dir::strip_utf8_bom(
                std::str::from_utf8(&bytes)
                    .map_err(|e| AirpError::BadRequest(format!("card_path 非 UTF-8: {}", e)))?,
            )
            .to_owned();
            let _ = serde_json::from_str::<serde_json::Value>(&clean)
                .map_err(|e| AirpError::BadRequest(format!("card_path 不是有效 JSON: {}", e)))?;
            ("json".to_string(), clean, None)
        }
    } else if let Some(json) = card_json {
        let clean = data_dir::strip_utf8_bom(&json).to_owned();
        let _ = serde_json::from_str::<serde_json::Value>(&clean)
            .map_err(|e| AirpError::BadRequest(format!("card_json 不是有效 JSON: {}", e)))?;
        ("json".to_string(), clean, None)
    } else if let Some(png_b64) = card_png_base64 {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&png_b64)
            .map_err(|e| AirpError::BadRequest(format!("base64 解码失败: {}", e)))?;
        // 从内存字节解析 tEXt/ccv3；解析失败即报错，不落盘。
        let json = crate::png_parser::parse_png_character_card_bytes(&bytes)?;
        ("png".to_string(), json, Some(bytes))
    } else {
        return Err(AirpError::BadRequest(
            "必须提供 card_path / card_json / card_png_base64 之一".to_string(),
        ));
    };

    // 阶段 2：形状校验。若内容明显是 SillyTavern 预设（顶层 prompts[] + 模型参数），
    // 拒绝导入为角色卡，提示改用 import_preset。此处尚未写盘，拒绝不留脏文件。
    if matches!(
        crate::orchestrator::card::detect_json_shape(&json_str),
        crate::orchestrator::card::JsonShape::Preset
    ) {
        return Err(AirpError::BadRequest(
            "内容像 SillyTavern 预设（顶层 prompts[] + 模型参数），不是角色卡。请改用 import_preset 导入。".to_string(),
        ));
    }

    // v1 平铺卡归一化为 v2 schema（data 嵌套）。v2/v3 卡原样返回。
    // 不归一化则下游 TavernCardV2 解析失败，greetings/lorebook 全丢。
    let json_str = crate::orchestrator::card::normalize_v1_to_v2(&json_str);

    // 阶段 2.5：确定 character_id。传入则校验；未传则 slugify card.name 派生。
    let final_id: String = match character_id {
        Some(id) => {
            // 传入的 id 必须本身合法（UI/调用方负责）；CharacterId 校验。
            CharacterId::new(id)?;
            id.to_string()
        }
        None => {
            let name = extract_card_name(&json_str);
            let base = slugify_id(&name);
            resolve_unique_id(data_root, &base)?
        }
    };
    // 最终 id 再过一次 newtype 校验（slugify 可能产生需复核的串）+ 防 None 路径漏网。
    CharacterId::new(&final_id)?;
    let char_dir = data_dir::character_dir(data_root, &final_id)?;

    // 阶段 3：落盘（仅在校验通过后）。
    if let Some(bytes) = png_bytes {
        fs::write(char_dir.join("card.png"), &bytes)?;
        let card_dir = data_dir::char_card_dir(data_root, &final_id)?;
        fs::write(card_dir.join("card.png"), &bytes)?;
    } else {
        fs::write(char_dir.join("card.json"), &json_str)?;
    }

    // CF-7: 解包资产（非阻塞；失败仅 warn）
    extract_card_assets(data_root, &final_id, &json_str);

    Ok((final_id, card_format, json_str))
}

/// 从归一化后的 TavernV2 JSON 提取 `data.name`。失败/缺字段返回空串。
fn extract_card_name(json_str: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json_str)
        .ok()
        .and_then(|v| {
            v.get("data")
                .and_then(|d| d.get("name"))?
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 把卡名 sanitize 成合法 id_segment：NFC 归一化，替换文件名非法字符/空白，
/// 移除不可见控制字符，去行首点，去 `..`，并限制长度。空串返回 `character`。
fn slugify_id(name: &str) -> String {
    let mut s = String::with_capacity(name.len().min(MAX_DERIVED_CHARACTER_ID_BYTES));
    for c in name.nfc() {
        if is_invisible_id_control(c) {
            continue;
        }
        match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => s.push('_'),
            c if c.is_whitespace() => s.push('_'),
            _ => s.push(c),
        }
    }
    // 去行首点（validate 拒 `.`/`..` 及以点开头）。
    while s.starts_with('.') {
        s.remove(0);
    }
    // 去 `..` 子串（validate 拒含 `..`）——逐字符折叠连续点为单点。
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_dot = false;
    for c in s.chars() {
        if c == '.' {
            if !prev_dot {
                collapsed.push(c);
            }
            prev_dot = true;
        } else {
            collapsed.push(c);
            prev_dot = false;
        }
    }
    let mut s = collapsed;
    truncate_utf8_bytes(&mut s, MAX_DERIVED_CHARACTER_ID_BYTES);
    if s.is_empty() {
        "character".to_string()
    } else {
        s
    }
}

fn is_invisible_id_control(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' // zero-width + LRM/RLM
                | '\u{202A}'..='\u{202E}' // bidi embedding/override
                | '\u{2060}'..='\u{206F}' // word joiner + bidi isolates
                | '\u{FEFF}' // BOM / zero-width no-break space
        )
}

fn truncate_utf8_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// 目标角色目录已存在时加 `-2`/`-3` 后缀直到空闲，不覆盖既有角色。
fn resolve_unique_id(data_root: &std::path::Path, base: &str) -> Result<String, AirpError> {
    let candidate = |id: &str| data_root.join("characters").join(id).exists();
    if !candidate(base) {
        return Ok(base.to_string());
    }
    for n in 2..u32::MAX {
        let id = format!("{}-{}", base, n);
        if !candidate(&id) {
            return Ok(id);
        }
    }
    Err(AirpError::BadRequest("角色 id 重名后缀耗尽".to_string()))
}

pub(super) async fn import_character(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<ImportCharacterRequest>,
) -> Result<Json<ImportCharacterResponse>, AirpError> {
    let cid_str = req.character_id.as_ref().map(|c| c.as_str().to_string());
    let (final_id, card_format, _json_str) = import_card_to_disk(
        &state.data_root,
        cid_str.as_deref(),
        req.card_path.as_deref(),
        req.card_json,
        req.card_png_base64,
    )?;
    Ok(Json(ImportCharacterResponse {
        character_id: final_id,
        card_format,
    }))
}

/// POST /v1/characters/:character_id/reextract
/// 对已导入的角色卡重新运行 CF-7 资产解包（world/ + card/greetings/）。
pub(super) async fn reextract_character_assets(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id_str): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id_str)
        .map_err(|e| AirpError::BadRequest(format!("非法 character_id: {}", e)))?;
    let char_dir = data_dir::character_dir(&state.data_root, cid.as_str())?;

    let json_str = if char_dir.join("card").join("raw.json").exists() {
        let raw = fs::read_to_string(char_dir.join("card").join("raw.json"))?;
        data_dir::strip_utf8_bom(&raw).to_owned()
    } else if char_dir.join("card.json").exists() {
        let raw = fs::read_to_string(char_dir.join("card.json"))?;
        data_dir::strip_utf8_bom(&raw).to_owned()
    } else if char_dir.join("card.png").exists() {
        crate::png_parser::parse_png_character_card(char_dir.join("card.png"))?
    } else {
        return Err(AirpError::NotFound(format!(
            "角色 {} 无可用卡片文件（card.json / card.png）",
            cid.as_str()
        )));
    };

    extract_card_assets(&state.data_root, cid.as_str(), &json_str);

    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "status": "ok",
        "message": "资产解包已触发（world/lorebook.json + card/greetings/）"
    })))
}

/// CF-7: 从 TavernV2 JSON 解包子资产，写入功能子目录。
///
/// 失败路径 `tracing::warn` 而非返回错误——导入主路径已完成，资产解包是增值操作。
/// `pub(crate)` 用于 M_MCP MCP-2 复用（避免与 daemon::import_character 代码漂移）。
pub(crate) fn extract_card_assets(data_root: &std::path::Path, character_id: &str, json_str: &str) {
    use crate::orchestrator::card::TavernCardV2;

    let card: TavernCardV2 = match serde_json::from_str(json_str) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(err = %e, character_id, "CF-7: 解析 TavernCardV2 失败，跳过资产解包");
            return;
        }
    };

    // ── card/raw.json ──
    match data_dir::char_card_dir(data_root, character_id) {
        Ok(card_dir) => {
            if let Err(e) = fs::write(card_dir.join("raw.json"), json_str) {
                tracing::warn!(err = %e, character_id, "CF-7: 写 card/raw.json 失败");
            }
        }
        Err(e) => tracing::warn!(err = %e, character_id, "CF-7: 创建 card/ 目录失败"),
    }

    // ── card/greetings/ ──
    match data_dir::char_greetings_dir(data_root, character_id) {
        Ok(greet_dir) => {
            if let Some(ref fm) = card.data.first_mes {
                let path = greet_dir.join("00.md");
                if let Err(e) = fs::write(&path, fm) {
                    tracing::warn!(err = %e, "CF-7: 写 greetings/00.md 失败");
                }
            }
            for (i, greeting) in card.data.alternate_greetings.iter().enumerate() {
                let path = greet_dir.join(format!("{:02}.md", i + 1));
                if let Err(e) = fs::write(&path, greeting) {
                    tracing::warn!(err = %e, idx = i + 1, "CF-7: 写 greetings/{:02}.md 失败", i + 1);
                }
            }
        }
        Err(e) => tracing::warn!(err = %e, character_id, "CF-7: 创建 greetings/ 目录失败"),
    }

    // ── world/lorebook.json ──
    if let Some(ref cb) = card.data.character_book {
        match convert_character_book_to_lorebook(cb) {
            Some(lorebook) => match serde_json::to_string_pretty(&lorebook) {
                Ok(lb_json) => {
                    let lb_path = data_dir::char_world_lorebook_path(data_root, character_id);
                    if let Err(e) = data_dir::char_world_dir(data_root, character_id) {
                        tracing::warn!(err = %e, "CF-7: 创建 world/ 目录失败");
                        return;
                    }
                    if let Err(e) = fs::write(&lb_path, lb_json) {
                        tracing::warn!(err = %e, "CF-7: 写 world/lorebook.json 失败");
                    } else {
                        tracing::info!(
                            character_id,
                            entries = lorebook.entries.len(),
                            "CF-7: world/lorebook.json 已写入"
                        );
                    }
                }
                Err(e) => tracing::warn!(err = %e, "CF-7: 序列化 Lorebook 失败"),
            },
            None => tracing::warn!(
                character_id,
                "CF-7: character_book 解析失败，跳过 lorebook 写入"
            ),
        }
    }
}

/// CF-7: 将 TavernV2 character_book Value 转换为 `Lorebook` 结构。
fn convert_character_book_to_lorebook(
    cb: &serde_json::Value,
) -> Option<crate::orchestrator::lorebook::Lorebook> {
    use crate::orchestrator::lorebook::{Lorebook, LorebookEntry};

    let entries_val = cb.get("entries").unwrap_or(cb);

    let raw_entries: Vec<&serde_json::Value> = if let Some(map) = entries_val.as_object() {
        map.values().collect()
    } else {
        entries_val.as_array()?.iter().collect()
    };

    let mut entries: Vec<LorebookEntry> = raw_entries
        .into_iter()
        .filter_map(|v| {
            let keys: Vec<String> = v
                .get("keys")
                .and_then(|k| k.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(|s| s.to_owned()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let content = v.get("content")?.as_str()?.to_owned();
            let enabled = v
                .get("disable")
                .and_then(|d| d.as_bool())
                .map(|disable| !disable);
            let priority = v
                .get("order")
                .or_else(|| v.get("insertion_order"))
                .and_then(|p| p.as_i64())
                .map(|p| p as i32);
            let constant = v.get("constant").and_then(|c| c.as_bool());
            let comment = v
                .get("comment")
                .and_then(|c| c.as_str())
                .map(|s| s.to_owned());
            Some(LorebookEntry {
                keys,
                content,
                enabled,
                priority,
                constant,
                comment,
            })
        })
        .collect();

    // 统一使用 trigger 的默认值（10）与排序方向（降序），避免存储顺序与运行时
    // trigger 输出顺序漂移。trigger 会重新排序，此处排序主要为可读性。
    entries.sort_by_key(|e| std::cmp::Reverse(e.priority.unwrap_or(10)));

    Some(Lorebook { entries })
}

// ── Character state / avatar handlers ────────────────────────────────────────

/// GET /v1/characters/:character_id/avatar — serve card.png as image/png.
pub(super) async fn get_character_avatar(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let char_dir = match data_dir::character_dir(&state.data_root, char_id.as_str()) {
        Ok(d) => d,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let png_path = char_dir.join("card.png");
    match fs::read(&png_path) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state
pub(super) async fn get_character_state(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let live_path = data_dir::char_state_dir(&state.data_root, char_id.as_str()).join("live.json");
    match fs::read_to_string(&live_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state/schema
pub(super) async fn get_character_state_schema(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let schema_path =
        data_dir::char_state_dir(&state.data_root, char_id.as_str()).join("schema.json");
    match fs::read_to_string(&schema_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state/history?limit=N
pub(super) async fn get_character_state_history(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .clamp(1, 1000);

    let history_path = data_dir::char_state_history_path(&state.data_root, char_id.as_str());
    let Ok(text) = fs::read_to_string(&history_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let entries: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(limit)
        .collect();

    match serde_json::to_string(&entries) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// ── Character card / lorebook CRUD（PR E：工作台编辑所需） ──────────────────

/// GET /v1/characters/:character_id — 返回角色卡 JSON 原文。
/// 优先读 `card/card.json`（迁移后路径），兼容旧 `card.json`。
pub(super) async fn get_character_card(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    Ok(Json(data_dir::get_character_card(&state.data_root, &cid)?))
}

/// DELETE /v1/characters/:character_id — 删除整个角色目录（card + state + sessions + ...）。
/// destructive：调用方负责确认。返回 {deleted: id}。
pub(super) async fn delete_character_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    ChatService::new(&state.data_root).delete_character(&cid)?;
    Ok(Json(serde_json::json!({
        "deleted": cid.as_str(),
        "status": "ok"
    })))
}

/// PUT /v1/characters/:character_id — 更新角色卡 JSON（整体替换）。
/// body 是 TavernCardV2 JSON；写回 `card/card.json` + `card/raw.json`。
/// 不重新解包资产（greetings/lorebook），如需重解请调 reextract。
///
/// 设计说明：raw.json 在导入时是"原始 imported 卡"的 sidecar（守 ASSET-SPEC
/// 规则2 存储永不丢）。本端点将其一并覆盖——把工作台编辑视为"新的规范化
/// 版本"，后续 reextract 会以编辑后的卡为源。如需保留原始 imported 卡，
/// 调用方应在 PUT 前自行备份（例如调 reextract 前再 PUT 一次原始内容）。
pub(super) async fn update_character_card(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Json(card): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    // 校验角色已存在（character_dir 会创建子目录，所以用 list_characters 校验）
    let exists = data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }
    let json_str = serde_json::to_string_pretty(&card)
        .map_err(|e| AirpError::BadRequest(format!("card JSON 序列化失败: {}", e)))?;
    let char_dir = data_dir::character_dir(&state.data_root, cid.as_str())?;
    let card_dir = char_dir.join("card");
    fs::create_dir_all(&card_dir)?;
    fs::write(card_dir.join("card.json"), &json_str)?;
    fs::write(card_dir.join("raw.json"), &json_str)?;
    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "status": "ok"
    })))
}

/// GET /v1/characters/:character_id/lorebook — 返回角色级世界书 JSON。
/// 不存在 → 404（与空对象 {} 区分）。
pub(super) async fn get_character_lorebook(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    // #67 #5 fix: 改用 Result<Json<Value>, AirpError> 统一错误格式。
    // 之前返回 Response + 裸 StatusCode::BAD_REQUEST，客户端 formatError 拿不到结构化 error body。
    let char_id = CharacterId::new(character_id)?;
    let lorebook = LorebookService::new(&state.data_root).read(&char_id)?;
    Ok(Json(serde_json::to_value(lorebook)?))
}

/// PUT /v1/characters/:character_id/lorebook — 更新角色级世界书（整体替换）。
/// body 是 `Lorebook` JSON：`{ entries: [{ keys, content, enabled?, priority?, comment? }] }`。
/// 角色不存在 → 404；写入前会校验 Lorebook 结构。
pub(super) async fn update_character_lorebook(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Json(body): Json<crate::orchestrator::Lorebook>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    // 校验角色已存在
    let exists = data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }
    LorebookService::new(&state.data_root).write(&cid, &body)?;
    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "entries_count": body.entries.len(),
        "status": "ok"
    })))
}

// ── Scene handlers ────────────────────────────────────────────────────────────

/// GET /v1/scenes — list all scene IDs.
pub(super) async fn list_scenes_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<Vec<String>>, AirpError> {
    let scenes = data_dir::list_scenes(&state.data_root)?;
    Ok(Json(scenes))
}

/// GET /v1/scenes/:scene_id — return scene.json for a scene.
///
/// AUDIT-2: scene_id is validated once via SceneId::new; downstream path
/// functions take &SceneId so traversal protection is compile-time enforced.
pub(super) async fn get_scene_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(scene_id): axum::extract::Path<String>,
) -> Response {
    let scene_id = match crate::types::SceneId::new(scene_id) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let path = data_dir::scene_json_path(&state.data_root, &scene_id);
    match fs::read_to_string(&path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// POST /v1/scenes — create or replace a scene from JSON body.
///
/// AUDIT-2: SceneConfig.scene_id is now a `SceneId`; serde Deserialize calls
/// `validate_id_segment` automatically, so a body with an invalid scene_id
/// is rejected at deserialize time (HTTP 400 returned by axum), and the
/// manual check below is no longer needed.
pub(super) async fn create_scene_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(scene): Json<crate::scene::SceneConfig>,
) -> Response {
    match scene.save(&state.data_root) {
        Ok(()) => {
            let path = data_dir::scene_json_path(&state.data_root, &scene.scene_id);
            (
                StatusCode::CREATED,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"scene_id": scene.scene_id, "path": path}).to_string(),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /v1/scenes/:scene_id/characters — add a character to an existing scene.
pub(super) async fn add_scene_character_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(scene_id): axum::extract::Path<String>,
    Json(body): Json<AddCharacterBody>,
) -> Response {
    let scene_id = match crate::types::SceneId::new(scene_id) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if data_dir::validate_id_segment(&body.character_id).is_err() {
        return (StatusCode::BAD_REQUEST, "非法 character_id").into_response();
    }
    let mut scene = match crate::scene::SceneConfig::load(&state.data_root, &scene_id) {
        Ok(s) => s,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    scene.characters.push(crate::scene::CharacterEntry {
        character_id: body.character_id,
        role: body.role,
        intro: body.intro,
    });
    match scene.save(&state.data_root) {
        Ok(()) => Json(serde_json::json!({"scene_id": scene_id.as_str(), "character_count": scene.characters.len()})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Models proxy ──────────────────────────────────────────────────────────────

/// GET /v1/models — proxy the upstream provider's /models endpoint.
pub(super) async fn list_models(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Response {
    let (endpoint, api_key) = {
        // #42 F-2：与 get_settings/update_settings 一致，poisoned lock 恢复而非 panic。
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        (cfg.endpoint.clone(), cfg.api_key.clone())
    };

    let models_url = match models_url_from_endpoint(&endpoint) {
        Some(url) => url,
        None => {
            let redacted = redact_endpoint_for_error(&endpoint);
            tracing::warn!(endpoint = %redacted, "models proxy: endpoint cannot be mapped to a /models URL");
            return models_proxy_error(
                StatusCode::BAD_GATEWAY,
                "invalid_endpoint",
                "provider endpoint cannot be mapped to a /models URL",
                None,
                None,
                Some(redacted),
            );
        }
    };

    let timeout = models_proxy_timeout();
    let mut req = state.http_client.get(&models_url).timeout(timeout);
    if let Some(key) = &api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
    }

    match req.send().await {
        Ok(resp) => {
            // #117 A：redirect 必须先于 success/non-success 分流判定，给 typed 脱敏文案。
            if let Some(classified) = crate::outbound::classify_redirect_response(&resp) {
                let upstream_status = match &classified {
                    AirpError::Upstream { status, .. } => *status,
                    _ => unreachable!("redirect classifier must return AirpError::Upstream"),
                };
                tracing::warn!(
                    upstream_status,
                    "models proxy: upstream redirected; outbound policy rejected"
                );
                return models_proxy_error(
                    StatusCode::BAD_GATEWAY,
                    "upstream_redirect_rejected",
                    format!(
                        "model provider /models redirected; outbound policy rejected to protect credentials: {}",
                        classified
                    ),
                    Some(upstream_status),
                    None,
                    None,
                );
            }
            let status = resp.status();
            match resp.bytes().await {
                Ok(body) if status.is_success() => (
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK),
                    [(header::CONTENT_TYPE, "application/json")],
                    body,
                )
                    .into_response(),
                Ok(body) => {
                    // #42 F-3：非 2xx 上游留痕，便于诊断（body 已截断脱敏后进响应）。
                    tracing::warn!(
                        upstream_status = status.as_u16(),
                        "models proxy: upstream returned non-success status"
                    );
                    models_proxy_error(
                        StatusCode::BAD_GATEWAY,
                        "upstream_status",
                        format!("model provider /models returned HTTP {}", status.as_u16()),
                        Some(status.as_u16()),
                        Some(truncate_error_text(&String::from_utf8_lossy(&body))),
                        None,
                    )
                }
                Err(e) => {
                    tracing::warn!(upstream_status = status.as_u16(), error = %e, "models proxy: failed to read upstream body");
                    models_proxy_error(
                        StatusCode::BAD_GATEWAY,
                        "upstream_body_read_failed",
                        "failed to read model provider /models response body",
                        Some(status.as_u16()),
                        None,
                        Some(e.to_string()),
                    )
                }
            }
        }
        Err(e) if e.is_timeout() => {
            tracing::warn!(
                timeout_ms = timeout.as_millis() as u64,
                "models proxy: upstream request timed out"
            );
            models_proxy_error(
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
                format!(
                    "model provider /models timed out after {}ms",
                    timeout.as_millis()
                ),
                None,
                None,
                None,
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "models proxy: upstream request failed");
            models_proxy_error(
                StatusCode::BAD_GATEWAY,
                "upstream_request_failed",
                "model provider /models request failed",
                None,
                None,
                None,
            )
        }
    }
}

/// 从 chat endpoint 推导 /models URL。
///
/// #42 F-1：改为基于 URL 解析推导，杜绝字符串 rfind('/') 在无路径 endpoint
/// （如 `http://example.com`）上命中 scheme 分隔符产生 `http://models` 之类
/// 丢失 host 的畸形 URL。规则：
/// - 非 http(s) 或无 host → None（走 invalid_endpoint 类型化错误）；
/// - 路径含 `/v1/` → 前缀 + `/v1/models`（OpenAI 兼容主路径）；
/// - 否则保守 fallback：把最后一个路径段替换为 `models`；无路径段则 None。
///
/// 推导结果一律剥离 query/fragment，避免把 endpoint 上的凭据带去 /models。
fn models_url_from_endpoint(endpoint: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(endpoint).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let path = url.path().to_string();
    let new_path = if let Some(pos) = path.find("/v1/") {
        format!("{}/v1/models", &path[..pos])
    } else {
        let trimmed = path.trim_end_matches('/');
        let pos = trimmed.rfind('/')?;
        if trimmed[pos + 1..].is_empty() {
            // 无有效路径段（如 "http://example.com" 或 "http://example.com/"）
            return None;
        }
        format!("{}/models", &trimmed[..pos])
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn redact_endpoint_for_error(endpoint: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(endpoint) {
        if !url.username().is_empty() {
            let _ = url.set_username("redacted");
        }
        if url.password().is_some() {
            let _ = url.set_password(Some("redacted"));
        }
        if url.query().is_some() {
            url.set_query(Some("redacted"));
        }
        // #40 建议 2：fragment 虽不发往服务端，但用户可能误把 secret 放在 # 后。
        if url.fragment().is_some() {
            url.set_fragment(Some("redacted"));
        }
        return url.to_string();
    }
    if let Some(pos) = endpoint.find(['?', '#']) {
        return format!("{}?redacted", &endpoint[..pos]);
    }
    endpoint.to_string()
}

#[derive(Debug, Serialize)]
struct ModelsProxyError {
    error: ModelsProxyErrorBody,
}

#[derive(Debug, Serialize)]
struct ModelsProxyErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

fn models_proxy_error(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
    upstream_status: Option<u16>,
    upstream_body: Option<String>,
    detail: Option<String>,
) -> Response {
    (
        status,
        Json(ModelsProxyError {
            error: ModelsProxyErrorBody {
                code,
                message: message.into(),
                upstream_status,
                upstream_body,
                detail,
            },
        }),
    )
        .into_response()
}

fn truncate_error_text(text: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 2048;
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= MAX_ERROR_BODY_CHARS {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CharacterId;
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvVarGuard {
        key: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let original = std::env::var_os(key);
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn with_local_path_env<F: FnOnce()>(enabled: bool, f: F) {
        let _lock = ENV_LOCK.blocking_lock();
        let _env = EnvVarGuard::set("AIRP_ALLOW_LOCAL_PATH", enabled.then_some("1"));
        f();
    }

    #[test]
    fn slugify_strips_invisible_controls() {
        let id = slugify_id("艾\u{200B}米\u{200F}丽\u{202E}");
        assert_eq!(id, "艾米丽");
        CharacterId::new(&id).unwrap();
    }

    #[test]
    fn slugify_normalizes_to_nfc() {
        let decomposed = slugify_id("Cafe\u{301}");
        let composed = slugify_id("Caf\u{00E9}");
        assert_eq!(decomposed, composed);
        assert_eq!(composed, "Caf\u{00E9}");
        CharacterId::new(&composed).unwrap();
    }

    #[test]
    fn slugify_truncates_long_names_on_utf8_boundary() {
        let id = slugify_id(&"角色".repeat(200));
        assert!(id.len() <= MAX_DERIVED_CHARACTER_ID_BYTES);
        assert!(std::str::from_utf8(id.as_bytes()).is_ok());
        CharacterId::new(&id).unwrap();
    }

    #[test]
    fn card_path_import_rejected_without_local_path_env() {
        with_local_path_env(false, || {
            let data_root = tempfile::tempdir().unwrap();
            let card_file = tempfile::NamedTempFile::new().unwrap();

            let result = import_card_to_disk(
                data_root.path(),
                Some("gate-test"),
                Some(card_file.path()),
                None,
                None,
            );

            assert!(
                matches!(result, Err(AirpError::BadRequest(msg)) if msg.contains("AIRP_ALLOW_LOCAL_PATH"))
            );
        });
    }

    #[test]
    fn card_path_import_allowed_with_local_path_env() {
        with_local_path_env(true, || {
            let data_root = tempfile::tempdir().unwrap();
            let card_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(
                card_file.path(),
                r#"{"spec":"chara_card_v2","data":{"name":"Gate Test","first_mes":"hi"}}"#,
            )
            .unwrap();

            let (character_id, card_format, json) = import_card_to_disk(
                data_root.path(),
                Some("gate-test"),
                Some(card_file.path()),
                None,
                None,
            )
            .unwrap();

            assert_eq!(character_id, "gate-test");
            assert_eq!(card_format, "json");
            assert!(json.contains("Gate Test"));
            assert!(data_root
                .path()
                .join("characters/gate-test/card/raw.json")
                .exists());
        });
    }

    // M3 RR-001 护栏 HTTP-level 回归测试
    //
    // 单测 `card_path_import_rejected_without_local_path_env` 只覆盖
    // `import_card_to_disk` 内部分支；此处验证完整 axum 路由链路：
    //   POST /v1/characters/import {card_path: "..."}  →  400 + 明确错误文案
    //
    // 守 RR-001：Web/browser 永远不能用 card_path 让 engine 读任意本地路径，
    // 即使持 bearer token、即使请求 body 形式合法。env 门控不可伪造
    // （进程启动时定，非请求头）。复用 ENV_LOCK 与 unit test 串行，避免 env race。
    //
    // Gemini Code Assist 建议抽 DaemonState 初始化样板：两个 m3_* 测试共用
    // `make_state_for_http_test`，避免 ~13 行重复。helper 返回 (state, _tmpguard)，
    // `_tmpguard` 持有 tempdir 防止目录被早回收。
    fn make_state_for_http_test() -> (Arc<super::super::DaemonState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(super::super::DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(super::super::MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://localhost".to_string(),
                api_key: None,
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });
        (state, tmp)
    }

    #[tokio::test]
    async fn m3_import_card_path_rejected_at_http_level() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let _lock = ENV_LOCK.lock().await;
        let _env = EnvVarGuard::set("AIRP_ALLOW_LOCAL_PATH", None);

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state.clone());
        let body = serde_json::json!({ "card_path": "/etc/passwd" });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/characters/import")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let s = String::from_utf8_lossy(&body_bytes);
        assert!(
            s.contains("AIRP_ALLOW_LOCAL_PATH"),
            "错误响应应明确提示 card_path 已被 env 门控禁用，got: {}",
            s
        );
    }

    // M3 happy-path HTTP 烟测：card_json 路径不被 env 门控影响，未设
    // AIRP_ALLOW_LOCAL_PATH 时仍可正常导入（确认护栏不影响合法路径）。
    #[tokio::test]
    async fn m3_import_card_json_works_without_local_path_env() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let _lock = ENV_LOCK.lock().await;
        let _env = EnvVarGuard::set("AIRP_ALLOW_LOCAL_PATH", None);

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state);
        let body = serde_json::json!({
            "card_json": r#"{"spec":"chara_card_v2","data":{"name":"Http M3 Test","first_mes":"hi"}}"#
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/characters/import")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        // slugify_id 不小写化，仅把空格 → '_'：`Http M3 Test` → `Http_M3_Test`
        assert_eq!(v["character_id"], "Http_M3_Test");
        assert_eq!(v["card_format"], "json");
    }

    // ── PR #74 W-01: get_character_lorebook HTTP-level 回归测试 ─────────────
    //
    // 守 #67 #5 修复：handler 改为 `Result<Json<Value>, AirpError>` 后，错误响应
    // 必须是 JSON envelope（`{"error":{"code","message"}}`），不能是裸 StatusCode。
    // 复用 make_state_for_http_test，3 个 case 覆盖主要分支。

    #[tokio::test]
    async fn pr74_lorebook_not_found_returns_json_envelope() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/does_not_exist/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["error"]["code"], "not_found");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("does_not_exist"),
            "错误 message 应含 character_id，got: {}",
            v["error"]["message"]
        );
    }

    #[tokio::test]
    async fn pr74_lorebook_invalid_character_id_returns_400_envelope() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state);
        // 含路径遍历字符 → CharacterId::new 校验失败 → BadRequest
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/..%2Fetc/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["error"]["code"], "bad_request");
    }

    #[tokio::test]
    async fn pr74_lorebook_happy_path_returns_json_value() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        // 在 data_root 下放一个合法 lorebook 文件
        let char_dir = tmp.path().join("characters").join("test_char");
        std::fs::create_dir_all(char_dir.join("world")).unwrap();
        std::fs::write(
            char_dir.join("world").join("lorebook.json"),
            r#"{"entries":[{"keys":["hi"],"content":"hello"}]}"#,
        )
        .unwrap();

        let app = super::super::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/test_char/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["entries"][0]["keys"][0], "hi");
        assert_eq!(v["entries"][0]["content"], "hello");
    }

    // ── #42 F-1 / #40：/v1/models URL 推导与 endpoint 脱敏 ──────────────────

    // W-01：HTTP-level 回归测试 — /v1/chat/history 返回 JSON 包含
    // message_timestamps 字段，且长度等于 messages。
    #[tokio::test]
    async fn pr75_chat_history_returns_message_timestamps() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("characters").join("ts_http_char")).unwrap();

        // 用 ChatLog API 写入 2 条消息（产生 ts）
        let mut log = crate::chat_store::ChatLog::new("ts_http_char");
        log.append(
            root,
            crate::adapter::ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "hello".to_string(),
            },
        )
        .unwrap();
        log.append(
            root,
            crate::adapter::ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content: "hi".to_string(),
            },
        )
        .unwrap();

        let app = super::super::create_router(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "character_id": "ts_http_char" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        // 无分页字段时必须保持 legacy ChatLog 响应形状。
        assert_eq!(v["character_id"], "ts_http_char");
        assert!(v["session_id"].is_string());
        // messages 数组长度 = 2
        assert_eq!(v["messages"].as_array().unwrap().len(), 2);
        assert_eq!(v["message_ids"].as_array().unwrap().len(), 2);
        // message_timestamps 字段存在且长度等于 messages
        let tss = v["message_timestamps"].as_array().unwrap();
        assert_eq!(tss.len(), 2, "message_timestamps 长度应等于 messages");
        // 每条都有 ts（非 null）
        assert!(tss[0].is_string(), "ts[0] 应为字符串");
        assert!(tss[1].is_string(), "ts[1] 应为字符串");

        // 显式 limit 才切换到分页窗口响应，并保留完整 total。
        let app = super::super::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "character_id": "ts_http_char",
                            "limit": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(page["messages"].as_array().unwrap().len(), 1);
        assert_eq!(page["total"], 2);
        assert_eq!(page["has_more"], true);
    }

    #[test]
    fn models_url_v1_endpoint_maps_to_v1_models() {
        assert_eq!(
            models_url_from_endpoint("https://api.example.com/v1/chat/completions"),
            Some("https://api.example.com/v1/models".to_string())
        );
    }

    #[test]
    fn models_url_no_path_endpoint_returns_none() {
        // #42 F-1：旧实现产生丢失 host 的 "http://models"，现在必须拒绝。
        assert_eq!(models_url_from_endpoint("http://example.com"), None);
        assert_eq!(models_url_from_endpoint("http://example.com/"), None);
    }

    #[test]
    fn models_url_non_http_scheme_returns_none() {
        assert_eq!(models_url_from_endpoint("file:///etc/passwd"), None);
        assert_eq!(models_url_from_endpoint("not-a-url"), None);
    }

    #[test]
    fn models_url_fallback_replaces_last_segment() {
        assert_eq!(
            models_url_from_endpoint("https://api.example.com/api/chat/completions"),
            Some("https://api.example.com/api/chat/models".to_string())
        );
    }

    #[test]
    fn models_url_strips_query_and_fragment() {
        assert_eq!(
            models_url_from_endpoint(
                "https://api.example.com/v1/chat/completions?api_key=secret#frag"
            ),
            Some("https://api.example.com/v1/models".to_string())
        );
    }

    #[test]
    fn redact_endpoint_clears_userinfo_password_query_fragment() {
        let redacted = redact_endpoint_for_error(
            "https://user:hunter2@api.example.com/v1/chat?api_key=secret#token=secret2",
        );
        assert!(!redacted.contains("hunter2"), "password leaked: {redacted}");
        assert!(!redacted.contains("user:"), "username leaked: {redacted}");
        assert!(
            !redacted.contains("secret"),
            "query/fragment leaked: {redacted}"
        );
        assert!(redacted.contains("api.example.com"));
    }

    #[test]
    fn redact_endpoint_unparseable_with_fragment() {
        assert_eq!(
            redact_endpoint_for_error("not-a-url#token=secret"),
            "not-a-url?redacted"
        );
    }

    // ── L1 修复（issue #89 B3 / #92 L1）：非法 card.json 校验测试覆盖 ───────
    //
    // PR #62 Gemini 建议写入前校验 TavernCardV2。本组测试覆盖非法 card.json 场景，
    // 断言现有行为：语法错误 / 预设误导入 / 缺 data.name 均被正确处理。

    /// 非法 JSON 语法 → BadRequest "不是有效 JSON"
    #[test]
    fn l1_invalid_card_json_syntax_rejected() {
        let data_root = tempfile::tempdir().unwrap();
        let result = import_card_to_disk(
            data_root.path(),
            Some("bad-syntax"),
            None,
            Some("{not valid json".to_string()),
            None,
        );
        assert!(
            matches!(&result, Err(AirpError::BadRequest(msg)) if msg.contains("不是有效 JSON")),
            "expected BadRequest with JSON syntax error, got: {:?}",
            result
        );
        // 不留脏文件
        assert!(!data_root.path().join("characters/bad-syntax").exists());
    }

    /// JSON 是预设（顶层 prompts[] + 模型参数）→ BadRequest "像 SillyTavern 预设"
    #[test]
    fn l1_preset_misimport_as_card_rejected() {
        let data_root = tempfile::tempdir().unwrap();
        let preset_json = serde_json::json!({
            "prompts": [{"name": "sys", "content": "be helpful"}],
            "model": "gpt-4o",
            "temperature": 0.7
        })
        .to_string();
        let result = import_card_to_disk(
            data_root.path(),
            Some("preset-as-card"),
            None,
            Some(preset_json),
            None,
        );
        assert!(
            matches!(&result, Err(AirpError::BadRequest(msg)) if msg.contains("预设")),
            "expected BadRequest rejecting preset-as-card, got: {:?}",
            result
        );
        assert!(!data_root.path().join("characters/preset-as-card").exists());
    }

    /// 合法 JSON 但缺 data.name → slugify 回退为 "character"，仍可导入（覆盖现有行为）
    #[test]
    fn l1_card_missing_data_name_falls_back_to_character_id() {
        let data_root = tempfile::tempdir().unwrap();
        // 无 data.name 的 v2 卡：spec 正确但缺 name 字段
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "",
                "description": "test",
                "first_mes": "hi"
            }
        })
        .to_string();
        // 传 None 触发 slugify 派生路径，由于 name 为空，应回退为 "character"
        let (id, _fmt, _json) =
            import_card_to_disk(data_root.path(), None, None, Some(json), None).unwrap();
        assert_eq!(id, "character");
        // 落盘成功
        assert!(data_root
            .path()
            .join("characters/character/card.json")
            .exists());
    }

    /// 合法 v2 卡 + 显式 character_id → 正常导入（happy path 覆盖）
    #[test]
    fn l1_valid_v2_card_imports() {
        let data_root = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "a knight",
                "first_mes": "Hello!"
            }
        })
        .to_string();
        let (id, fmt, _json) =
            import_card_to_disk(data_root.path(), Some("alice-test"), None, Some(json), None)
                .unwrap();
        assert_eq!(id, "alice-test");
        assert_eq!(fmt, "json");
        assert!(data_root
            .path()
            .join("characters/alice-test/card.json")
            .exists());
        assert!(data_root
            .path()
            .join("characters/alice-test/card/raw.json")
            .exists());
    }

    /// 三参数全 None → BadRequest "必须提供 ... 之一"
    #[test]
    fn l1_no_card_source_rejected() {
        let data_root = tempfile::tempdir().unwrap();
        let result = import_card_to_disk(data_root.path(), Some("none"), None, None, None);
        assert!(
            matches!(&result, Err(AirpError::BadRequest(msg)) if msg.contains("card_path") && msg.contains("card_json")),
            "expected BadRequest requiring one of card_path/card_json/card_png_base64, got: {:?}",
            result
        );
        // 不留脏文件（审计 CR4：与其他拒绝测试一致防御）
        assert!(!data_root.path().join("characters/none").exists());
    }

    // ── import_preset（#114）──────────────────────────────────────────────────────

    /// #114：合法 TavernPreset 导入应写盘到 presets/{id}/preset.json，且 prompts_count 正确。
    #[tokio::test]
    async fn import_preset_writes_preset_json_and_returns_count() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state.clone());

        let body = serde_json::json!({
            "preset_id": "myrp",
            "preset_json": r#"{"prompts":[{"identifier":"main","name":"Main","prompt":"hi","enabled":true}]}"#
        });
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["preset_id"], "myrp");
        assert_eq!(v["prompts_count"], 1);

        // 写盘：presets/myrp/preset.json 存在且可回解析
        let written = std::fs::read_to_string(
            state
                .data_root
                .join("presets")
                .join("myrp")
                .join("preset.json"),
        )
        .unwrap();
        let back: crate::orchestrator::TavernPreset = serde_json::from_str(&written).unwrap();
        assert_eq!(back.prompts.unwrap().len(), 1);

        let get = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/presets/myrp")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(get.into_body(), usize::MAX)
            .await
            .unwrap();
        let prompts: Vec<crate::orchestrator::TavernPrompt> =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            prompts.len(),
            1,
            "an imported preset must be immediately readable"
        );
    }

    #[tokio::test]
    async fn concurrent_imports_do_not_overwrite_the_same_preset() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state.clone());
        let request = |prompt: &'static str| {
            let body = serde_json::json!({
                "preset_id": "same-id",
                "preset_json": format!(r#"{{"prompts":[{{"identifier":"main","name":"Main","prompt":"{prompt}","enabled":true}}]}}"#)
            });
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/presets/import")
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };

        let (first, second) = tokio::join!(
            app.clone().oneshot(request("first")),
            app.oneshot(request("second"))
        );
        let statuses = [first.unwrap().status(), second.unwrap().status()];
        assert_eq!(
            statuses.iter().filter(|status| status.is_success()).count(),
            1
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == axum::http::StatusCode::BAD_REQUEST)
                .count(),
            1
        );

        let dir = state.data_root.join("presets/same-id");
        let written = std::fs::read_to_string(dir.join("preset.json")).unwrap();
        serde_json::from_str::<crate::orchestrator::TavernPreset>(&written).unwrap();
        assert!(!dir.join("preset.json.tmp").exists());
        assert!(!dir.join("preset.json.bak").exists());
    }

    #[tokio::test]
    async fn import_preset_rejects_legacy_duplicate_without_creating_directory() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let presets = state.data_root.join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::write(presets.join("legacy.json"), r#"{"prompts":[]}"#).unwrap();
        let app = super::super::create_router(state.clone());
        let body = serde_json::json!({
            "preset_id": "legacy",
            "preset_json": r#"{"prompts":[]}"#
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(!presets.join("legacy").exists());
    }

    /// #114：preset_id 路径遍历 → BadRequest，不写盘。
    #[tokio::test]
    async fn import_preset_rejects_traversal_preset_id() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state.clone());

        let body = serde_json::json!({
            "preset_id": "../evil",
            "preset_json": r#"{"prompts":[]}"#
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        // 关键：无 preset.json 落盘（路径遍历被 PresetId::new 拒，未进写盘分支）
        assert!(
            !state.data_root.join("presets").join("evil").exists(),
            "traversal preset_id must not write any preset file"
        );
    }

    /// #114：preset_json 非 TavernPreset 形状 → BadRequest，不写盘。
    #[tokio::test]
    async fn import_preset_rejects_invalid_preset_json() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state.clone());

        let body = serde_json::json!({
            "preset_id": "bad",
            "preset_json": "not json at all"
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(!state.data_root.join("presets").join("bad").exists());
    }
}
