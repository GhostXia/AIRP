//! Prepare phase (single-character branch): validate request, load assets,
//! build system prompt, persist user message, advance timeline.
//!
//! 与 `prepare_scene::prepare_scene_pipeline` 的 multi-character 分支并列。当
//! `payload.scene_id` 缺省时走此分支：加载单一角色卡 / lorebook / preset /
//! persona / 卷上下文，构建 system prompt，并在 Chat 模式下持久化 user message
//! + 推进 timeline / checkpoint。
//!
//! 四个入口（`prepare_pipeline` / `preview_pipeline` / `prepare_regen_pipeline`
//! / `prepare_continue_pipeline`）共享 `prepare_pipeline_with_mode`，差异仅在
//! `PrepareMode`。Preview 不写盘；Regen 不追加 user message；Continue 复用
//! 末尾 assistant message。

use std::fs;
use std::sync::Arc;

use crate::adapter::{ChatMessage, GenerationParams, ProviderConfig};
use crate::daemon::{ChatCompletionRequest, DaemonState};
use crate::data_dir;
use crate::error::AirpError;
use crate::fsm::StreamingFsm;
use crate::orchestrator::{
    inject_current_context, inject_volume_context, Orchestrator, SystemPromptPart, TavernPreset,
};
use crate::volume_manager;
use crate::xml_unpacker::StreamingXmlUnpacker;

use super::helpers::{
    assemble_regex_filters, effective_root_for_mode, merge_persona_into_user_profile,
    read_only_session_dir, resolve_param_sources, resolve_request_persona,
};
use super::prepare_scene::prepare_scene_pipeline;
use super::trace::build_prompt_trace;
use super::types::{FinalizerCtx, PrepareMode, PreparedPipeline};

// ── prepare ───────────────────────────────────────────────────────────────────

/// Validates the request, loads all required files, builds the system prompt,
/// persists the user message, and returns a ready-to-stream pipeline.
pub fn prepare_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
) -> Result<PreparedPipeline, AirpError> {
    prepare_pipeline_with_mode(payload, state, PrepareMode::Chat)
}

/// Build the same provider-ready pipeline without advancing timeline state or writing history.
pub fn preview_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
) -> Result<PreparedPipeline, AirpError> {
    prepare_pipeline_with_mode(payload, state, PrepareMode::Preview)
}

/// Regen: caller has already deleted the last assistant message. Build a pipeline
/// that generates a new response from existing history without appending/persisting
/// a new user message.
pub fn prepare_regen_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
) -> Result<PreparedPipeline, AirpError> {
    prepare_pipeline_with_mode(payload, state, PrepareMode::Regen)
}

/// Continue: build a pipeline that generates a continuation of the last assistant
/// message. The finalizer appends generated text to the existing last message.
pub fn prepare_continue_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
) -> Result<PreparedPipeline, AirpError> {
    prepare_pipeline_with_mode(payload, state, PrepareMode::Continue)
}

fn prepare_pipeline_with_mode(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
    mode: PrepareMode,
) -> Result<PreparedPipeline, AirpError> {
    // MS-6: scene branch — scene_id takes precedence over character_id
    if let Some(ref sid) = payload.scene_id {
        return prepare_scene_pipeline(payload, state, sid, mode);
    }

    // DX-1: per-user data root isolation
    let effective_root =
        effective_root_for_mode(&state.data_root, payload.user_id.as_deref(), mode)?;

    // Resolve all Persona inputs before timeline advancement, chat persistence,
    // or any other request side effect. A rejected explicit Persona must leave
    // user state untouched.
    let (request_persona, persona_activation_source) =
        resolve_request_persona(payload, &state.data_root)?;
    let (effective_user_name, effective_user_variables) =
        merge_persona_into_user_profile(&payload.user_profile, request_persona.as_ref());

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

    // 5. Resolve the checkpoint for this turn before assembly. Both modes stay
    // read-only here; Chat persists the user message and advances the timeline
    // only after every other fallible preparation step succeeds.
    let next_checkpoint = payload
        .character_id
        .as_ref()
        .map(|cid| crate::orchestrator::gating::get_next_checkpoint(&effective_root, cid.as_str()));
    // R-04: Pre-load chat history when client omits messages_history.
    // Loaded before the durable user append so it excludes the current message;
    // the provider message list appends that message explicitly below.
    let auto_history: Option<Vec<ChatMessage>> = if payload.messages_history.is_none() {
        payload.character_id.as_ref().and_then(|cid| {
            let service = crate::domain::ChatService::new(&effective_root);
            let history = if mode == PrepareMode::Preview {
                service.recent_read_only(cid, payload.session_id.as_ref(), 50)
            } else {
                service.recent(cid, payload.session_id.as_ref(), 50)
            };
            history.ok()
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

    // 7. Load preset JSON + extract top-level API params (P-08)
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
    let assembly = orchestrator.build_system_prompt_assembly_with_preset(
        &effective_root,
        cid_str,
        &effective_user_name,
        &effective_user_variables,
        &triggered_lore,
        preset_json.as_deref(),
        payload.enabled_presets.as_ref(),
        &payload.message,
        next_checkpoint.as_deref(),
    );
    let mut system_prompt = assembly.prompt;
    let mut prompt_parts = assembly.parts;

    // 10. Volume context injection
    // M5.1：若客户端指定 session_id 走 sessions/{uuid}/ 路径，否则保持 legacy session/。
    let session_dir_opt: Option<std::path::PathBuf> =
        payload.character_id.as_ref().and_then(|id| {
            if mode == PrepareMode::Preview {
                let path = read_only_session_dir(
                    &effective_root,
                    id.as_str(),
                    payload.session_id.as_ref(),
                );
                path.is_dir().then_some(path)
            } else {
                data_dir::resolve_session_dir(
                    &effective_root,
                    id.as_str(),
                    payload.session_id.as_ref(),
                )
                .ok()
            }
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
        let mut recent_context = String::new();
        inject_current_context(sd, &mut recent_context);
        if !recent_context.is_empty() {
            system_prompt.push_str(&recent_context);
            prompt_parts.push(SystemPromptPart {
                source_kind: "memory",
                source_id: payload.session_id.as_ref().map(ToString::to_string),
                item_id: None,
                display_name: "近期上下文",
                content: recent_context,
            });
        }
        let mut related_history = String::new();
        inject_volume_context(sd, &payload.message, &mut related_history);
        if !related_history.is_empty() {
            system_prompt.push_str(&related_history);
            prompt_parts.push(SystemPromptPart {
                source_kind: "memory",
                source_id: payload.session_id.as_ref().map(ToString::to_string),
                item_id: None,
                display_name: "相关历史卷",
                content: related_history,
            });
        }
        if let Some(hint) = volume_manager::soft_pressure_hint(
            sd,
            snapshot.volume_config.soft_threshold_tokens,
            snapshot.volume_config.hard_threshold_tokens,
        ) {
            system_prompt.push_str(&hint);
            prompt_parts.push(SystemPromptPart {
                source_kind: "memory",
                source_id: payload.session_id.as_ref().map(ToString::to_string),
                item_id: None,
                display_name: "上下文压力提示",
                content: hint,
            });
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

    // #114 effective config summary：算出本轮参数来源，传给 trace。
    let param_sources = resolve_param_sources(payload, preset_params.as_ref());

    // 12. Build message list
    // When client omits messages_history, fall back to auto_history (loaded before step 7).
    // auto_history does NOT include the current user message yet, so we append it here.
    // Regen/Continue modes: history already contains the user message (or ends with
    // assistant for continue); do NOT append a new user message.
    let messages = {
        let mut list = payload
            .messages_history
            .clone()
            .or(auto_history)
            .unwrap_or_default();
        if mode == PrepareMode::Chat || mode == PrepareMode::Preview {
            list.push(ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: payload.message.clone(),
            });
        } else if mode == PrepareMode::Continue {
            // Pre-validate: last message must be assistant (fail before expensive LLM call).
            let last_msg = list.last().ok_or_else(|| {
                AirpError::BadRequest("cannot continue: chat history is empty".into())
            })?;
            if last_msg.role != crate::adapter::MessageRole::Assistant {
                return Err(AirpError::BadRequest(
                    "cannot continue: last message is not from assistant".into(),
                ));
            }
        }
        list
    };
    let prompt_trace = build_prompt_trace(
        payload,
        request_persona.as_ref(),
        persona_activation_source,
        provider_config.as_ref(),
        &gen_params,
        &param_sources,
        &prompt_parts,
        &messages,
        &effective_root,
    );

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

    // Commit preparation side effects last. A non-2xx preparation response is
    // therefore guaranteed uncommitted and safe for the onboarding retry UI.
    // Persist first so an append failure cannot consume a timeline checkpoint.
    // Regen/Continue: skip user message persistence and timeline advancement.
    if mode == PrepareMode::Chat {
        if let Some(ref cid) = payload.character_id {
            crate::domain::ChatService::new(&effective_root).append(
                cid,
                payload.session_id.as_ref(),
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: payload.message.clone(),
                },
            )?;
            Orchestrator::advance_timeline_and_checkpoint(&effective_root, cid.as_str());
        }
    }

    Ok(PreparedPipeline {
        provider_config: provider_config.clone(),
        gen_params: gen_params.clone(),
        system_prompt,
        prompt_trace,
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
            continue_mode: mode == PrepareMode::Continue,
            swipe_candidates: payload.swipe_candidates.clone(),
        },
        http_client: state.http_client.clone(),
    })
}
