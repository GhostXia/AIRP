//! Helper functions shared by `prepare` and `prepare_scene` branches.
//!
//! 这一层是无状态纯函数 + 简单 IO 工具：
//!   - 路径解析（`effective_root_for_mode` / `read_only_session_dir`）
//!   - 标签 / source_id / param_sources 计算（`provider_label` / `trace_source_id`
//!     / `resolve_param_sources`）
//!   - revision 读取（`read_revision_or_diagnostic`）
//!   - 资产加载（`load_char_card_json`）
//!   - Persona 解析与合并（`resolve_request_persona` /
//!     `merge_persona_into_user_profile`）
//!   - 正则过滤器组装（`assemble_regex_filters`）
//!
//! 把这些独立成文件是为了让 `prepare.rs` 与 `prepare_scene.rs` 各自只关注
//! 自己的装配流程，而不混入跨分支共享工具。

use std::fs;
use std::path::PathBuf;

use crate::adapter::Provider as AdapterProvider;
use crate::daemon::ChatCompletionRequest;
use crate::data_dir;
use crate::domain::{Persona, PersonaService};
use crate::error::AirpError;
use crate::fsm::RegexFilter;
use crate::orchestrator::trace::{ParamSources, PersonaActivationSource, PromptDiagnostic};
use crate::orchestrator::TavernPreset;
use crate::types::{PersonaId, UserId};

use super::types::PrepareMode;

pub(super) fn effective_root_for_mode(
    root: &std::path::Path,
    user_id: Option<&str>,
    mode: PrepareMode,
) -> Result<PathBuf, AirpError> {
    // The current P1 WebUI is single-user. Its session/history endpoints use the
    // global data root, so the canonical `default` identity must use that same
    // persistence root. Non-default IDs retain the existing isolation contract.
    if matches!(user_id, None | Some("") | Some("default")) {
        return Ok(root.to_path_buf());
    }
    if mode == PrepareMode::Chat || mode == PrepareMode::Regen || mode == PrepareMode::Continue {
        return data_dir::resolve_effective_root(root, user_id);
    }
    match user_id {
        None | Some("") => Ok(root.to_path_buf()),
        Some(user_id) => {
            data_dir::validate_id_segment(user_id)?;
            Ok(root.join("users").join(user_id))
        }
    }
}

pub(super) fn read_only_session_dir(
    root: &std::path::Path,
    character_id: &str,
    session_id: Option<&crate::types::SessionId>,
) -> PathBuf {
    let character = root.join("characters").join(character_id);
    match session_id {
        Some(session_id) => {
            let session = character.join("sessions").join(session_id.to_string());
            let memory = session.join("memory");
            if memory.is_dir() {
                memory
            } else {
                session
            }
        }
        None => {
            let memory = character.join("memory");
            if memory.is_dir() {
                memory
            } else {
                character.join("session")
            }
        }
    }
}

pub(super) fn provider_label(provider: &AdapterProvider) -> String {
    match provider {
        AdapterProvider::OpenAI => "openai_compatible".to_string(),
    }
}

pub(super) fn trace_source_id(
    source_kind: &str,
    payload: &ChatCompletionRequest,
) -> Option<String> {
    match source_kind {
        "card" | "known" | "lorebook" | "state" => {
            payload.character_id.as_ref().map(ToString::to_string)
        }
        "scene" => payload.scene_id.clone(),
        "preset" => payload.preset_id.as_ref().map(ToString::to_string),
        "memory" | "history" | "user" => payload.session_id.as_ref().map(ToString::to_string),
        _ => None,
    }
}

/// "model 来自请求体还是 preset 还是 daemon 默认"，无需暴露具体 endpoint / api_key。
pub(super) fn resolve_param_sources(
    payload: &ChatCompletionRequest,
    preset_params: Option<&TavernPreset>,
) -> ParamSources {
    let preset_temperature = preset_params.and_then(|p| p.temperature);
    let preset_max_tokens = preset_params.and_then(|p| p.max_tokens);
    let preset_model = preset_params.and_then(|p| p.model.as_deref());

    let provider_source = if payload.provider.is_some() {
        Some("request")
    } else {
        Some("snapshot")
    };
    let model_source = if payload.model.is_some() {
        Some("request")
    } else if preset_model.is_some() {
        Some("preset")
    } else {
        Some("snapshot")
    };
    let (temperature, temperature_source) = if payload.temperature.is_some() {
        (payload.temperature, Some("request"))
    } else if preset_temperature.is_some() {
        (preset_temperature, Some("preset"))
    } else {
        (None, None)
    };
    let (max_tokens, max_tokens_source) = if payload.max_tokens.is_some() {
        (payload.max_tokens, Some("request"))
    } else if preset_max_tokens.is_some() {
        (preset_max_tokens, Some("preset"))
    } else {
        (None, None)
    };

    ParamSources {
        provider_source,
        model_source,
        temperature,
        temperature_source,
        max_tokens,
        max_tokens_source,
    }
}

/// Phase 2h：尝试读取 `asset_dir/current_revision`。
/// - 可读 → `Some(rev)`
/// - 文件不存在（旧数据未升级）→ 推送 `{kind}` 诊断，返回 `None`
/// - 文件存在但损坏 → 服务端日志记录详细错误（含路径），HTTP 诊断用通用消息，返回 `None`
///
/// CodeRabbit 审计修复：诊断消息不包含文件系统路径，避免通过 HTTP preview 接口泄露
/// 服务器内部路径结构。详细错误（含 `asset_dir` 路径与原始错误）通过 `tracing::warn!`
/// 记录到服务端日志，便于运维排查。
pub(super) fn read_revision_or_diagnostic(
    asset_dir: &std::path::Path,
    kind: &str,
    asset_label: &str,
    diagnostics: &mut Vec<PromptDiagnostic>,
) -> Option<u64> {
    match crate::revision::atomic::read_current_revision(asset_dir) {
        Ok(Some(rev)) => Some(rev),
        Ok(None) => {
            diagnostics.push(PromptDiagnostic {
                kind: kind.to_string(),
                message: format!(
                    "{asset_label} 尚未升级到统一 revision 合同（无 current_revision）。"
                ),
            });
            None
        }
        Err(e) => {
            tracing::warn!(
                asset_dir = ?asset_dir,
                error = %e,
                kind = kind,
                "Phase 2h: current_revision 读取失败（路径仅记录在服务端日志，不进入 HTTP 诊断）"
            );
            diagnostics.push(PromptDiagnostic {
                kind: kind.to_string(),
                message: format!("{asset_label} current_revision 读取失败（详见服务端日志）。"),
            });
            None
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// MS-6: Try to load a character card JSON from the standard locations.
/// Priority: card/raw.json > card.json > card.png
pub(super) fn load_char_card_json(root: &std::path::Path, character_id: &str) -> Option<String> {
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
/// 返回 `(Option<Persona>, PersonaActivationSource)`：`None` 表示请求未带
/// `user_id`（保持单用户向后兼容，source = `Absent`）；否则按以下顺序返回 Persona：
///
/// 1. **显式 `persona_id`**（请求体指定；source = `Explicit`）：id 不存在时
///    `find_for_character` / `get` 已在 `PersonaService` 内返 `NotFound`，与本契约一致。
///    `default` 大小写不敏感（service 内 canonicalize）。
/// 2. **绑定查找**（仅单角色分支；`scene_id` 缺省且 `character_id` 存在；
///    source = `SessionBinding` 或 `CharacterBinding`）：`PersonaService::find_for_character`
///    先精确匹配 session 绑定（→ `SessionBinding`），再匹配该角色下的 generic 绑定
///    （→ `CharacterBinding`）。命中后用 `get` 读出 Persona。
/// 3. **默认 persona**（source = `Default`）：`get_default` 返回存储的 default；
///    未写盘时返回 `Persona::initial` 内存快照（不触发隐式落盘）。
///
/// Scene 模式（`scene_id` 存在）只走 precedence 1 与 3：scene 有多角色，
/// 没有单一绑定目标，`find_for_character` 跳过；多角色 persona 绑定语义延后。
///
/// #114 effective config summary：返回的 `PersonaActivationSource` 用于填充
/// `EffectiveIds.persona_activation_source`，让用户能看到本轮 persona 是
/// "显式选择" / "自动（绑定）" / "默认" 哪条路径命中。
pub(super) fn resolve_request_persona(
    payload: &ChatCompletionRequest,
    data_root: &std::path::Path,
) -> Result<(Option<Persona>, PersonaActivationSource), AirpError> {
    let Some(user_id_str) = payload.user_id.as_deref() else {
        return Ok((None, PersonaActivationSource::Absent));
    };
    let uid = UserId::new(user_id_str)?;
    let service = PersonaService::new(data_root);

    // precedence 1: 显式 persona_id
    // #153 E1: persona_id 现在是 Option<PersonaId>，反序列化阶段已校验，
    // 此处不再需要重复 validate_id_segment。as_str() 取内部 &str 传给 service。
    if let Some(persona_id) = payload.persona_id.as_ref().map(PersonaId::as_str) {
        let persona = service.get(&uid, persona_id, "User")?;
        return Ok((Some(persona), PersonaActivationSource::Explicit));
    }

    // precedence 2: resolve_effective_persona（仅单角色分支）
    // 复用 PersonaService 的结构化解析，确保与 HTTP effective 端点使用同一真相；
    // source 字段直接告诉调用方命中了 session_binding / character_binding / default。
    if payload.scene_id.is_none() {
        if let Some(ref cid) = payload.character_id {
            // SessionId 是 newtype(uuid::Uuid)；find_for_character 需要
            // Option<&str>。在本地构造 String 再借，避免给 SessionId 强加 Deref。
            let session_id_str = payload.session_id.as_ref().map(|s| s.to_string());
            let resolution =
                service.resolve_effective_persona(&uid, cid.as_str(), session_id_str.as_deref())?;
            if let Some(pid) = resolution.effective_persona_id {
                let persona = service.get(&uid, &pid, "User")?;
                let source = match resolution.source {
                    crate::domain::EffectivePersonaSource::SessionBinding => {
                        PersonaActivationSource::SessionBinding
                    }
                    crate::domain::EffectivePersonaSource::CharacterBinding => {
                        PersonaActivationSource::CharacterBinding
                    }
                    // resolve_effective_persona 在 session/character 都未命中时
                    // 返回 source=Default + effective_persona_id=None，不会进到这里。
                    crate::domain::EffectivePersonaSource::Default => {
                        PersonaActivationSource::Default
                    }
                };
                return Ok((Some(persona), source));
            }
        }
    }

    // precedence 3: default persona
    let persona = service.get_default(&uid, "User")?;
    Ok((Some(persona), PersonaActivationSource::Default))
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
pub(super) fn merge_persona_into_user_profile(
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
pub(super) fn assemble_regex_filters(
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
