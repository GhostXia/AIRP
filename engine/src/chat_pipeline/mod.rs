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
pub use prepare::{
    prepare_continue_pipeline, prepare_pipeline, prepare_regen_pipeline, preview_pipeline,
};
pub use stdout_runner::run_pipeline_to_stdout;
pub use stream::build_sse_stream;
pub use types::{FinalizerCtx, GenerationStepResult, PrepareMode, PreparedPipeline};

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
