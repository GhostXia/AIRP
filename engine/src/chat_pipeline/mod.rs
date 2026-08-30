//! M3: Chat Pipeline — three phases:
//!   `prepare` (validate + build prompt) → `stream` (FSM + unpack + SSE)
//!   → `finalize` (persist + volume side-effects).
//! FSM + Unpacker owned by stream task (no Arc/Mutex); oneshot channel to finalizer.
//!
//! 模块拆分（审计 §4.4，子模块均为私有，公开 API 由本文件 `pub use` 再导出）：
//!   - [`types`]：跨阶段共享所有权边界类型（`PreparedPipeline` / `FinalizerCtx`
//!     / `PrepareMode` / `SseMessage` / `GenerationStepResult`）。
//!   - [`helpers`]：prepare / prepare_scene 共享的无状态工具（路径解析、param
//!     sources、revision 读取、persona 合并、过滤器组装等）。
//!   - [`trace`]：#115 Phase 2h 装配轨迹构建（含 6 类 asset revision 双源读取）。
//!   - [`prepare`]：单角色分支 prepare 入口（`prepare_pipeline` /
//!     `preview_pipeline` / `prepare_regen_pipeline` /
//!     `prepare_continue_pipeline`）。
//!   - [`prepare_scene`]：多角色 scene 分支 prepare 入口。
//!   - [`stream`]：把 `PreparedPipeline` 转 SSE 事件流（FSM + Unpacker + mpsc）。
//!   - [`finalize`]：assistant 消息 / live state / 封卷副作用提交点。
//!   - [`state_extract`]：`<state>…</state>` 块剥离与 JSON 解析。
//!   - [`stdout_runner`]：CLI `run` 子命令路径，复用全部 daemon 改进。
//!   - [`generation_step`]：M_AGENT-1 单步生成，供 AgentLoop 协调器复用。
//!   - tests：`#[cfg(test)] mod tests;`，保留原测试结构不拆分。
//!     （`tests` 是 `#[cfg(test)]` 模块，非测试构建下不存在，故不使用 intra-doc 链接。）
//!
//! 公开 API 表面由本文件 `pub use` 重新导出，外部调用方应使用
//! `crate::chat_pipeline::*` 而非直接引用子模块。
//!
// `rustdoc::private_intra_doc_links`：上面的 [`types`] / [`helpers`] 等链接指向
// 私有子模块，公开 docs 渲染时无法解析。这里有意保留链接——在
// `cargo doc --document-private-items` 模式下能正确跳转，便于内部导航。
// 抑制此 lint 比删除链接更符合"更开放、更透明"取向。
#![allow(rustdoc::private_intra_doc_links)]

mod finalize;
mod generation_step;
mod helpers;
mod prepare;
mod prepare_scene;
mod state_extract;
mod stdout_runner;
mod stream;
mod trace;
mod types;

#[cfg(test)]
mod tests;

// ── Public API surface (preserved verbatim from old chat_pipeline.rs) ─────────

pub use finalize::finalize_generation;
pub use generation_step::run_generation_step;
pub(crate) use prepare::prepare_regen_pipeline;
pub use prepare::{prepare_continue_pipeline, prepare_pipeline, preview_pipeline};
pub use stdout_runner::run_pipeline_to_stdout;
pub use stream::build_sse_stream;
pub use types::{FinalizerCtx, GenerationStepResult, PrepareMode, PreparedPipeline};

/// Prepare one authoritative speaker inside a scene conversation.
///
/// The regular scene pipeline remains the source of prompt assembly, provider
/// selection, filters, memory, lorebook, and persona behavior. This adapter
/// only narrows the current response to one scene participant and replaces the
/// legacy caller-supplied history with the Conversation event projection.
pub(crate) fn prepare_scene_participant_pipeline(
    payload: &crate::daemon::ChatCompletionRequest,
    state: &std::sync::Arc<crate::daemon::DaemonState>,
    speaker_character_id: &str,
    messages: Vec<crate::adapter::ChatMessage>,
) -> Result<PreparedPipeline, crate::error::AirpError> {
    let scene_id = payload
        .scene_id
        .as_deref()
        .ok_or_else(|| crate::error::AirpError::Internal("scene id not injected".to_string()))?;
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, payload.user_id.as_deref())?;
    let scene =
        crate::scene::SceneConfig::load(&effective_root, &crate::types::SceneId::new(scene_id)?)?;
    if !scene
        .characters
        .iter()
        .any(|entry| entry.character_id == speaker_character_id)
    {
        return Err(crate::error::AirpError::Conflict(format!(
            "conversation participant character:{speaker_character_id} is no longer in scene {scene_id}"
        )));
    }

    let mut pipeline =
        prepare_scene::prepare_scene_pipeline(payload, state, scene_id, PrepareMode::Chat)?;
    let focus = format!(
        "\n\n[Scene turn]\nRespond only as the scene character with id \
         \"{speaker_character_id}\". Do not speak for the user or any other participant."
    );
    let position = pipeline.system_prompt.len();
    pipeline.system_prompt.push_str(&focus);
    let mut segments = std::mem::take(&mut pipeline.prompt_trace.segments);
    segments.retain(|segment| segment.source_kind != "history" && segment.source_kind != "user");
    segments.push(crate::orchestrator::trace::PromptSegment {
        source_kind: "scene_turn".to_string(),
        source_id: Some(scene_id.to_string()),
        item_id: Some(speaker_character_id.to_string()),
        display_name: Some("Active scene speaker".to_string()),
        role: Some("system".to_string()),
        position,
        enabled_reason: Some("selected by conversation orchestration policy".to_string()),
        chars: focus.chars().count(),
        estimated_tokens: crate::volume_store::estimate_tokens(&focus),
        truncated: false,
        stable_or_volatile: crate::orchestrator::trace::Stability::Volatile,
        input_class: crate::orchestrator::trace::PromptInputClass::RpDomain,
        content_revision: None,
        content_hash: None,
        original_bytes: None,
        included_bytes: None,
        redacted: None,
        evidence_items: None,
    });
    let mut message_position = position + focus.len();
    for message in &messages {
        let role = match message.role {
            crate::adapter::MessageRole::User => "user",
            crate::adapter::MessageRole::Assistant => "assistant",
            crate::adapter::MessageRole::System => "system",
        };
        segments.push(crate::orchestrator::trace::PromptSegment {
            source_kind: "conversation_event".to_string(),
            source_id: None,
            item_id: None,
            display_name: Some("Conversation event projection".to_string()),
            role: Some(role.to_string()),
            position: message_position,
            enabled_reason: Some("projected from authoritative event journal".to_string()),
            chars: message.content.chars().count(),
            estimated_tokens: crate::volume_store::estimate_tokens(&message.content),
            truncated: false,
            stable_or_volatile: crate::orchestrator::trace::Stability::Volatile,
            input_class: crate::orchestrator::trace::PromptInputClass::RpDomain,
            content_revision: None,
            content_hash: None,
            original_bytes: None,
            included_bytes: None,
            redacted: None,
            evidence_items: None,
        });
        message_position += message.content.len();
    }
    pipeline.prompt_trace = crate::orchestrator::trace::PromptAssemblyTrace::new(
        pipeline.prompt_trace.effective.clone(),
        segments,
        pipeline.prompt_trace.diagnostics.clone(),
    );
    pipeline.messages = messages;
    Ok(pipeline)
}

pub(crate) fn extract_conversation_state(text: &str) -> (String, Option<serde_json::Value>) {
    state_extract::extract_state_content(text)
}

// ── Test-only re-exports ──────────────────────────────────────────────────────
//
// `tests.rs` 用 `use super::*;` 拉入父模块作用域。原 `chat_pipeline.rs` 是单文件，
// 文件顶部的 `use std::fs; use std::path::PathBuf; use crate::adapter::ChatMessage;`
// 等所有 import 都通过 `use super::*;` 进入 tests 子模块。拆分后这些 import 移到
// 各子模块，tests.rs 的 glob 就拉不到了。
//
// 这里用 `#[cfg(test)] use ...` 把原文件顶部 import 与内部辅助函数重新带入
// `chat_pipeline` 模块作用域，让 tests.rs 的 `use super::*;` 行为保持不变。
// `#[allow(unused_imports)]` 抑制 "unused import" 警告——这些 import 在 mod.rs
// 本体不被引用，仅由 tests.rs 通过 glob 消费。
#[cfg(test)]
#[allow(unused_imports)]
use std::{fs, path::PathBuf, sync::Arc};

#[cfg(test)]
#[allow(unused_imports)]
use crate::adapter::{BackendEngine, ChatMessage, GenerationParams, Provider, ProviderConfig};
#[cfg(test)]
#[allow(unused_imports)]
use crate::config::VolumeConfig;
#[cfg(test)]
#[allow(unused_imports)]
use crate::daemon::{ChatCompletionRequest, DaemonState, MutableConfig, UserProfile};
#[cfg(test)]
#[allow(unused_imports)]
use crate::data_dir;
#[cfg(test)]
#[allow(unused_imports)]
use crate::domain::{ChatService, Persona, PersonaBinding, PersonaService};
#[cfg(test)]
#[allow(unused_imports)]
use crate::error::AirpError;
#[cfg(test)]
#[allow(unused_imports)]
use crate::fsm::{RegexFilter, StreamingFsm};
#[cfg(test)]
#[allow(unused_imports)]
use crate::orchestrator::trace::{
    EffectiveIds, ParamSources, PersonaActivationSource, PromptAssemblyTrace, PromptDiagnostic,
    PromptSegment, Stability,
};
#[cfg(test)]
#[allow(unused_imports)]
use crate::orchestrator::{
    inject_current_context, inject_volume_context, Orchestrator, SystemPromptPart, TavernPreset,
};
#[cfg(test)]
#[allow(unused_imports)]
use crate::types::{CharacterId, SessionId, UserId};
#[cfg(test)]
#[allow(unused_imports)]
use crate::xml_unpacker::{StreamingXmlUnpacker, UnpackedChunk};
#[cfg(test)]
#[allow(unused_imports)]
use crate::{volume_manager, volume_store};

#[cfg(test)]
#[allow(unused_imports)]
use finalize::persist_live_state;
#[cfg(test)]
#[allow(unused_imports)]
use helpers::{
    assemble_regex_filters, effective_root_for_mode, load_char_card_json,
    merge_persona_into_user_profile, provider_label, read_only_session_dir,
    read_revision_or_diagnostic, resolve_param_sources, resolve_request_persona, trace_source_id,
};
#[cfg(test)]
#[allow(unused_imports)]
use state_extract::extract_state_content;
#[cfg(test)]
#[allow(unused_imports)]
use trace::build_prompt_trace;
