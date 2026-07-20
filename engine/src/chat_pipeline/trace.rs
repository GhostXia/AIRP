//! Prompt assembly trace construction (#115 Phase 2h).
//!
//! `build_prompt_trace` 把本轮 system prompt 装配过程转换为有界、无正文的
//! `PromptAssemblyTrace`：每段来源（card / persona / preset / lorebook / state /
//! memory / history / user）记录 source_id / position / chars / estimated_tokens /
//! stable_or_volatile，并按 §5.3 规则逐个 read_current_revision 填充 6 类
//! `*_revision`，旧数据或读取失败时推送对应 `*_revision_unavailable` 诊断。
//!
//! 独立成文件便于聚焦 trace 不变量与 revision 双源读取逻辑（Persona 双源：
//! current_revision 优先，回退 Persona.revision）。

use crate::adapter::{ChatMessage, GenerationParams, ProviderConfig};
use crate::daemon::ChatCompletionRequest;
use crate::domain::Persona;
use crate::orchestrator::trace::{
    EffectiveIds, ParamSources, PersonaActivationSource, PromptAssemblyTrace, PromptDiagnostic,
    PromptSegment, Stability,
};
use crate::orchestrator::SystemPromptPart;

use super::helpers::{
    provider_label, read_only_session_dir, read_revision_or_diagnostic, trace_source_id,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn build_prompt_trace(
    payload: &ChatCompletionRequest,
    persona: Option<&Persona>,
    persona_source: PersonaActivationSource,
    provider_config: &ProviderConfig,
    gen_params: &GenerationParams,
    param_sources: &ParamSources,
    prompt_parts: &[SystemPromptPart],
    messages: &[ChatMessage],
    effective_root: &std::path::Path,
) -> PromptAssemblyTrace {
    let mut position = 0usize;
    let mut segments = Vec::new();

    for part in prompt_parts {
        let chars = part.content.chars().count();
        segments.push(PromptSegment {
            source_kind: part.source_kind.to_string(),
            source_id: part
                .source_id
                .clone()
                .or_else(|| trace_source_id(part.source_kind, payload)),
            item_id: part.item_id.clone(),
            display_name: Some(part.display_name.to_string()),
            role: Some("system".to_string()),
            position,
            enabled_reason: Some("进入本轮 system prompt".to_string()),
            chars,
            estimated_tokens: crate::volume_store::estimate_tokens(&part.content),
            truncated: false,
            stable_or_volatile: match part.source_kind {
                "known" | "state" | "memory" => Stability::Volatile,
                _ => Stability::Stable,
            },
        });
        position += part.content.len();
    }

    for (index, message) in messages.iter().enumerate() {
        let is_current_user = index + 1 == messages.len();
        let source_kind = if is_current_user { "user" } else { "history" };
        let role = match message.role {
            crate::adapter::MessageRole::User => "user",
            crate::adapter::MessageRole::Assistant => "assistant",
            crate::adapter::MessageRole::System => "system",
        };
        segments.push(PromptSegment {
            source_kind: source_kind.to_string(),
            source_id: trace_source_id(source_kind, payload),
            item_id: None,
            display_name: Some(if is_current_user {
                "当前消息".to_string()
            } else {
                "会话历史".to_string()
            }),
            role: Some(role.to_string()),
            position,
            enabled_reason: Some(if is_current_user {
                "本轮用户输入".to_string()
            } else {
                "纳入本轮上下文窗口".to_string()
            }),
            chars: message.content.chars().count(),
            estimated_tokens: crate::volume_store::estimate_tokens(&message.content),
            truncated: false,
            stable_or_volatile: Stability::Volatile,
        });
        position += message.content.len();
    }

    let mut diagnostics = Vec::new();
    // #115 Phase 2h：trace 完整性收口。
    // Phase 2b-2g 已让 6 个 asset service 在写入时 commit_revision，落盘 `current_revision`。
    // 这里按 spec §5.3 规则逐个 read_current_revision：
    //   - 可读 → 填充实际 u64
    //   - 不可读（旧数据未升级或文件损坏）→ 推送对应 *_revision_unavailable 诊断
    // 不允许用 mtime、文件名时间戳或单调递增计数器冒充内容版本。
    //
    // CodeRabbit 审计修复：当 `character_card_id` 或 `lorebook_path` 显式指定外部
    // card/lorebook 源时，不读取 `characters/{cid}/` 下的 canonical revision 指针——
    // 实际 prompt 内容不来自该目录，读取会产生误导性 revision。留 None 不 push 诊断。
    let is_character_context = payload.scene_id.is_none() && payload.character_id.is_some();
    let uses_canonical_character_card = payload.character_card_id.is_none();
    let uses_canonical_lorebook = payload.lorebook_path.is_none();

    // ── Character revision（character 上下文）─────────────────────────────
    // scene 模式下 character_revision 字段语义不适用（多角色无单一 revision），留 None 不 push。
    // `character_card_id` 提供外部/内联 card 时，canonical 目录 revision 不代表实际内容，跳过。
    let character_revision = if is_character_context && uses_canonical_character_card {
        payload.character_id.as_ref().and_then(|cid| {
            let char_dir = effective_root.join("characters").join(cid.as_str());
            read_revision_or_diagnostic(
                &char_dir,
                "character_revision_unavailable",
                "角色卡",
                &mut diagnostics,
            )
        })
    } else {
        None
    };

    // ── Lorebook revision（character 上下文）───────────────────────────────
    // scene 模式下 lorebook 来源由场景决定，不在此处填充。
    // `lorebook_path` 提供外部/内联 lorebook 时，canonical world/ revision 不代表实际内容，跳过。
    let lorebook_revision = if is_character_context && uses_canonical_lorebook {
        payload.character_id.as_ref().and_then(|cid| {
            let world_dir = effective_root
                .join("characters")
                .join(cid.as_str())
                .join("world");
            read_revision_or_diagnostic(
                &world_dir,
                "lorebook_revision_unavailable",
                "Worldbook",
                &mut diagnostics,
            )
        })
    } else {
        None
    };

    // ── State revision（character 上下文）─────────────────────────────────
    let state_revision = if is_character_context {
        payload.character_id.as_ref().and_then(|cid| {
            let state_dir = effective_root
                .join("characters")
                .join(cid.as_str())
                .join("state");
            read_revision_or_diagnostic(
                &state_dir,
                "state_revision_unavailable",
                "State",
                &mut diagnostics,
            )
        })
    } else {
        None
    };

    // ── Memory revision（character 上下文，按 session_dir 读取）───────────
    let memory_revision = if is_character_context {
        payload.character_id.as_ref().and_then(|cid| {
            let session_dir =
                read_only_session_dir(effective_root, cid.as_str(), payload.session_id.as_ref());
            read_revision_or_diagnostic(
                &session_dir,
                "memory_revision_unavailable",
                "Memory",
                &mut diagnostics,
            )
        })
    } else {
        None
    };

    // ── Preset revision（已由 Phase 2b 实现，保持原逻辑）──────────────────
    let preset_revision = payload.preset_id.as_ref().and_then(|pid| {
        let preset_dir = effective_root.join("presets").join(pid.as_str());
        read_revision_or_diagnostic(
            &preset_dir,
            "preset_revision_unavailable",
            "Preset",
            &mut diagnostics,
        )
    });

    // ── Persona revision（双源读取：current_revision 优先，回退 Persona.revision）──
    // spec §6.6 D6.4：新数据读 personas/{pid}/current_revision；旧数据无该指针时
    // 回退到 Persona.revision，二者皆不可用才 push 诊断。
    // scene 模式下 persona 由场景决定，跳过诊断推送。
    // 注意：`effective_root` 在 Chat 模式下已被 `resolve_effective_root` 解析为
    // `data_root/users/{uid}/`，所以 persona asset 路径直接基于 effective_root 拼装，
    // 不要再叠加 `users/{uid}`（避免 double-prefix）。
    let (persona_id, persona_revision) = match persona {
        Some(value) => {
            let pid = value.id.clone();
            let persona_asset_dir = effective_root.join("personas").join(&value.id);
            let rev = match crate::revision::atomic::read_current_revision(&persona_asset_dir) {
                Ok(Some(rev)) => Some(rev),
                Ok(None) => {
                    // spec §6.6 D6.4 双源读取：current_revision 不存在时回退到
                    // `Persona.revision`（legacy 字段）。但 `Persona.revision == 0`
                    // 表示 `Persona::initial`（get_default 在无 persona 文件时返回
                    // 的内存占位），实际从未保存过——视作"两者都不可用"，
                    // 推送 persona_revision_unavailable 诊断。
                    if value.revision > 0 {
                        Some(value.revision)
                    } else {
                        diagnostics.push(PromptDiagnostic {
                            kind: "persona_revision_unavailable".to_string(),
                            message: "Persona 尚未升级到统一 revision 合同且无 legacy revision 可回退（Persona::initial 未保存）.".to_string(),
                        });
                        None
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        persona_dir = ?persona_asset_dir,
                        error = %e,
                        "Phase 2h: persona current_revision 读取失败（路径仅记录在服务端日志）"
                    );
                    diagnostics.push(PromptDiagnostic {
                        kind: "persona_revision_unavailable".to_string(),
                        message: "Persona current_revision 读取失败（详见服务端日志）。"
                            .to_string(),
                    });
                    None
                }
            };
            (Some(pid), rev)
        }
        None => {
            if is_character_context {
                diagnostics.push(PromptDiagnostic {
                    kind: "persona_revision_unavailable".to_string(),
                    message: "本轮未激活 persona；persona_revision 不可用。".to_string(),
                });
            }
            (None, None)
        }
    };

    PromptAssemblyTrace::new(
        EffectiveIds {
            character_id: if is_character_context {
                payload.character_id.as_ref().map(ToString::to_string)
            } else {
                None
            },
            character_revision,
            persona_id,
            persona_revision,
            preset_id: payload.preset_id.as_ref().map(ToString::to_string),
            preset_revision,
            lorebook_revision,
            scene_id: payload.scene_id.clone(),
            state_revision,
            memory_revision,
            provider: provider_label(&provider_config.provider),
            // Endpoint paths can contain deployment details or query credentials. The product
            // summary only needs to disclose whether an endpoint is configured.
            endpoint: if provider_config.endpoint.is_empty() {
                "not_configured".to_string()
            } else {
                "configured".to_string()
            },
            model: gen_params.model.clone(),
            // ── #114 effective config summary ───────────────────────────────
            // Persona 激活来源与显示名：source 来自 resolve_request_persona；
            // name 仅暴露 persona.name（显示名），不暴露 variables / api_key。
            persona_activation_source: Some(persona_source.as_str().to_string()),
            persona_name: persona.map(|p| p.name.clone()),
            // 参数来源：由 resolve_param_sources 在调用前一次性算好。
            provider_source: param_sources.provider_source.map(|s| s.to_string()),
            model_source: param_sources.model_source.map(|s| s.to_string()),
            temperature: param_sources.temperature,
            temperature_source: param_sources.temperature_source.map(|s| s.to_string()),
            max_tokens: param_sources.max_tokens,
            max_tokens_source: param_sources.max_tokens_source.map(|s| s.to_string()),
        },
        segments,
        diagnostics,
    )
}
