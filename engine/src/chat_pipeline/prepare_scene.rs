//! Scene (multi-character) branch of prepare phase.
//!
//! 与 `prepare::prepare_pipeline_with_mode` 的 single-character 分支并列。当
//! `payload.scene_id` 存在时走此分支：加载 SceneConfig、所有角色卡、合并
//! lorebook、构建多角色 system prompt、把 session_dir 路由到 scene memory/。
//!
//! AUDIT-2：入口处把 `scene_id` 校验为 `SceneId`，下游函数接收 `&SceneId`，
//! 无法绕过校验。

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::{ChatMessage, GenerationParams, ProviderConfig};
use crate::daemon::{ChatCompletionRequest, DaemonState};
use crate::data_dir;
use crate::error::AirpError;
use crate::fsm::{RegexFilter, StreamingFsm};
use crate::orchestrator::{
    inject_current_context, inject_plot_direction, inject_volume_context, SystemPromptPart,
    TavernPreset,
};
use crate::volume_manager;
use crate::xml_unpacker::StreamingXmlUnpacker;

use super::helpers::{
    assemble_regex_filters, effective_root_for_mode, load_char_card_json,
    merge_persona_into_user_profile, resolve_param_sources, resolve_request_persona,
};
use super::trace::build_prompt_trace;
use super::types::{FinalizerCtx, PrepareMode, PreparedPipeline};

/// MS-6: Scene pipeline — loads SceneConfig, all character cards, merges lorebooks,
/// builds a multi-character system prompt, and routes session_dir to scene memory/.
///
/// AUDIT-2: validates scene_id into SceneId once on entry; downstream functions
/// receive `&SceneId` so they cannot accidentally bypass validation.
pub(super) fn prepare_scene_pipeline(
    payload: &ChatCompletionRequest,
    state: &Arc<DaemonState>,
    scene_id: &str,
    mode: PrepareMode,
) -> Result<PreparedPipeline, AirpError> {
    let scene_id = crate::types::SceneId::new(scene_id)
        .map_err(|e| AirpError::BadRequest(format!("非法 scene_id: {}", e)))?;

    // DX-1: per-user data root isolation
    let effective_root =
        effective_root_for_mode(&state.data_root, payload.user_id.as_deref(), mode)?;

    let scene = crate::scene::SceneConfig::load(&effective_root, &scene_id)?;

    // Load card JSONs + per-character lorebooks
    let mut card_jsons: Vec<(String, Option<String>)> = Vec::new();
    let mut lorebooks: Vec<crate::orchestrator::lorebook::SourcedLorebook> = Vec::new();

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
                        lorebooks.push(crate::orchestrator::lorebook::SourcedLorebook {
                            source_id: format!("character:{}", entry.character_id),
                            lorebook: lb,
                        });
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
                lorebooks.push(crate::orchestrator::lorebook::SourcedLorebook {
                    source_id: format!("scene:{scene_id}"),
                    lorebook: lb,
                });
            }
        }
    }

    let merged_lb = crate::orchestrator::lorebook::merge_sourced_lorebooks(&lorebooks);

    // Scan message + history for lorebook triggers
    let mut scan_text = payload.message.clone();
    if let Some(ref history) = payload.messages_history {
        for h in history {
            scan_text.push(' ');
            scan_text.push_str(&h.content);
        }
    }
    let triggered_lore_entries = merged_lb.trigger(&scan_text);

    let cards: Vec<(&str, Option<&str>)> = card_jsons
        .iter()
        .map(|(id, json)| (id.as_str(), json.as_deref()))
        .collect();

    // A1b: scene 模式只走 precedence 1（显式 persona_id）与 3（default）；
    //      `find_for_character` 跳过，因为 scene 有多角色，没有单一绑定目标。
    //      与 single 分支一致，传 `state.data_root`（全局 root）。
    let (request_persona, persona_activation_source) =
        resolve_request_persona(payload, &state.data_root)?;
    let (effective_user_name, effective_user_variables) =
        merge_persona_into_user_profile(&payload.user_profile, request_persona.as_ref());

    let mut assembly = crate::orchestrator::build_multi_char_system_prompt_assembly_sourced(
        &scene,
        &cards,
        &triggered_lore_entries,
        &effective_user_name,
    );
    let mut prompt_variables = effective_user_variables.clone();
    prompt_variables.insert("user".to_string(), effective_user_name.clone());
    for part in &mut assembly.parts {
        for (key, value) in &prompt_variables {
            part.content = part.content.replace(&format!("{{{{{key}}}}}"), value);
        }
    }
    let mut system_prompt: String = assembly
        .parts
        .iter()
        .map(|part| part.content.as_str())
        .collect();
    let mut prompt_parts = assembly.parts;

    let session_dir_opt: Option<PathBuf> = if mode == PrepareMode::Preview {
        let path = effective_root
            .join("scenes")
            .join(scene_id.as_str())
            .join("memory");
        path.is_dir().then_some(path)
    } else {
        data_dir::scene_memory_dir(&effective_root, &scene_id).ok()
    };

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
        // 2.1 常驻有界记忆注入
        let mut resident_memory = String::new();
        crate::memory::inject_resident_memory(sd, &mut resident_memory);
        if !resident_memory.is_empty() {
            system_prompt.push_str(&resident_memory);
            prompt_parts.push(SystemPromptPart {
                source_kind: "memory",
                source_id: payload.session_id.as_ref().map(ToString::to_string),
                item_id: None,
                display_name: "常驻记忆",
                content: resident_memory,
            });
        }
        // 阶段三补全 D3：封卷时生成的剧情方向注入。
        let mut plot_direction = String::new();
        inject_plot_direction(sd, &mut plot_direction);
        if !plot_direction.is_empty() {
            system_prompt.push_str(&plot_direction);
            prompt_parts.push(SystemPromptPart {
                source_kind: "memory",
                source_id: payload.session_id.as_ref().map(ToString::to_string),
                item_id: None,
                display_name: "剧情方向",
                content: plot_direction,
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
    // #114 effective config summary：算出本轮参数来源，传给 trace。
    let param_sources = resolve_param_sources(payload, preset_params.as_ref());
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

    // issue #27：复用共享过滤器组装（含 PR-4 预设正则），与 single 分支产出一致集合。
    let filters = assemble_regex_filters(payload, &effective_root);

    let mut runtime_variables = effective_user_variables.clone();
    runtime_variables.insert("user".to_string(), effective_user_name.clone());
    let fsm = StreamingFsm::new(filters, runtime_variables);

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
            character_id: None,
            session_id: None,
            user_id: payload
                .user_id
                .as_deref()
                .map(crate::types::UserId::new)
                .transpose()?,
            data_root: effective_root.clone(),
            session_dir: session_dir_opt,
            provider_config,
            gen_params,
            volume_config: snapshot.volume_config.clone(),
            http_client: state.http_client.clone(),
            continue_mode: false,
            swipe_candidates: Vec::new(),
        },
        http_client: state.http_client.clone(),
    })
}

// Suppress unused-import warning: RegexFilter is imported because future scene
// filter assembly extensions will land here; matches single-char branch imports.
#[allow(dead_code)]
fn _regex_filter_marker(_f: RegexFilter) {}
