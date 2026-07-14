//! M3: Chat Pipeline — three phases:
//!   `prepare` (validate + build prompt) → `stream` (FSM + unpack + SSE)
//!   → `finalize` (persist + volume side-effects).
//! FSM + Unpacker owned by stream task (no Arc/Mutex); oneshot channel to finalizer.

use axum::response::sse::Event;
use futures_util::{stream, Stream, StreamExt};
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::{
    call_streaming_api_auto, BackendEngine, ChatMessage, GenerationParams, ProviderConfig,
};
use crate::config::VolumeConfig;
use crate::daemon::{ChatCompletionRequest, DaemonState};
use crate::data_dir;
use crate::domain::{ChatService, Persona, PersonaService};
use crate::error::AirpError;
use crate::fsm::{RegexFilter, StreamingFsm};
use crate::orchestrator::{
    inject_current_context, inject_volume_context, Orchestrator, TavernPreset,
};
use crate::types::UserId;
use crate::xml_unpacker::{StreamingXmlUnpacker, UnpackedChunk};
use crate::{volume_manager, volume_store};

// ── Prepared pipeline ─────────────────────────────────────────────────────────

/// Everything needed to start streaming a response.
///
/// M4.2：连接层配置（`provider_config`）用 `Arc` 共享给 stream 与 finalizer 任务，
/// 消除原 `AdapterConfig` 在 prepare_pipeline 末尾的双重 clone。
pub struct PreparedPipeline {
    /// 连接层配置（端点 / api_key / provider），多任务共享。
    pub provider_config: Arc<ProviderConfig>,
    /// 生成参数（model / temperature / max_tokens）。
    pub gen_params: GenerationParams,
    /// 完整组装好的 system prompt。
    pub system_prompt: String,
    /// 历史消息 + 当前用户消息列表。
    pub messages: Vec<ChatMessage>,
    /// 流过滤 FSM 实例。
    pub fsm: StreamingFsm,
    /// XML 标签拆包器实例。
    pub unpacker: StreamingXmlUnpacker,
    /// finalize 阶段所需上下文。
    pub finalizer: FinalizerCtx,
    /// M0 F-01：复用 daemon 持有的 reqwest 连接池。
    pub http_client: reqwest::Client,
    /// DX-6：后端引擎（Direct / AnthropicMessages / ClaudeCodeSdk）。
    pub engine: BackendEngine,
}

/// Context passed to the finalizer task (run after the stream ends).
pub struct FinalizerCtx {
    /// 角色 ID；为 `None` 时跳过 ChatLog 持久化。
    pub character_id: Option<crate::types::CharacterId>,
    /// Named session scope; `None` keeps the legacy per-character log.
    pub session_id: Option<crate::types::SessionId>,
    /// 数据根目录。
    pub data_root: PathBuf,
    /// 卷系统 session 目录；为 `None` 时跳过卷副作用。
    pub session_dir: Option<PathBuf>,
    /// 共享连接层配置（与 `PreparedPipeline.provider_config` 同源）。
    pub provider_config: Arc<ProviderConfig>,
    /// 生成参数；封卷会派生新参数（覆盖 model / temperature）。
    pub gen_params: GenerationParams,
    /// 卷系统运行参数（阈值 / 维护间隔等）。
    pub volume_config: VolumeConfig,
    /// M0 F-01：封卷任务需要再次发起 HTTP 调用，仍复用同一连接池。
    pub http_client: reqwest::Client,
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// MS-6: Try to load a character card JSON from the standard locations.
/// Priority: card/raw.json > card.json > card.png
fn load_char_card_json(root: &std::path::Path, character_id: &str) -> Option<String> {
    let char_dir = root.join("characters").join(character_id);
    if char_dir.join("card").join("raw.json").exists() {
        fs::read_to_string(char_dir.join("card").join("raw.json"))
            .ok()
            .map(|raw| data_dir::strip_utf8_bom(&raw).to_owned())
    } else if char_dir.join("card.json").exists() {
        fs::read_to_string(char_dir.join("card.json"))
            .ok()
            .map(|raw| data_dir::strip_utf8_bom(&raw).to_owned())
    } else if char_dir.join("card.png").exists() {
        crate::png_parser::parse_png_character_card(
            char_dir.join("card.png").to_str().unwrap_or(""),
        )
        .ok()
    } else {
        None
    }
}

/// A1b：按 precedence contract 解析请求激活的 Persona。
///
/// `data_root` 是**全局** data root（`state.data_root`），不是 user-scoped
/// effective root；`PersonaService` 内部用 `user_dir(root, uid)` 拼
/// `users/{uid}/personas/` 路径，若传 user-scoped root 会双重嵌套。
///
/// 返回 `None` 表示请求未带 `user_id`（保持单用户向后兼容）；否则按以下顺序
/// 返回 Persona：
///
/// 1. **显式 `persona_id`**（请求体指定）：id 不存在时 `find_for_character` /
///    `get` 已在 `PersonaService` 内返 `NotFound`，与本契约一致。`default`
///    大小写不敏感（service 内 canonicalize）。
/// 2. **绑定查找**（仅单角色分支；`scene_id` 缺省且 `character_id` 存在）：
///    `PersonaService::find_for_character` 先精确匹配 session 绑定，再匹配
///    该角色下的 generic 绑定。命中后用 `get` 读出 Persona。
/// 3. **默认 persona**：`get_default` 返回存储的 default；未写盘时返回
///    `Persona::initial` 内存快照（不触发隐式落盘）。
///
/// Scene 模式（`scene_id` 存在）只走 precedence 1 与 3：scene 有多角色，
/// 没有单一绑定目标，`find_for_character` 跳过；多角色 persona 绑定语义延后。
fn resolve_request_persona(
    payload: &ChatCompletionRequest,
    data_root: &std::path::Path,
) -> Result<Option<Persona>, AirpError> {
    let Some(user_id_str) = payload.user_id.as_deref() else {
        return Ok(None);
    };
    let uid = UserId::new(user_id_str)?;
    let service = PersonaService::new(data_root);

    // precedence 1: 显式 persona_id
    if let Some(persona_id) = payload.persona_id.as_deref() {
        data_dir::validate_id_segment(persona_id)?;
        let persona = service.get(&uid, persona_id, "User")?;
        return Ok(Some(persona));
    }

    // precedence 2: find_for_character（仅单角色分支）
    if payload.scene_id.is_none() {
        if let Some(ref cid) = payload.character_id {
            // SessionId 是 newtype(uuid::Uuid)；find_for_character 需要
            // Option<&str>。在本地构造 String 再借，避免给 SessionId 强加 Deref。
            let session_id_str = payload.session_id.as_ref().map(|s| s.to_string());
            if let Some(pid) =
                service.find_for_character(&uid, cid.as_str(), session_id_str.as_deref())?
            {
                let persona = service.get(&uid, &pid, "User")?;
                return Ok(Some(persona));
            }
        }
    }

    // precedence 3: default persona
    Ok(Some(service.get_default(&uid, "User")?))
}

/// A1b：把解析到的 Persona 与请求体 `user_profile` 合并为有效的 (name, variables)。
///
/// 合同：
/// - `name`：请求体 `user_profile.name` 非空时优先；否则用 persona `name`。
///   这样客户端可以显式传当前用户显示名（覆盖 persona），或者发空串让
///   persona 的 `{{user}}` 默认值生效（A2 将采用此约定）。
/// - `variables`：persona `variables` 作为底层 defaults；请求体
///   `user_profile.variables` 同名键覆盖。这样 persona 提供持久化的 tone 等
///   persona 级变量，客户端临时 override 不破坏存储。
///
/// `persona == None` 时原样返回 `user_profile`（user_id 缺失路径，向后兼容）。
fn merge_persona_into_user_profile(
    user_profile: &crate::daemon::types::UserProfile,
    persona: Option<&Persona>,
) -> (String, std::collections::HashMap<String, String>) {
    let Some(persona) = persona else {
        return (user_profile.name.clone(), user_profile.variables.clone());
    };
    let name = if user_profile.name.is_empty() {
        persona.name.clone()
    } else {
        user_profile.name.clone()
    };
    let mut merged = persona.variables.clone();
    merged.extend(user_profile.variables.clone());
    (name, merged)
}

/// issue #27：组装流式过滤器集合，single 与 scene 分支复用同一份加载逻辑。
///
/// 顺序固定，确保两分支产出**一致**的过滤器集合：
///   1. 请求体自带的 `regex_filters`；
///   2. PR-4：预设关联的 SillyTavern 正则脚本（仅在指定 `preset_id` 时加载，
///      内部筛选 AI Output + 空 replaceString 的「隐藏」类脚本）；
///   3. 内置隐藏段 `<卷评估…/>` 与 `<state>…</state>`。
///
/// 抽出此函数以消除原 single / scene 两分支的不对称：scene 分支此前漏掉第 2 步
/// （PR-4 预设正则），导致同一 preset 在 scene / 群聊模式下本应隐藏的
/// thought / status 段泄露到输出。
fn assemble_regex_filters(
    payload: &ChatCompletionRequest,
    effective_root: &std::path::Path,
) -> Vec<RegexFilter> {
    let mut filters: Vec<RegexFilter> = payload
        .regex_filters
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|r| RegexFilter::from_regex(&r))
        .collect();
    // PR-4: 预设关联的 SillyTavern 正则脚本（仅 AI Output + 空 replaceString）
    if let Some(ref pid) = payload.preset_id {
        match crate::preset_regex::load_preset_regex_scripts(effective_root, pid.as_str()) {
            Ok(scripts) => {
                let preset_filters = crate::preset_regex::scripts_to_filters(&scripts);
                if !preset_filters.is_empty() {
                    tracing::debug!(
                        preset = pid.as_str(),
                        count = preset_filters.len(),
                        "PR-4: 注入预设关联正则过滤器"
                    );
                    filters.extend(preset_filters);
                }
            }
            Err(e) => tracing::warn!(err = %e, "PR-4: 加载预设正则脚本失败"),
        }
    }
    filters.push(RegexFilter::from_regex("<卷评估[\\s\\S]*?/>"));
    // M_LS-2: strip <state>…</state> during streaming so users never see raw state tokens
    filters.push(RegexFilter::from_regex("<state>[\\s\\S]*?</state>"));
    filters
}

// ── prepare (scene branch) ────────────────────────────────────────────────────

/// MS-6: Scene pipeline — loads SceneConfig, all character cards, merges lorebooks,
/// builds a multi-character system prompt, and routes session_dir to scene memory/.
///
/// AUDIT-2: validates scene_id into SceneId once on entry; downstream functions
/// receive `&SceneId` so they cannot accidentally bypass validation.
fn prepare_scene_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
    scene_id: &str,
) -> Result<PreparedPipeline, AirpError> {
    let scene_id = crate::types::SceneId::new(scene_id)
        .map_err(|e| AirpError::BadRequest(format!("非法 scene_id: {}", e)))?;

    // DX-1: per-user data root isolation
    let effective_root =
        data_dir::resolve_effective_root(&state.data_root, payload.user_id.as_deref())?;

    let scene = crate::scene::SceneConfig::load(&effective_root, &scene_id)?;

    // Load card JSONs + per-character lorebooks
    let mut card_jsons: Vec<(String, Option<String>)> = Vec::new();
    let mut lorebooks: Vec<crate::orchestrator::Lorebook> = Vec::new();

    for entry in &scene.characters {
        let card_json = load_char_card_json(&effective_root, &entry.character_id);
        card_jsons.push((entry.character_id.clone(), card_json));

        let include_lb = match scene.lorebook_merge {
            crate::scene::LorebookMerge::Union => true,
            crate::scene::LorebookMerge::PrimaryOnly => {
                entry.role == crate::scene::CharacterRole::Primary
            }
        };
        if include_lb {
            let lb_path = data_dir::char_world_lorebook_path(&effective_root, &entry.character_id);
            if lb_path.exists() {
                if let Ok(raw) = fs::read_to_string(&lb_path) {
                    let cleaned = data_dir::strip_utf8_bom(&raw);
                    if let Ok(lb) = serde_json::from_str::<crate::orchestrator::Lorebook>(cleaned) {
                        lorebooks.push(lb);
                    }
                }
            }
        }
    }

    // Scene-level lorebook (always included regardless of merge mode)
    let scene_lb_path = data_dir::scene_world_lorebook_path(&effective_root, &scene_id);
    if scene_lb_path.exists() {
        if let Ok(raw) = fs::read_to_string(&scene_lb_path) {
            let cleaned = data_dir::strip_utf8_bom(&raw);
            if let Ok(lb) = serde_json::from_str::<crate::orchestrator::Lorebook>(cleaned) {
                lorebooks.push(lb);
            }
        }
    }

    let merged_lb = crate::orchestrator::merge_lorebooks(&lorebooks);

    // Scan message + history for lorebook triggers
    let mut scan_text = payload.message.clone();
    if let Some(ref history) = payload.messages_history {
        for h in history {
            scan_text.push(' ');
            scan_text.push_str(&h.content);
        }
    }
    let triggered_lore = merged_lb.trigger(&scan_text);

    let cards: Vec<(&str, Option<&str>)> = card_jsons
        .iter()
        .map(|(id, json)| (id.as_str(), json.as_deref()))
        .collect();

    // A1b: scene 模式只走 precedence 1（显式 persona_id）与 3（default）；
    //      `find_for_character` 跳过，因为 scene 有多角色，没有单一绑定目标。
    //      与 single 分支一致，传 `state.data_root`（全局 root）。
    let request_persona = resolve_request_persona(payload, &state.data_root)?;
    let (effective_user_name, effective_user_variables) =
        merge_persona_into_user_profile(&payload.user_profile, request_persona.as_ref());

    let mut system_prompt = crate::orchestrator::build_multi_char_system_prompt(
        &scene,
        &cards,
        &triggered_lore,
        &effective_user_name,
    );
    let mut prompt_variables = effective_user_variables.clone();
    prompt_variables.insert("user".to_string(), effective_user_name.clone());
    for (key, value) in prompt_variables {
        system_prompt = system_prompt.replace(&format!("{{{{{key}}}}}"), &value);
    }

    let session_dir_opt: Option<PathBuf> =
        data_dir::scene_memory_dir(&effective_root, &scene_id).ok();

    let snapshot = {
        let cfg = state
            .config
            .read()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        cfg.clone()
    };

    if let Some(ref sd) = session_dir_opt {
        inject_current_context(sd, &mut system_prompt);
        inject_volume_context(sd, &payload.message, &mut system_prompt);
        if let Some(hint) = volume_manager::soft_pressure_hint(
            sd,
            snapshot.volume_config.soft_threshold_tokens,
            snapshot.volume_config.hard_threshold_tokens,
        ) {
            system_prompt.push_str(&hint);
        }
    }

    let preset_json: Option<String> = payload.preset_id.as_ref().and_then(|pid| {
        let new_path = data_dir::preset_json_path(&effective_root, pid.as_str());
        let legacy_path = effective_root
            .join("presets")
            .join(format!("{}.json", pid.as_str()));
        let p = if new_path.exists() {
            new_path
        } else {
            legacy_path
        };
        fs::read_to_string(&p)
            .ok()
            .map(|raw| data_dir::strip_utf8_bom(&raw).to_owned())
    });
    let preset_params: Option<TavernPreset> = preset_json
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok());

    let provider_config = Arc::new(ProviderConfig {
        provider: payload
            .provider
            .clone()
            .unwrap_or_else(|| snapshot.provider.clone()),
        endpoint: payload
            .endpoint
            .clone()
            .unwrap_or_else(|| snapshot.endpoint.clone()),
        api_key: payload.api_key.clone().or_else(|| snapshot.api_key.clone()),
    });
    let gen_params = GenerationParams {
        model: payload
            .model
            .clone()
            .or_else(|| preset_params.as_ref().and_then(|p| p.model.clone()))
            .unwrap_or_else(|| snapshot.model.clone()),
        temperature: payload
            .temperature
            .or_else(|| preset_params.as_ref().and_then(|p| p.temperature)),
        max_tokens: payload
            .max_tokens
            .or_else(|| preset_params.as_ref().and_then(|p| p.max_tokens)),
    };

    let messages = {
        let mut list = payload.messages_history.clone().unwrap_or_default();
        list.push(ChatMessage {
            role: crate::adapter::MessageRole::User,
            content: payload.message.clone(),
        });
        list
    };

    // issue #27：复用共享过滤器组装（含 PR-4 预设正则），与 single 分支产出一致集合。
    let filters = assemble_regex_filters(payload, &effective_root);

    let runtime_variables = effective_user_variables.clone();
    let fsm = StreamingFsm::new(filters, runtime_variables);

    Ok(PreparedPipeline {
        provider_config: provider_config.clone(),
        gen_params: gen_params.clone(),
        system_prompt,
        messages,
        fsm,
        unpacker: StreamingXmlUnpacker::new(),
        engine: snapshot.engine.clone(),
        finalizer: FinalizerCtx {
            character_id: None,
            session_id: None,
            data_root: effective_root.clone(),
            session_dir: session_dir_opt,
            provider_config,
            gen_params,
            volume_config: snapshot.volume_config.clone(),
            http_client: state.http_client.clone(),
        },
        http_client: state.http_client.clone(),
    })
}

// ── prepare ───────────────────────────────────────────────────────────────────

/// Validates the request, loads all required files, builds the system prompt,
/// persists the user message, and returns a ready-to-stream pipeline.
pub fn prepare_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
) -> Result<PreparedPipeline, AirpError> {
    // MS-6: scene branch — scene_id takes precedence over character_id
    if let Some(ref sid) = payload.scene_id {
        return prepare_scene_pipeline(payload, state, sid);
    }

    // DX-1: per-user data root isolation
    let effective_root =
        data_dir::resolve_effective_root(&state.data_root, payload.user_id.as_deref())?;

    // 1. ID validation：M5.0a — CharacterId / PresetId 在反序列化时已校验，
    //    此处不再需要显式 validate_id_segment 调用。

    // 2. Load character card
    let card_json: Option<String> = if let Some(ref card_id) = payload.character_card_id {
        let trimmed = card_id.trim();
        if trimmed.starts_with('{') {
            Some(card_id.clone())
        } else {
            let resolved = data_dir::safe_resolve_under_data_root(&effective_root, card_id)?;
            if resolved
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
            {
                crate::png_parser::parse_png_character_card(resolved.to_str().unwrap_or(""))
                    .map(Some)?
            } else {
                Some(fs::read_to_string(&resolved)?)
            }
        }
    } else {
        None
    };

    // 3. Load lorebook
    //    M4.5: `lorebook_path` 以 `{` 开头视为内联 JSON 字符串，跳过路径解析。
    //    CF-8: `lorebook_path` 为 None 且有 character_id 时，自动发现
    //    `world/lorebook.json`（CF-7 导入时写入），消除手动填写路径的需求（STR-02）。
    let lore_json: Option<String> = if let Some(ref lb_path) = payload.lorebook_path {
        if lb_path.trim_start().starts_with('{') {
            Some(lb_path.clone())
        } else {
            let resolved = data_dir::safe_resolve_under_data_root(&effective_root, lb_path)?;
            let raw = fs::read_to_string(&resolved)?;
            // STR-01: PowerShell 写 JSON 常带 UTF-8 BOM，serde_json 拒绝；提前剥除
            Some(data_dir::strip_utf8_bom(&raw).to_owned())
        }
    } else if let Some(ref cid) = payload.character_id {
        // CF-8: 自动发现 world/lorebook.json（由 CF-7 导入时写入）
        let auto_lb = data_dir::char_world_lorebook_path(&effective_root, cid.as_str());
        if auto_lb.exists() {
            match fs::read_to_string(&auto_lb) {
                Ok(raw) => {
                    tracing::debug!(path = ?auto_lb, "CF-8: 自动加载 world/lorebook.json");
                    Some(data_dir::strip_utf8_bom(&raw).to_owned())
                }
                Err(e) => {
                    tracing::warn!(err = %e, "CF-8: 读取 world/lorebook.json 失败，跳过");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // 4. Build orchestrator
    let orchestrator = Orchestrator::new(card_json.as_deref(), lore_json.as_deref())?;

    // 5. Advance timeline (side-effect, best-effort)
    if let Some(ref cid) = payload.character_id {
        Orchestrator::advance_timeline_and_checkpoint(&effective_root, cid.as_str());
    }

    // R-04: Pre-load chat history when client omits messages_history.
    // Loaded BEFORE step 7 so it excludes the current user message;
    // step 12 appends it explicitly. Reused for both lorebook scan and context.
    let auto_history: Option<Vec<ChatMessage>> = if payload.messages_history.is_none() {
        payload.character_id.as_ref().and_then(|cid| {
            ChatService::new(&effective_root)
                .recent(cid, payload.session_id.as_ref(), 50)
                .ok()
        })
    } else {
        None
    };

    // 6. Trigger lorebook
    let mut scan_text = payload.message.clone();
    let history_for_scan = payload.messages_history.as_ref().or(auto_history.as_ref());
    if let Some(history) = history_for_scan {
        for h in history {
            scan_text.push(' ');
            scan_text.push_str(&h.content);
        }
    }
    let triggered_lore = orchestrator.trigger_lorebook(&scan_text);

    // 7. Persist user message (early-write; survives stream failures)
    if let Some(ref cid) = payload.character_id {
        if let Err(e) = ChatService::new(&effective_root).append(
            cid,
            payload.session_id.as_ref(),
            ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: payload.message.clone(),
            },
        ) {
            tracing::warn!(err = %e, "无法持久化 user 消息");
        }
    }

    // 8. Load preset JSON + extract top-level API params (P-08)
    //    PR-3: 优先读 `presets/{pid}/preset.json`（M_PR 目录形态），
    //    降级读旧扁平 `presets/{pid}.json`（兜底用户未触发启动迁移的边缘场景）。
    let preset_json: Option<String> = payload.preset_id.as_ref().and_then(|pid| {
        let new_path = data_dir::preset_json_path(&effective_root, pid.as_str());
        let legacy_path = effective_root
            .join("presets")
            .join(format!("{}.json", pid.as_str()));
        let p = if new_path.exists() {
            new_path
        } else {
            legacy_path
        };
        fs::read_to_string(&p)
            .ok()
            // STR-01: 剥除可能由 Windows 工具写入的 UTF-8 BOM
            .map(|raw| data_dir::strip_utf8_bom(&raw).to_owned())
    });
    // Parse once for top-level params; build_system_prompt_with_preset re-parses internally.
    let preset_params: Option<TavernPreset> = preset_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());

    // 9. Build system prompt
    let cid_str: Option<&str> = payload.character_id.as_ref().map(|c| c.as_str());
    // A1b: resolve persona (explicit > bound > default) and merge with the
    //      request user_profile. Request fields override persona defaults so
    //      legacy callers see no behavior change. `state.data_root` (not the
    //      user-scoped effective_root) is passed because PersonaService builds
    //      `users/{uid}/personas/` from the global root.
    let request_persona = resolve_request_persona(payload, &state.data_root)?;
    let (effective_user_name, effective_user_variables) =
        merge_persona_into_user_profile(&payload.user_profile, request_persona.as_ref());
    let mut system_prompt = orchestrator.build_system_prompt_with_preset(
        &effective_root,
        cid_str,
        &effective_user_name,
        &effective_user_variables,
        &triggered_lore,
        preset_json.as_deref(),
        payload.enabled_presets.as_ref(),
        &payload.message,
    );

    // 10. Volume context injection
    // M5.1：若客户端指定 session_id 走 sessions/{uuid}/ 路径，否则保持 legacy session/。
    let session_dir_opt: Option<PathBuf> = payload.character_id.as_ref().and_then(|id| {
        data_dir::resolve_session_dir(&effective_root, id.as_str(), payload.session_id.as_ref())
            .ok()
    });

    // M4.4：一次性快照 daemon 当前热重载配置，后续读取本地变量；
    // 锁 RAII 出 scope 立即释放，避免长时间持锁。
    let snapshot = {
        let cfg = state
            .config
            .read()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        cfg.clone()
    };

    if let Some(ref sd) = session_dir_opt {
        inject_current_context(sd, &mut system_prompt);
        inject_volume_context(sd, &payload.message, &mut system_prompt);
        if let Some(hint) = volume_manager::soft_pressure_hint(
            sd,
            snapshot.volume_config.soft_threshold_tokens,
            snapshot.volume_config.hard_threshold_tokens,
        ) {
            system_prompt.push_str(&hint);
        }
    }

    // 11. API config（M4.2：拆为 provider 与 gen_params 两段）
    // Priority: request body > preset top-level > daemon defaults
    let preset_temperature = preset_params.as_ref().and_then(|p| p.temperature);
    let preset_max_tokens = preset_params.as_ref().and_then(|p| p.max_tokens);
    let preset_model = preset_params.as_ref().and_then(|p| p.model.clone());
    let provider_config = Arc::new(ProviderConfig {
        provider: payload
            .provider
            .clone()
            .unwrap_or_else(|| snapshot.provider.clone()),
        endpoint: payload
            .endpoint
            .clone()
            .unwrap_or_else(|| snapshot.endpoint.clone()),
        api_key: payload.api_key.clone().or_else(|| snapshot.api_key.clone()),
    });
    let gen_params = GenerationParams {
        model: payload
            .model
            .clone()
            .or(preset_model)
            .unwrap_or_else(|| snapshot.model.clone()),
        temperature: payload.temperature.or(preset_temperature),
        max_tokens: payload.max_tokens.or(preset_max_tokens),
    };

    // 12. Build message list
    // When client omits messages_history, fall back to auto_history (loaded before step 7).
    // auto_history does NOT include the current user message yet, so we append it here.
    let messages = {
        let mut list = payload
            .messages_history
            .clone()
            .or(auto_history)
            .unwrap_or_default();
        list.push(ChatMessage {
            role: crate::adapter::MessageRole::User,
            content: payload.message.clone(),
        });
        list
    };

    // 13. Build FSM
    // issue #27：复用共享过滤器组装（含 PR-4 预设正则），与 scene 分支产出一致集合。
    let filters = assemble_regex_filters(payload, &effective_root);

    let mut runtime_variables = effective_user_variables.clone();
    runtime_variables.insert("user".to_string(), effective_user_name.clone());
    if let Some(ref card) = orchestrator.card {
        if let Some(ref name) = card.name {
            runtime_variables.insert("char".to_string(), name.clone());
        }
    }
    let fsm = StreamingFsm::new(filters, runtime_variables);

    Ok(PreparedPipeline {
        provider_config: provider_config.clone(),
        gen_params: gen_params.clone(),
        system_prompt,
        messages,
        fsm,
        unpacker: StreamingXmlUnpacker::new(),
        engine: snapshot.engine.clone(),
        finalizer: FinalizerCtx {
            character_id: payload.character_id.clone(),
            session_id: payload.session_id,
            data_root: effective_root.clone(),
            session_dir: session_dir_opt,
            provider_config,
            gen_params,
            volume_config: snapshot.volume_config.clone(),
            http_client: state.http_client.clone(),
        },
        http_client: state.http_client.clone(),
    })
}

// ── stream ────────────────────────────────────────────────────────────────────

/// Converts a `PreparedPipeline` into an SSE event stream.
///
/// Architecture (M3.2 – no Arc/Mutex on hot path):
///   - Spawns a single **processing task** that owns FSM + Unpacker.
///   - Processing task drives the raw API stream, sends `UnpackedChunk` batches
///     via a bounded mpsc channel.
///   - On normal end OR cancellation, sends accumulated text to the **finalizer**
///     via a oneshot channel.
///   - Spawns a **finalizer task** that persists ChatLog + volume side-effects.
///   - The SSE response polls the mpsc receiver (no mutex needed).
pub fn build_sse_stream(
    pipeline: PreparedPipeline,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let PreparedPipeline {
        provider_config,
        gen_params,
        system_prompt,
        messages,
        fsm,
        unpacker,
        finalizer,
        http_client,
        engine,
    } = pipeline;

    let raw_stream = call_streaming_api_auto(
        &engine,
        http_client,
        provider_config,
        gen_params,
        system_prompt,
        messages,
    );

    // chunk_tx: processing task → SSE layer
    // acc_tx:   processing task → finalizer task
    let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel::<Result<Vec<UnpackedChunk>, String>>(32);
    let (acc_tx, acc_rx) = tokio::sync::oneshot::channel::<(String, String)>();

    // ── Processing task ───────────────────────────────────────────────────────
    tokio::spawn(async move {
        let mut fsm = fsm;
        let mut unpacker = unpacker;
        let mut raw_acc = String::new();
        let mut cleaned_acc = String::new();
        let mut cancelled = false;

        tokio::pin!(raw_stream);
        while let Some(item) = raw_stream.next().await {
            match item {
                Ok(token) => {
                    raw_acc.push_str(&token);
                    let cleaned = fsm.process_chunk(&token);
                    cleaned_acc.push_str(&cleaned);
                    let chunks = unpacker.process_chunk(&cleaned);
                    if chunk_tx.send(Ok(chunks)).await.is_err() {
                        // Receiver dropped → client disconnected
                        cancelled = true;
                        break;
                    }
                }
                Err(e) => {
                    // API error; push error event then stop
                    let _ = chunk_tx.send(Err(e)).await;
                    break;
                }
            }
        }

        if !cancelled {
            // Normal end: flush FSM tail + unpacker
            let tail = fsm.finish();
            cleaned_acc.push_str(&tail);
            let mut final_chunks = unpacker.process_chunk(&tail);
            final_chunks.extend(unpacker.finish());
            if !final_chunks.is_empty() {
                let _ = chunk_tx.send(Ok(final_chunks)).await;
            }
        }

        // Always send accumulators (partial if cancelled, complete if normal)
        let _ = acc_tx.send((raw_acc, cleaned_acc));
    });

    // ── Finalizer task ────────────────────────────────────────────────────────
    tokio::spawn(async move {
        let (raw_acc, cleaned_acc) = match acc_rx.await {
            Ok(pair) => pair,
            Err(_) => {
                tracing::debug!("processing task dropped without sending accumulators");
                (String::new(), String::new())
            }
        };
        run_finalize(finalizer, raw_acc, cleaned_acc).await;
    });

    // ── SSE stream: mpsc receiver → Event items ───────────────────────────────
    stream::unfold(chunk_rx, |mut rx| async move {
        rx.recv().await.map(|result| {
            let events = chunks_result_to_events(result);
            (events, rx)
        })
    })
    .flat_map(stream::iter)
}

// ── finalize ──────────────────────────────────────────────────────────────────

async fn run_finalize(ctx: FinalizerCtx, raw_acc: String, cleaned_acc: String) {
    // A2-1: credit estimated LLM output tokens toward the per-(user)-root daily
    // quota. `ctx.data_root` is the effective root (DX-1 per-user isolation), so
    // record_tokens writes the same quota.json that check_and_increment gated on.
    // raw_acc = full raw generation (pre-filter), the truest proxy for billed
    // output. Best-effort: record_tokens never blocks a completed response.
    let out_tokens = crate::volume_store::estimate_tokens(&raw_acc);
    crate::quota::record_tokens(&ctx.data_root, out_tokens.min(u32::MAX as usize) as u32);

    // (1) Persist assistant message to ChatLog
    //     M_LS-1: strip <state>…</state> before persisting; side-persist state/live.json.
    if let Some(ref cid) = ctx.character_id {
        let (stripped, live_state) = extract_state_content(&cleaned_acc);
        if let Some(ref state) = live_state {
            persist_live_state(&ctx.data_root, cid.as_str(), state).await;
        }
        if !stripped.trim().is_empty() {
            if let Err(e) = ChatService::new(&ctx.data_root).append(
                cid,
                ctx.session_id.as_ref(),
                ChatMessage {
                    role: crate::adapter::MessageRole::Assistant,
                    content: stripped,
                },
            ) {
                tracing::warn!(err = %e, "持久化 assistant 消息失败");
            }
        }
    }

    // (2) Volume side-effects
    if let Some(sd) = ctx.session_dir {
        let (cleaned, signal) = volume_manager::parse_seal_signal(&raw_acc);

        if !cleaned.trim().is_empty() {
            let _ = volume_store::append_to_current(&sd, &cleaned);
        }

        let should_seal = signal.as_ref().map(|s| s.should_seal).unwrap_or(false)
            || volume_manager::should_force_seal(&sd, ctx.volume_config.hard_threshold_tokens);

        // JoinSet 结构化管理：封卷 + 维护子任务，finalize 等待两者完成。
        let mut join_set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

        if should_seal {
            let sd_clone = sd.clone();
            // M4.2：封卷派生新 gen_params（覆盖 temperature / 可选 model）；
            // provider_config 直接复用同一 Arc，连接层不变。
            let mut seal_params = ctx.gen_params.clone();
            seal_params.temperature = Some(ctx.volume_config.seal_temperature);
            if let Some(model_override) = ctx.volume_config.seal_model.clone() {
                seal_params.model = model_override;
            }
            let seal_provider = ctx.provider_config.clone();
            let seal_client = ctx.http_client.clone();
            join_set.spawn(async move {
                if let Err(e) = volume_manager::run_seal_flow(
                    &seal_client,
                    &sd_clone,
                    seal_provider,
                    seal_params,
                )
                .await
                {
                    tracing::error!(err = %e, "封卷流程失败");
                }
            });
        }

        if let Ok(turn_count) = volume_store::increment_turn_counter(&sd) {
            let interval = ctx.volume_config.maintenance_interval.max(1) as u64;
            if turn_count > 0 && turn_count % interval == 0 {
                let sd_maint = sd.clone();
                join_set.spawn(async move {
                    if let Err(e) = volume_manager::run_maintenance(&sd_maint) {
                        tracing::error!(err = %e, "维护任务失败");
                    }
                });
            }
        }

        // 等待全部子任务结束；JoinError（panic / cancel）单独 tracing
        while let Some(res) = join_set.join_next().await {
            if let Err(je) = res {
                if je.is_panic() {
                    tracing::error!(err = %je, "封卷/维护子任务 panic");
                } else if je.is_cancelled() {
                    tracing::warn!("封卷/维护子任务被取消");
                }
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn chunks_result_to_events(
    result: Result<Vec<UnpackedChunk>, String>,
) -> Vec<Result<Event, Infallible>> {
    match result {
        Ok(chunks) => chunks
            .into_iter()
            .filter_map(|chunk| match &chunk {
                UnpackedChunk::Think(t) if t.is_empty() => None,
                UnpackedChunk::Body(t) if t.is_empty() => None,
                _ => {
                    let data = serde_json::to_string(&chunk).unwrap_or_default();
                    Some(Ok(Event::default().event("message").data(data)))
                }
            })
            .collect(),
        Err(e) => {
            let data = serde_json::to_string(&serde_json::json!({
                "type": "body_chunk",
                "text": format!("\n[Error/网关错误]: {}\n", e)
            }))
            .unwrap_or_default();
            vec![Ok(Event::default().event("error").data(data))]
        }
    }
}

// ── stdout runner (M4.5) ──────────────────────────────────────────────────────

/// Drives a `PreparedPipeline` to completion, printing `Body` chunks to stdout
/// and `Think` chunks to stderr.
///
/// 与 `build_sse_stream` 共享同一 prepare/stream/finalize 路径——CLI `run` 子命令
/// 复用全部 daemon 改进（FSM + Unpacker + 持久化 + 卷注入）而不需 TCP 自 POST。
pub async fn run_pipeline_to_stdout(pipeline: PreparedPipeline) -> Result<(), AirpError> {
    use std::io::Write;

    let PreparedPipeline {
        provider_config,
        gen_params,
        system_prompt,
        messages,
        mut fsm,
        mut unpacker,
        finalizer,
        http_client,
        engine,
    } = pipeline;

    let raw_stream = call_streaming_api_auto(
        &engine,
        http_client,
        provider_config,
        gen_params,
        system_prompt,
        messages,
    );
    tokio::pin!(raw_stream);

    let mut raw_acc = String::new();
    let mut cleaned_acc = String::new();
    let mut had_error: Option<String> = None;

    while let Some(item) = raw_stream.next().await {
        match item {
            Ok(token) => {
                raw_acc.push_str(&token);
                let cleaned = fsm.process_chunk(&token);
                cleaned_acc.push_str(&cleaned);
                for chunk in unpacker.process_chunk(&cleaned) {
                    print_chunk_to_stdout(&chunk);
                }
            }
            Err(e) => {
                eprintln!("\n[Error]: {}", e);
                had_error = Some(e);
                break;
            }
        }
    }

    if had_error.is_none() {
        let tail = fsm.finish();
        cleaned_acc.push_str(&tail);
        let tail_chunks: Vec<_> = unpacker
            .process_chunk(&tail)
            .into_iter()
            .chain(unpacker.finish())
            .collect();
        for chunk in tail_chunks {
            print_chunk_to_stdout(&chunk);
        }
    }

    println!();
    let _ = std::io::stdout().flush();

    // 即使流出错也调用 finalize，让累积的 user/assistant 文本仍能持久化。
    run_finalize(finalizer, raw_acc, cleaned_acc).await;

    match had_error {
        Some(e) => Err(AirpError::Upstream { status: 0, body: e }),
        None => Ok(()),
    }
}

fn print_chunk_to_stdout(chunk: &UnpackedChunk) {
    use std::io::Write;
    match chunk {
        UnpackedChunk::Body(text) if !text.is_empty() => {
            print!("{}", text);
            let _ = std::io::stdout().flush();
        }
        UnpackedChunk::Think(text) if !text.is_empty() => {
            // stderr 避免污染 stdout 管道；ANSI dim 标记思考块
            eprintln!("\x1b[2m[思考] {}\x1b[0m", text.trim_end());
        }
        UnpackedChunk::ActionOptions { options } if !options.is_empty() => {
            for (i, opt) in options.iter().enumerate() {
                println!("\x1b[33m[选项 {}] {}\x1b[0m", i + 1, opt);
            }
        }
        _ => {}
    }
}

// ── M_LS-1: <state> tag extraction ────────────────────────────────────────────

/// Strips all `<state>…</state>` blocks from `text`.
///
/// Returns `(text_without_state_tags, last_valid_state_json)`.
/// - All `<state>…</state>` blocks are removed from output text.
/// - The **last** block whose content parses as valid JSON is returned as `Some(Value)`.
/// - Unclosed `<state>` tag: kept in text as-is (graceful degradation).
/// - Invalid JSON inside tag: block still removed, but `last_state` not updated.
pub(crate) fn extract_state_content(text: &str) -> (String, Option<serde_json::Value>) {
    const OPEN: &str = "<state>";
    const CLOSE: &str = "</state>";

    let mut result = String::with_capacity(text.len());
    let mut last_state: Option<serde_json::Value> = None;
    let mut pos = 0;

    loop {
        match text[pos..].find(OPEN) {
            None => {
                result.push_str(&text[pos..]);
                break;
            }
            Some(tag_start) => {
                result.push_str(&text[pos..pos + tag_start]);
                let after_open = pos + tag_start + OPEN.len();
                match text[after_open..].find(CLOSE) {
                    None => {
                        // Unclosed tag — keep from <state> onward, stop scanning
                        result.push_str(&text[pos + tag_start..]);
                        break;
                    }
                    Some(content_len) => {
                        let json_str = &text[after_open..after_open + content_len];
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str.trim()) {
                            last_state = Some(v);
                        }
                        pos = after_open + content_len + CLOSE.len();
                    }
                }
            }
        }
    }

    (result, last_state)
}

/// Writes `state` to `characters/{character_id}/state/live.json` (overwrite).
///
/// Failures are silently logged; state persistence is best-effort.
async fn persist_live_state(
    data_root: &std::path::Path,
    character_id: &str,
    state: &serde_json::Value,
) {
    let character = match crate::types::CharacterId::new(character_id) {
        Ok(character) => character,
        Err(error) => {
            tracing::warn!(%error, character_id, "state persistence rejected invalid character id");
            return;
        }
    };
    if let Err(error) = crate::domain::StateService::new(data_root).write(&character, state) {
        tracing::warn!(%error, character_id, "state persistence rejected");
    }
}

// ── M_AGENT-1: 单步生成（供 AgentLoop 协调器复用）─────────────────────────────
//
// 与 `build_sse_stream` 的区别：后者把 prepare→stream→finalize 三相封装成 SSE 流，
// 结果吞进 finalizer；而 AgentLoop 需要每步拿到累积结果（raw / cleaned / 拆包
// chunks）来决策下一步（调工具 or 续写 or 收敛），且 finalize 由协调器在收敛时
// 统一触发，不在每步触发。故抽出此函数：跑一次生成，返回累积，**不 finalize**。
//
// 复用纪律（计划书 §4.1 铁律）：不重写 SSE / provider / 拆包。本函数内部仍走
// `call_streaming_api_auto` + `StreamingFsm` + `StreamingXmlUnpacker`，只是把
// 累积结果交还调用方而非塞进 SSE channel。

/// 单步生成的累积结果。
pub struct GenerationStepResult {
    /// 原始上游输出（pre-filter），最贴近计费 token 的代理。
    pub raw_acc: String,
    /// FSM 过滤后的输出（含 `<state>` 等，未拆包）。
    pub cleaned_acc: String,
    /// XML 拆包后的语义 chunks（immersive / action / state）。
    pub chunks: Vec<UnpackedChunk>,
    /// 上游流错误（若有）；存在时 raw/cleaned 为已累积的部分。
    pub error: Option<String>,
    /// Finalizer retained by the control-plane coordinator and consumed only
    /// after the model has converged on this generation.
    pub finalizer: FinalizerCtx,
}

/// 跑一次生成步骤：复用 `PreparedPipeline` 的全部装配，跑流式生成，返回累积。
///
/// **不触发 finalize**（不持久化 ChatLog / 不落 state / 不封卷）——调用方
/// （`AgentLoop`）在收敛时自行决定是否落库。这避免 loop 多步中间态污染 ChatLog。
pub async fn run_generation_step(pipeline: PreparedPipeline) -> GenerationStepResult {
    let PreparedPipeline {
        provider_config,
        gen_params,
        system_prompt,
        messages,
        mut fsm,
        mut unpacker,
        finalizer,
        http_client,
        engine,
    } = pipeline;

    let raw_stream = call_streaming_api_auto(
        &engine,
        http_client,
        provider_config,
        gen_params,
        system_prompt,
        messages,
    );
    tokio::pin!(raw_stream);

    let mut raw_acc = String::new();
    let mut cleaned_acc = String::new();
    let mut chunks: Vec<UnpackedChunk> = Vec::new();
    let mut error: Option<String> = None;

    while let Some(item) = raw_stream.next().await {
        match item {
            Ok(token) => {
                raw_acc.push_str(&token);
                let cleaned = fsm.process_chunk(&token);
                cleaned_acc.push_str(&cleaned);
                chunks.extend(unpacker.process_chunk(&cleaned));
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }

    if error.is_none() {
        let tail = fsm.finish();
        cleaned_acc.push_str(&tail);
        chunks.extend(unpacker.process_chunk(&tail));
        chunks.extend(unpacker.finish());
    }

    GenerationStepResult {
        raw_acc,
        cleaned_acc,
        chunks,
        error,
        finalizer,
    }
}

/// Commit one converged Agent generation through the same persistence, state,
/// volume, and maintenance finalizer used by the ordinary chat pipeline.
pub async fn finalize_generation(finalizer: FinalizerCtx, raw_acc: String, cleaned_acc: String) {
    run_finalize(finalizer, raw_acc, cleaned_acc).await;
}

#[cfg(test)]
mod tests;
