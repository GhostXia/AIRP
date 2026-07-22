//! Types shared across chat_pipeline submodules.
//!
//! 包含 prepare / stream / finalize / generation_step 阶段共用的所有权边界类型。
//! 公开 API 表面（`PreparedPipeline` / `FinalizerCtx` / `PrepareMode` /
//! `SseMessage` / `GenerationStepResult`）由父模块 `chat_pipeline` 通过
//! `pub use` 重新导出，外部调用方不应直接引用 `chat_pipeline::types::*`。

use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::{BackendEngine, ChatMessage, GenerationParams, ProviderConfig};
use crate::config::VolumeConfig;
use crate::fsm::StreamingFsm;
use crate::orchestrator::trace::PromptAssemblyTrace;
use crate::types::{CharacterId, SessionId, UserId};
use crate::xml_unpacker::{StreamingXmlUnpacker, UnpackedChunk};

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
    /// 与实际 provider payload 同源的有界、无正文装配轨迹。
    pub prompt_trace: PromptAssemblyTrace,
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
    pub character_id: Option<CharacterId>,
    /// Named session scope; `None` keeps the legacy per-character log.
    pub session_id: Option<SessionId>,
    /// DX-1：用户 ID；为 `Some` 时 `data_root` 已是该用户的独立根，
    /// 用户模型（user_model.md）直接落在 `data_root` 下。为 `None` 时
    /// 跳过用户模型抽取（单用户向后兼容模式）。
    pub user_id: Option<UserId>,
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
    /// Continue mode: append generated text to the existing last assistant
    /// message instead of creating a new one.
    pub continue_mode: bool,
    /// #249 Swipe：regen 时捕获的旧候选列表。非空时，finalizer 会将新生成
    /// 的文本追加为最后一个候选，并将 swipe_index 指向新候选。
    pub swipe_candidates: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareMode {
    Chat,
    Preview,
    /// Regen: delete last assistant message (done by caller) then generate a
    /// new response from existing history. Does NOT append/persist a new user
    /// message and does NOT advance timeline/checkpoint.
    Regen,
    /// Continue: generate a continuation of the last assistant message without
    /// adding a new user message. Finalizer appends to the existing last
    /// assistant message rather than creating a new one.
    Continue,
}

/// Internal SSE message envelope used between the processing task and the
/// SSE response stream. Private to `chat_pipeline`.
pub enum SseMessage {
    Chunks(Vec<UnpackedChunk>),
    Error {
        code: String,
        message: String,
        retryable: bool,
        commit_state: &'static str,
    },
    Done,
}

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
