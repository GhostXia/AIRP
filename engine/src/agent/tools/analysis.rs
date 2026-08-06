//! Analysis enhance/apply family built-in Agent tools.
//!
//! 设计纪律（#155 PR 3）：
//! - 2 个 tool struct 保持私有；对 facade 只暴露 [`register`]。
//! - `ENHANCE_ANALYSIS_SYSTEM_PROMPT` 与 `enhance_md_via_llm_shared` 是
//!   跨模块共享资产（daemon HTTP `enhance` 端点也调），facade 做最小 re-export
//!   保持原 `pub` / `pub(crate)` 调用路径不变。
//! - 不改任何 `ToolMeta` 文案、side_effect 或入参/出参形状。
//! - LLM 调用复用 `state.config` + `state.http_client` + `call_streaming_api_auto`，
//!   与 chat_pipeline 同路径。
//!
//! 工具清单：
//! - `enhance_analysis`：读 analysis MD，调 LLM 增强，返回 diff 预览（readonly）
//! - `apply_enhanced_analysis`：写入确认的 enhanced_md（destructive，默认 dry-run）

use super::params::required_character_id;
use super::*;
use crate::daemon::DaemonState;
use crate::domain::AnalysisService;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ── Decompose Agent Flow：analysis 增强 / 应用 工具（Task 6） ────────────────
//
// 对应计划 `docs/superpowers/plans/2026-07-07-decompose-agent-flow.md`。
// A1 修复：enhance 只读返回 diff 预览，不写盘；apply 是 destructive → dry-run 默认。
// A2 修复：world_book/ 开头的文件名拒绝（世界书只读，不参与 enhance）。
//
// #160 A1：enhance/apply 路径原用 `path.exists()` + `std::fs::read_to_string`，
//   在 `Tool::call` 返回的 Tokio future 内阻塞 executor worker。改为
//   `tokio::fs::try_exists` / `read_to_string`，与 apply 写路径一致。
// #160 A2：apply 路径复用了只描述 enhance 的 "not eligible for enhance" 文案，
//   对调用者描述不准确。提取共享常量，文案同时覆盖 enhance/apply。
//
// L3 修复（issue #92）：enhance 真正调 LLM 增强 analysis MD。
// A3 修复：不调 `state.adapter`（DaemonState 无此字段，计划书 placeholder 已规避），
// 改用 `state.http_client` + `state.config` + `call_streaming_api_auto`，
// 与 chat_pipeline 同路径。LLM 输出原样作为 enhanced_md 返回，apply 端点二次确认写盘。
//
// E-P1-3 P0（PR #431）：读/写盘收口至 `AnalysisService`。
// - enhance 读盘：`spawn_blocking` + `AnalysisService::load_file`（替换 `tokio::fs::try_exists`
//   + `read_to_string`）。Service 内部同步 `std::fs`，`spawn_blocking` 包装避免占用
//   tokio worker 线程（审计 PR #431 Point 4，`search.rs:44` 既定惯例）。
// - apply 写盘：`spawn_blocking` + `AnalysisService::save_file`（替换 `tokio::fs::write`）。
//   原子写（`replace_file` tmp+rename+fsync）+ `character_lock` 串行化，消除半写可见。
// - world_book/ 拒绝 + 路径安全校验已移入 Service，调用方不再重复实现。
// - apply 路径的 `try_exists` 前置存在性校验删除：原校验是防御性 guard，正常流程
//   `apply` 必跟在 `enhance` 之后（文件已被读出），race 场景由 `character_lock`
//   串行化覆盖；删除后 `apply` 可在文件不存在时创建新文件，行为变化已在 PR 描述记录。
// - 已知 gap（审计 Point 1）：character_lock 不检测语义冲突，last-write-wins
//   静默丢失风险由未来 revision 合同/CAS 解决。

/// enhance 专用 system prompt：指示 LLM 增强 analysis MD，保留结构、补全占位符。
/// #155 PR 3：item 自身声明为 `pub`（analysis 是 tools 的私有子模块，`pub` 项
/// 仅 tools 内可达；facade `pub use` re-export 后才对外可见），daemon
/// `enhance_md_via_llm` 经 `crate::agent::tools::ENHANCE_ANALYSIS_SYSTEM_PROMPT`
/// 复用同一份，避免两条路径产物漂移（审计 G2/G3）。
pub const ENHANCE_ANALYSIS_SYSTEM_PROMPT: &str = r#"你是角色卡分析增强助手。下面会给你一份角色卡拆解生成的 Markdown 分析文件，其中可能含 `<!-- Agent分析后填充 -->` 占位符。

任务：
1. 阅读全文，理解角色设定。
2. 补全所有 `<!-- Agent分析后填充 -->` 占位符，写出具体、贴合角色的内容。
3. 保留原有标题层级、字段名、表格结构，不删改已有非占位内容。
4. 输出完整的增强后 Markdown（不要包 ```markdown 围栏，不要加任何前后缀说明）。
5. 世界书条目（world_book/ 前缀）不允许 enhance，调用方已拦截，你不会收到。
"#;

/// `enhance_md_via_llm_shared`：调 LLM 增强 analysis MD，返回 trimmed enhanced_md。
///
/// 共享于 agent tool `EnhanceAnalysisTool` 与 daemon HTTP `enhance` 端点，防漂移（审计 CR5）。
/// 两路径各自的 `has_changes` 比较与原始 MD 保留由调用方处理——本 helper 只负责 LLM 调用与
/// token 累积。复用 `state.config` + `state.http_client` + `call_streaming_api_auto`，
/// 与 chat_pipeline 同路径。低温度（0.3）保证增强稳定。
///
/// #155 PR 3：item 自身声明为 `pub`（analysis 是 tools 的私有子模块，`pub` 项仅 tools
/// 内可达）；facade `pub(crate) use` re-export 为 `crate::agent::tools::enhance_md_via_llm_shared`，
/// 保持原 crate-private 调用路径不变。
pub async fn enhance_md_via_llm_shared(
    state: &Arc<DaemonState>,
    original_md: &str,
    filename: &str,
) -> Result<String, AirpError> {
    use crate::adapter::{call_streaming_api_auto, ChatMessage, GenerationParams, MessageRole};
    use futures_util::StreamExt;

    let (provider_config, gen_params, engine) = {
        let cfg = state.read_config();
        (
            std::sync::Arc::new(crate::adapter::ProviderConfig {
                provider: cfg.provider.clone(),
                endpoint: cfg.endpoint.clone(),
                api_key: cfg.api_key.clone(),
            }),
            GenerationParams {
                model: cfg.model.clone(),
                temperature: Some(0.3),
                // 审计 CR1：2048 对中长卡 analysis MD 增强会截断，提至 8192。
                max_tokens: Some(8192),
            },
            cfg.engine.clone(),
        )
    };

    let messages = vec![ChatMessage {
        role: MessageRole::User,
        content: format!(
            "请增强以下角色卡分析文件（文件名：{}）：\n\n{}",
            filename, original_md
        ),
    }];

    let stream = call_streaming_api_auto(
        &engine,
        state.http_client.clone(),
        provider_config,
        gen_params,
        ENHANCE_ANALYSIS_SYSTEM_PROMPT.to_string(),
        messages,
    );
    tokio::pin!(stream);

    let mut enhanced = String::new();
    let mut upstream_err: Option<String> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(token) => enhanced.push_str(&token),
            Err(e) => {
                upstream_err = Some(e);
                break;
            }
        }
    }
    if let Some(e) = upstream_err {
        return Err(AirpError::Internal(format!(
            "enhance LLM upstream error: {}",
            e
        )));
    }
    Ok(enhanced.trim().to_string())
}

/// `enhance_analysis`：读 analysis MD，调 LLM 增强，返回 diff 预览（A1：不写盘）。
/// readonly。A2：拒绝 world_book/ 前缀（已移入 Service）。
/// L3：真正调 LLM（call_streaming_api_auto，与 chat_pipeline 同路径）。
/// E-P1-3 P0：读盘收口至 `AnalysisService::load_file`（spawn_blocking 包装，
///   审计 PR #431 Point 4：避免相对原 `tokio::fs::read_to_string` 的 blocking 池卸载回归）。
/// params: `{ "character_id": string, "filename": string }`
/// → `{ "filename": string, "original_md": string, "enhanced_md": string, "has_changes": bool }`
struct EnhanceAnalysisTool {
    state: Arc<DaemonState>,
}

impl Tool for EnhanceAnalysisTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "enhance_analysis",
            description: "Read a character analysis MD file, call LLM to fill placeholders, and return a diff preview (readonly, no write). World book entries are read-only and rejected.",
            side_effect: ToolSideEffect::Readonly,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let filename = params
                .get("filename")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing filename".into()))?;

            let cid = required_character_id(&params)?;

            // E-P1-3 P0：读盘收口至 AnalysisService（spawn_blocking 包装）。
            // - world_book/ 拒绝已移入 Service，调用方不再重复实现。
            // - 文件不存在时 Service 返回 `NotFound`（与原 `tokio::fs::try_exists` 行为一致）。
            // - 路径安全校验由 `char_analysis_file_path` 内置，Service 调用它即继承全部校验。
            // - spawn_blocking：Service 内部同步 `std::fs`，相对原 `tokio::fs::read_to_string`
            //   （内部已卸载到 blocking 池）必须显式包装，避免占用 tokio worker 线程（审计 Point 4）。
            let svc = AnalysisService::new(state.data_root.clone());
            let cid_str = cid.as_str().to_string();
            let filename_str = filename.to_string();
            let original_md =
                tokio::task::spawn_blocking(move || svc.load_file(&cid_str, &filename_str))
                    .await
                    .map_err(|e| {
                        AirpError::Internal(format!("analysis load task failed: {e}"))
                    })??;

            // L3：调共享 helper 增强 MD（审计 CR5：抽公共逻辑防两路径漂移）。
            let enhanced_md = enhance_md_via_llm_shared(&state, &original_md, filename).await?;
            let has_changes = enhanced_md != original_md.trim();
            // #432: 返回 original_md_hash 供 apply 阶段做 CAS 校验，防止 enhance→apply
            // 之间文件被修改导致 last-write-wins 静默丢失。
            let original_md_hash = AnalysisService::content_hash(&original_md);
            Ok(ToolResult {
                output: serde_json::json!({
                    "filename": filename,
                    "original_md": original_md,
                    "original_md_hash": original_md_hash,
                    "enhanced_md": enhanced_md,
                    "has_changes": has_changes,
                }),
                dry_run: false,
            })
        })
    }
}

/// `apply_enhanced_analysis`：写入用户确认的 enhanced_md 到 analysis MD 文件。
/// **destructive** → 默认 dry-run，未 confirm 只回预览。
/// A2：拒绝 world_book/ 前缀（已移入 Service）。
/// E-P1-3 P0：写盘收口至 `AnalysisService::save_file`（原子 `replace_file` +
///   `character_lock` 串行化；spawn_blocking 包装，审计 PR #431 Point 4）。
/// params: `{ "character_id": string, "filename": string, "enhanced_md": string }`
/// → `{ "character_id": string, "filename": string, "status": "applied" }`
struct ApplyEnhancedAnalysisTool {
    state: Arc<DaemonState>,
}

impl Tool for ApplyEnhancedAnalysisTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "apply_enhanced_analysis",
            description: "Write a confirmed enhanced_md to a character analysis MD file. Destructive — dry-run unless confirm=true. World book entries are read-only and rejected.",
            side_effect: ToolSideEffect::Destructive,
        }
    }

    fn call(
        &self,
        params: Value,
        confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let filename = params
                .get("filename")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing filename".into()))?;
            let enhanced_md = params
                .get("enhanced_md")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing enhanced_md".into()))?;
            // #432: 可选 expected_hash（来自 enhance 返回的 original_md_hash）。
            // 不传则跳过 CAS 校验（向后兼容旧调用方）。
            let expected_hash = params
                .get("expected_hash")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let cid = required_character_id(&params)?;

            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "character_id": cid.to_string(),
                        "filename": filename,
                        "action": "apply_enhanced_analysis",
                        "will_write": format!("{} bytes of enhanced_md to {}", enhanced_md.len(), filename),
                        "requires": "confirm=true",
                    }),
                    dry_run: true,
                });
            }

            // E-P1-3 P0：写盘收口至 AnalysisService（spawn_blocking 包装）。
            // - world_book/ 拒绝已移入 Service。
            // - 路径安全校验由 `char_analysis_file_path` 内置。
            // - 原子写：`replace_file`（tmp + rename + fsync），消除原 `tokio::fs::write` 的半写可见。
            // - 串行化：`character_lock(character_id).read()`，与 `LorebookService` 读路径一致。
            // - spawn_blocking：Service 内部同步 `std::fs`，相对原 `tokio::fs::write`
            //   （内部已卸载到 blocking 池）必须显式包装，避免占用 tokio worker 线程（审计 Point 4）。
            // - #432 CAS：expected_hash 匹配才写入，否则返回 Conflict 让调用方 re-enhance。
            let svc = AnalysisService::new(state.data_root.clone());
            let cid_str = cid.as_str().to_string();
            let filename_str = filename.to_string();
            let enhanced_md_owned = enhanced_md.to_string();
            tokio::task::spawn_blocking(move || {
                svc.save_file(
                    &cid_str,
                    &filename_str,
                    &enhanced_md_owned,
                    expected_hash.as_deref(),
                )
            })
            .await
            .map_err(|e| AirpError::Internal(format!("analysis save task failed: {e}")))??;

            tracing::warn!(
                character_id = %cid,
                filename,
                "apply_enhanced_analysis executed"
            );
            Ok(ToolResult {
                output: serde_json::json!({
                    "character_id": cid.to_string(),
                    "filename": filename,
                    "status": "applied",
                }),
                dry_run: false,
            })
        })
    }
}

/// 由 facade `default_registry` 集中调用，注册本 family 全部 2 个工具。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(EnhanceAnalysisTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ApplyEnhancedAnalysisTool { state }))
        .expect(COLLISION);
}
