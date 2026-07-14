//! Engine-authoritative Agent tool execution surface.
//!
//! 协调器（`AgentLoop`）在每一步选择"派生纯净 subagent / 调一个工具 / 收敛结束"。
//! 本模块定义工具抽象、注册表、capability/allowlist 门和内建 domain tools。
//!
//! ## 模块结构（#155 PR 2 之后）
//! - `tools.rs`（本文件）：facade。保留 [`Tool`] / [`ToolMeta`] / [`ToolResult`] /
//!   [`ToolSideEffect`] / [`ToolRegistry`] / [`EchoTool`] / [`default_registry`]
//!   契约，以及尚未拆出的 PR 3 工具（state / lorebook / volume / context / analysis）。
//! - `tools/params.rs`：跨 family 参数 helper（`pub(super)`，不外泄）。
//! - `tools/session.rs`：session family 5 工具，`pub(super) fn register` 集中注册。
//! - `tools/character.rs`：character family 3 工具，`pub(super) fn register` 集中注册。
//! - `tools/tests/`：按 family 分组的测试子模块。
//!
//! ## 设计纪律（计划书 §2.1 第 4 条：工具最小授权）
//! - 每个工具带 [`ToolMeta`]：`readonly` / `mutate` / `destructive` / `append`。
//! - **破坏性工具默认 dry-run**：[`Tool::call`] 接收 `confirm: bool`，未确认时
//!   破坏性工具只回"将执行什么"的描述，不落副作用。M_AGENT-5 会补确认流。
//! - 工具入参/出参均为 `serde_json::Value`，零 schema 强制（呼应开放接入戒律）。

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::data_dir;
use crate::domain::{LorebookService, StateService};
use crate::types::{CharacterId, PresetId};
use airp_state_protocol::Capability;

mod character;
mod params;
mod session;
#[cfg(test)]
mod tests;

// PR 3 工具仍留在 facade，但它们依赖已移到 `params.rs` 的 helper。
// 这里 `use` 进 facade 命名空间，让 PR 3 工具的 inline 调用继续解析，
// 不改 PR 3 工具一行代码（"先移动"原则；PR 3 会再迁这些工具）。
use params::{optional_session_id, required_character_id};

/// 工具副作用分类（驱动 dry-run / 确认流 / 幂等去重）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffect {
    /// 纯读，零副作用。
    Readonly,
    /// 写入/创建/更新，幂等或可安全重试。
    Mutate,
    /// 删除/覆盖/回滚，默认 dry-run，需显式确认。
    Destructive,
    /// 仅追加（JSONL append 等），幂等键去重适用。
    Append,
}

/// 工具元数据，对应 MCP `ToolAnnotations` 的子集。
#[derive(Debug, Clone, Serialize)]
pub struct ToolMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub side_effect: ToolSideEffect,
}

/// 工具执行结果。
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    /// 工具返回值（任意 JSON）。
    pub output: Value,
    /// 是否为 dry-run（破坏性工具未确认时为 true）。
    pub dry_run: bool,
}

/// 工具 trait。`call` 是异步 boxed Future（动态分发足够，工具非热路径）。
pub trait Tool: Send + Sync {
    fn meta(&self) -> ToolMeta;

    /// 执行工具。
    ///
    /// `confirm` 对破坏性工具语义：`false` → dry-run（只描述将做什么，不落副作用）；
    /// `true` → 真执行。readonly / mutate / append 工具忽略 `confirm`。
    fn call(
        &self,
        params: Value,
        confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>>;
}

/// Tool registry assembled from built-ins; future MCP sources must pass the
/// same name-collision and authorization gates.
#[derive(Default)]
pub struct ToolRegistry {
    tools: std::collections::HashMap<&'static str, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册工具。重名视为注册表组装期的编程错误 → 返回 `Err`，绝不静默覆盖
    /// （旧实现 `insert` 会悄悄顶掉同名工具，掩盖冲突；issue #24）。
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), AirpError> {
        let name = tool.meta().name;
        if self.tools.contains_key(name) {
            return Err(AirpError::Config(format!(
                "duplicate tool registration: {}",
                name
            )));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<ToolMeta> {
        let mut tools: Vec<_> = self.tools.values().map(|t| t.meta()).collect();
        tools.sort_by_key(|tool| tool.name);
        tools
    }

    /// Engine-authoritative gate. The model can only select registered tools,
    /// the caller must explicitly grant `call:tool`, and an optional allowlist
    /// can further reduce the surface for a run.
    pub fn allowed(
        &self,
        name: &str,
        capabilities: &[Capability],
        allowlist: Option<&[String]>,
    ) -> bool {
        self.tools.contains_key(name)
            && capabilities.contains(&Capability::CallTool)
            && allowlist.is_none_or(|allowed| allowed.iter().any(|item| item == name))
    }
}

// ── mock 工具：echo（M_AGENT-1 验收用）─────────────────────────────────────

/// Echo：原样回传入参，标注 dry_run=false。用于验证 loop → 工具 → 回灌闭环。
pub struct EchoTool;

impl Tool for EchoTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "echo",
            description:
                "M_AGENT-1 mock: returns its input verbatim. Verifies loop→tool→subagent wiring.",
            side_effect: ToolSideEffect::Readonly,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        Box::pin(async move {
            Ok(ToolResult {
                output: params,
                dry_run: false,
            })
        })
    }
}

/// Construct the built-in registry used by the structured Agent loop.
///
/// `state` 让 built-in 工具访问数据层（`data_root`）。echo 等无状态工具
/// 忽略它。调用方（`AgentLoop::new`）已有 `Arc<DaemonState>`，传入即可。
///
/// 注册顺序：echo → session family → character family → state/lorebook/volume
/// → analysis。family 内顺序由各 `register` fn 内部决定，但 `ToolRegistry::list`
/// 最终按 name 字典序输出，故注册顺序不影响 `/v1/agent/tools` 响应。
pub fn default_registry(state: Arc<DaemonState>) -> ToolRegistry {
    // 内建工具集是编译期固定的、名字不重复的集合；若这里冒出重名，那是新增
    // 工具时的编程错误，应在启动时立刻炸出来，而非静默覆盖（issue #24）。
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(EchoTool))
        .expect("built-in tool name collision");
    // M_AGENT-2 第一批：会话类 5 工具。
    session::register(&mut reg, state.clone());
    // M_AGENT-2 第二批：角色类 3 工具（list/get/delete）。
    character::register(&mut reg, state.clone());
    // M_AGENT-2 第三批：state + lorebook（PR 3 将拆出，本 PR 保留在 facade）。
    reg.register(Box::new(GetCharacterStateTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(UpdateCharacterStateTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(GetLorebookTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(UpdateLorebookTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(ApplyLorebookTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(MergeLorebooksTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(SealVolumeTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(ExportContextBundleTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    // Decompose Agent Flow（Task 4）：analysis enhance/apply 工具。
    reg.register(Box::new(EnhanceAnalysisTool {
        state: state.clone(),
    }))
    .expect("built-in tool name collision");
    reg.register(Box::new(ApplyEnhancedAnalysisTool { state }))
        .expect("built-in tool name collision");
    reg
}

// ── PR 3 范围：state / lorebook / volume / context / analysis 工具 ─────────
//
// 这批工具按 #155 计划将在 PR 3 拆到 `tools/state_lorebook.rs` /
// `tools/volume_context.rs` / `tools/analysis.rs`。本 PR 保留在 facade，
// 不改一行实现，只通过 `use params::{required_character_id, optional_session_id}`
// 让它们继续解析已移到 `params.rs` 的 helper。

fn read_lorebook_or_empty(
    data_root: &std::path::Path,
    character: &CharacterId,
) -> Result<crate::orchestrator::Lorebook, AirpError> {
    match LorebookService::new(data_root).read(character) {
        Ok(lorebook) => Ok(lorebook),
        Err(AirpError::NotFound(_)) => Ok(crate::orchestrator::Lorebook {
            entries: Vec::new(),
        }),
        Err(error) => Err(error),
    }
}

struct GetCharacterStateTool {
    state: Arc<DaemonState>,
}

impl Tool for GetCharacterStateTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_character_state",
            description: "Read a character's current state/live.json.",
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
            let character = required_character_id(&params)?;
            let path =
                data_dir::char_state_dir(&state.data_root, character.as_str()).join("live.json");
            if !path.exists() {
                return Err(AirpError::NotFound(format!(
                    "state for {character} not found"
                )));
            }
            Ok(ToolResult {
                output: serde_json::from_slice(&std::fs::read(path)?)?,
                dry_run: false,
            })
        })
    }
}

struct UpdateCharacterStateTool {
    state: Arc<DaemonState>,
}

impl Tool for UpdateCharacterStateTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "update_character_state",
            description:
                "Validate and replace a character's live state, creating a revisioned snapshot.",
            side_effect: ToolSideEffect::Mutate,
        }
    }
    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let daemon = self.state.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let value = params
                .get("state")
                .ok_or_else(|| AirpError::BadRequest("missing state".to_string()))?;
            let snapshot = StateService::new(&daemon.data_root).write(&character, value)?;
            Ok(ToolResult {
                output: serde_json::to_value(snapshot)?,
                dry_run: false,
            })
        })
    }
}

struct GetLorebookTool {
    state: Arc<DaemonState>,
}

impl Tool for GetLorebookTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_lorebook",
            description: "Read the normalized AIRP v1 lorebook for a character.",
            side_effect: ToolSideEffect::Readonly,
        }
    }
    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let daemon = self.state.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let lorebook = LorebookService::new(&daemon.data_root).read(&character)?;
            Ok(ToolResult {
                output: serde_json::to_value(lorebook)?,
                dry_run: false,
            })
        })
    }
}

struct UpdateLorebookTool {
    state: Arc<DaemonState>,
}

impl Tool for UpdateLorebookTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "update_lorebook",
            description: "Replace a character's lorebook. Accepts AIRP canonical or SillyTavern form; normalizes via shared WorldbookNormalizer.",
            side_effect: ToolSideEffect::Destructive,
        }
    }
    fn call(
        &self,
        params: Value,
        confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let daemon = self.state.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let raw = params
                .get("lorebook")
                .cloned()
                .ok_or_else(|| AirpError::BadRequest("missing lorebook".to_string()))?;
            let (lorebook, report) = crate::orchestrator::normalize_worldbook(&raw);
            if let Some(reason) = report.replacement_error() {
                return Err(AirpError::BadRequest(format!("invalid lorebook: {reason}")));
            }
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "character_id": character.as_str(),
                        "action": "update_lorebook",
                        "entries": lorebook.entries.len(),
                        "import_report": report,
                        "requires": "confirm=true"
                    }),
                    dry_run: true,
                });
            }
            LorebookService::new(&daemon.data_root).write(&character, &lorebook)?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "updated": character.as_str(),
                    "entries": lorebook.entries.len(),
                    "import_report": report
                }),
                dry_run: false,
            })
        })
    }
}

struct ApplyLorebookTool {
    state: Arc<DaemonState>,
}

impl Tool for ApplyLorebookTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "apply_lorebook",
            description: "Return enabled lorebook entries triggered by the supplied text.",
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
            let character = required_character_id(&params)?;
            let text = params
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("missing text".to_string()))?;
            let lorebook = read_lorebook_or_empty(&state.data_root, &character)?;
            let context = lorebook.trigger(text);
            let output = crate::context_limit::truncate_for_context(&context);
            Ok(ToolResult {
                output: serde_json::json!({
                    "character_id": character.as_str(),
                    "matched": !context.is_empty(),
                    "context": output,
                    "truncated": context.len() > crate::context_limit::max_read_bytes(),
                }),
                dry_run: false,
            })
        })
    }
}

struct MergeLorebooksTool {
    state: Arc<DaemonState>,
}

impl Tool for MergeLorebooksTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "merge_lorebooks",
            description:
                "Merge character lorebooks without writing them; strategy is union or primary_only.",
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
            let raw_ids = params
                .get("character_ids")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AirpError::BadRequest("character_ids must be a non-empty array".to_string())
                })?;
            if raw_ids.is_empty() {
                return Err(AirpError::BadRequest(
                    "character_ids must be a non-empty array".to_string(),
                ));
            }
            let characters: Vec<CharacterId> = raw_ids
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| {
                            AirpError::BadRequest(
                                "character_ids entries must be strings".to_string(),
                            )
                        })
                        .and_then(CharacterId::new)
                })
                .collect::<Result<_, _>>()?;
            let strategy = params
                .get("strategy")
                .and_then(Value::as_str)
                .unwrap_or("union");
            if !matches!(strategy, "union" | "primary_only") {
                return Err(AirpError::BadRequest(
                    "strategy must be union or primary_only".to_string(),
                ));
            }

            let lorebooks = if strategy == "primary_only" {
                vec![read_lorebook_or_empty(&state.data_root, &characters[0])?]
            } else {
                characters
                    .iter()
                    .map(|character| read_lorebook_or_empty(&state.data_root, character))
                    .collect::<Result<Vec<_>, _>>()?
            };
            let merged = crate::orchestrator::merge_lorebooks(&lorebooks);
            let serialized = serde_json::to_string_pretty(&merged)?;
            let output = crate::context_limit::truncate_with_notice(
                &serialized,
                "merged lorebook exceeds the single-read cap; query source characters separately",
            );
            Ok(ToolResult {
                output: serde_json::json!({
                    "strategy": strategy,
                    "characters": characters.iter().map(CharacterId::as_str).collect::<Vec<_>>(),
                    "entries": merged.entries.len(),
                    "lorebook_json": output,
                    "truncated": serialized.len() > crate::context_limit::max_read_bytes(),
                }),
                dry_run: false,
            })
        })
    }
}

struct SealVolumeTool {
    state: Arc<DaemonState>,
}

impl Tool for SealVolumeTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "seal_volume",
            description: "Summarize current session memory into the next volume and clear current.md. Destructive — dry-run unless confirmed.",
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
            let character = required_character_id(&params)?;
            data_dir::get_character_card(&state.data_root, &character)?;
            let session_id = optional_session_id(&params)?;
            let session_dir = data_dir::resolve_session_dir(
                &state.data_root,
                character.as_str(),
                session_id.as_ref(),
            )?;
            let current = crate::volume_store::read_current(&session_dir)?;
            let next_volume = crate::volume_store::next_volume_number(&session_dir);
            let preview = serde_json::json!({
                "character_id": character.as_str(),
                "session_id": session_id.as_ref().map(ToString::to_string),
                "next_volume": next_volume,
                "current_bytes": current.len(),
                "current_tokens_estimated": crate::volume_store::estimate_tokens(&current),
            });
            if current.trim().is_empty() {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "sealed": false,
                        "reason": "current memory is empty",
                        "preview": preview,
                    }),
                    dry_run: true,
                });
            }
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "action": "seal_volume",
                        "preview": preview,
                        "requires": "confirm=true",
                    }),
                    dry_run: true,
                });
            }
            let (provider, params) = {
                let config = state
                    .config
                    .read()
                    .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
                let volume = &config.volume_config;
                (
                    Arc::new(crate::adapter::ProviderConfig {
                        provider: config.provider.clone(),
                        endpoint: config.endpoint.clone(),
                        api_key: config.api_key.clone(),
                    }),
                    crate::adapter::GenerationParams {
                        model: volume
                            .seal_model
                            .clone()
                            .unwrap_or_else(|| config.model.clone()),
                        temperature: Some(volume.seal_temperature),
                        max_tokens: None,
                    },
                )
            };
            let written_volume = crate::volume_manager::run_seal_flow(
                &state.http_client,
                &session_dir,
                provider,
                params,
            )
            .await?
            .ok_or_else(|| {
                AirpError::Volume("current memory became empty before sealing".into())
            })?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "sealed": true,
                    "character_id": character.as_str(),
                    "session_id": session_id.as_ref().map(ToString::to_string),
                    "volume": written_volume,
                    "file": format!("volumes/vol_{written_volume:03}.md"),
                }),
                dry_run: false,
            })
        })
    }
}

struct ExportContextBundleTool {
    state: Arc<DaemonState>,
}

impl Tool for ExportContextBundleTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "export_context_bundle",
            description: "Write a bounded generic-Markdown context bundle for an isolated subagent under the AIRP data root.",
            side_effect: ToolSideEffect::Mutate,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let preset_id = params
                .get("preset_id")
                .and_then(Value::as_str)
                .map(PresetId::new)
                .transpose()?;
            let include_lorebook = params
                .get("include_lorebook")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let thinking_mode = params
                .get("thinking_mode_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty());

            let card = data_dir::get_character_card(&state.data_root, &character)?;
            let raw_card = data_dir::read_character_card_text(&state.data_root, &character)?;
            let normalized = crate::orchestrator::card::normalize_v1_to_v2(&raw_card);
            let orchestrator = crate::orchestrator::Orchestrator::new(Some(&normalized), None)?;
            let character_name = card
                .get("data")
                .and_then(|data| data.get("name"))
                .or_else(|| card.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(character.as_str());

            let stable_prompt =
                orchestrator.build_system_prompt("User", &std::collections::HashMap::new(), "");
            let mut context = format!(
                "# RP Context Bundle: {character_name}\n\n> Feed this to an ISOLATED subagent as its system context. A fresh context lets the persona dominate instead of competing with the orchestrator's coding register. Generic Markdown only; wrap it in the host's own skill shape when needed.\n\n---\n\n"
            );
            if let Some(text) = thinking_mode {
                context.push_str("## Thinking mode (verbatim; AIRP does not interpret)\n");
                context.push_str(text);
                context.push_str("\n\n");
            }
            context.push_str("## Stable character context\n");
            context.push_str(&stable_prompt);

            if include_lorebook {
                let lorebook = LorebookService::new(&state.data_root).read(&character)?;
                let mut enabled: Vec<_> = lorebook
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.enabled.unwrap_or(true))
                    .collect();
                enabled.sort_by_key(|(index, entry)| {
                    (std::cmp::Reverse(entry.priority.unwrap_or(10)), *index)
                });
                if !enabled.is_empty() {
                    context.push_str("\n\n## World knowledge\n");
                    for (_, entry) in enabled {
                        context.push('\n');
                        context.push_str(&entry.content);
                        context.push('\n');
                    }
                }
            }

            let state_path =
                data_dir::char_state_dir(&state.data_root, character.as_str()).join("live.json");
            let live_state = match tokio::fs::read_to_string(&state_path).await {
                Ok(state) => Some(state),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };

            let bundle_dir =
                data_dir::ensure_context_bundle_dir(&state.data_root, character.as_str())?;
            for stale in ["preset_raw.json", "extensions.json"] {
                let path = bundle_dir.join(stale);
                if tokio::fs::try_exists(&path).await? {
                    tokio::fs::remove_file(path).await?;
                }
            }

            let mut files = vec!["context.md".to_string()];
            if let Some(preset) = preset_id.as_ref() {
                let raw_path = data_dir::preset_json_path(&state.data_root, preset.as_str());
                if !tokio::fs::try_exists(&raw_path).await? {
                    return Err(AirpError::NotFound(format!(
                        "preset {} has no preset.json",
                        preset
                    )));
                }
                tokio::fs::copy(raw_path, bundle_dir.join("preset_raw.json")).await?;
                files.push("preset_raw.json".to_string());
                context.push_str("\n> `preset_raw.json` is verbatim passthrough; AIRP does not interpret its prompts.\n");
            }

            let extensions = card
                .get("data")
                .and_then(|data| data.get("extensions"))
                .or_else(|| card.get("extensions"));
            if let Some(extensions) = extensions.filter(|value| {
                !value.is_null() && value.as_object().is_none_or(|object| !object.is_empty())
            }) {
                tokio::fs::write(
                    bundle_dir.join("extensions.json"),
                    serde_json::to_vec_pretty(extensions)?,
                )
                .await?;
                files.push("extensions.json".to_string());
                context.push_str("\n> `extensions.json` is raw bundled-card passthrough; AIRP does not interpret it.\n");
            }

            if let Some(live_state) = live_state {
                context.push_str(
                    "\n\n## Current state (volatile; keep after stable context)\n```json\n",
                );
                context.push_str(live_state.trim());
                context.push_str("\n```\n");
            }

            let context_bytes = context.len();
            let stored_context = crate::context_limit::truncate_for_context(&context);
            tokio::fs::write(bundle_dir.join("context.md"), &stored_context).await?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "character_id": character.as_str(),
                    "bundle_path": format!("exports/context-bundles/{}", character.as_str()),
                    "files": files,
                    "context_bytes": context_bytes,
                    "stored_bytes": stored_context.len(),
                    "truncated": context_bytes > crate::context_limit::max_read_bytes(),
                }),
                dry_run: false,
            })
        })
    }
}

// ── Decompose Agent Flow：analysis 增强 / 应用 工具（Task 6） ────────────────
//
// 对应计划 `docs/superpowers/plans/2026-07-07-decompose-agent-flow.md`。
// A1 修复：enhance 只读返回 diff 预览，不写盘；apply 是 destructive → dry-run 默认。
// A2 修复：world_book/ 开头的文件名拒绝（世界书只读，不参与 enhance）。
//
// L3 修复（issue #92）：enhance 真正调 LLM 增强 analysis MD。
// A3 修复：不调 `state.adapter`（DaemonState 无此字段，计划书 placeholder 已规避），
// 改用 `state.http_client` + `state.config` + `call_streaming_api_auto`，
// 与 chat_pipeline 同路径。LLM 输出原样作为 enhanced_md 返回，apply 端点二次确认写盘。

/// enhance 专用 system prompt：指示 LLM 增强 analysis MD，保留结构、补全占位符。
/// pub 以便 daemon `enhance_md_via_llm` 复用同一份，避免两条路径产物漂移（审计 G2/G3）。
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
pub(crate) async fn enhance_md_via_llm_shared(
    state: &Arc<DaemonState>,
    original_md: &str,
    filename: &str,
) -> Result<String, AirpError> {
    use crate::adapter::{call_streaming_api_auto, ChatMessage, GenerationParams, MessageRole};
    use futures_util::StreamExt;

    let (provider_config, gen_params, engine) = {
        let cfg = state
            .config
            .read()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
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
/// readonly。A2：拒绝 world_book/ 前缀。
/// L3：真正调 LLM（call_streaming_api_auto，与 chat_pipeline 同路径）。
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
            let cid_str = params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let filename = params
                .get("filename")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing filename".into()))?;

            // A2 修复：世界书条目只读，不参与 enhance
            if filename.starts_with("world_book/") {
                return Err(AirpError::BadRequest(
                    "world_book entries are read-only and not eligible for enhance (issue #87)"
                        .into(),
                ));
            }

            let cid = CharacterId::new(cid_str)?;
            let path = data_dir::char_analysis_file_path(&state.data_root, cid.as_str(), filename)?;
            if !path.exists() {
                return Err(AirpError::NotFound(format!(
                    "analysis file {} not found for character {}",
                    filename, cid
                )));
            }
            let original_md = std::fs::read_to_string(&path)?;

            // L3：调共享 helper 增强 MD（审计 CR5：抽公共逻辑防两路径漂移）。
            let enhanced_md = enhance_md_via_llm_shared(&state, &original_md, filename).await?;
            let has_changes = enhanced_md != original_md.trim();
            Ok(ToolResult {
                output: serde_json::json!({
                    "filename": filename,
                    "original_md": original_md,
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
/// A2：拒绝 world_book/ 前缀。
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
            let cid_str = params
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

            // A2 修复：世界书条目不可 apply
            if filename.starts_with("world_book/") {
                return Err(AirpError::BadRequest(
                    "world_book entries are read-only and not eligible for enhance (issue #87)"
                        .into(),
                ));
            }

            let cid = CharacterId::new(cid_str)?;
            let path = data_dir::char_analysis_file_path(&state.data_root, cid.as_str(), filename)?;
            if !path.exists() {
                return Err(AirpError::NotFound(format!(
                    "analysis file {} not found for character {}",
                    filename, cid
                )));
            }

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

            tokio::fs::write(&path, enhanced_md).await?;
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
