//! Engine-authoritative Agent tool execution surface.
//!
//! 协调器（`AgentLoop`）在每一步选择"派生纯净 subagent / 调一个工具 / 收敛结束"。
//! 本模块定义工具抽象、注册表、capability/allowlist 门和内建 domain tools。
//!
//! ## 设计纪律（计划书 §2.1 第 4 条：工具最小授权）
//! - 每个工具带 [`ToolMeta`]：`readonly` / `mutate` / `destructive` / `append`。
//! - **破坏性工具默认 dry-run**：[`Tool::call`] 接收 `confirm: bool`，未确认时
//!   破坏性工具只回"将执行什么"的描述，不落副作用。M_AGENT-5 会补确认流。
//! - 工具入参/出参均为 `serde_json::Value`，零 schema 强制（呼应开放接入戒律）。
//!
//! M_AGENT-2 会把 Core 已有进程内数据操作（chat_store / volume_* / orchestrator /
//! scene / preset_regex / png_parser）包成 built-in 工具；M_AGENT-3 会合并 MCP
//! upstream 工具。本骨架仅含 echo，验证 loop → 工具 → subagent 闭环。

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::adapter::{ChatMessage, MessageRole};
#[cfg(test)]
use crate::chat_store::ChatLog;
use crate::daemon::DaemonState;
use crate::data_dir;
use crate::domain::{ChatService, LorebookService, StateService};
use crate::types::{CharacterId, PresetId, SessionId};
use airp_state_protocol::Capability;

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
    tools: HashMap<&'static str, Box<dyn Tool>>,
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
pub fn default_registry(state: Arc<DaemonState>) -> ToolRegistry {
    // 内建工具集是编译期固定的、名字不重复的集合；若这里冒出重名，那是新增
    // 工具时的编程错误，应在启动时立刻炸出来，而非静默覆盖（issue #24）。
    const COLLISION: &str = "built-in tool name collision";
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(EchoTool)).expect(COLLISION);
    // M_AGENT-2 第一批：会话类 5 工具。
    reg.register(Box::new(ListSessionsTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(StartSessionTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(AppendMessageTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetRecentContextTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(RollbackMessagesTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    // M_AGENT-2 第二批：角色类 3 工具（list/get/delete）。
    reg.register(Box::new(ListCharactersTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetCharacterTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(DeleteCharacterTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetCharacterStateTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(UpdateCharacterStateTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetLorebookTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(UpdateLorebookTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ApplyLorebookTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(MergeLorebooksTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(SealVolumeTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ExportContextBundleTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    // Decompose Agent Flow（Task 4）：analysis enhance/apply 工具。
    reg.register(Box::new(EnhanceAnalysisTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ApplyEnhancedAnalysisTool { state }))
        .expect(COLLISION);
    reg
}

fn required_character_id(params: &Value) -> Result<CharacterId, AirpError> {
    let value = params
        .get("character_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AirpError::BadRequest("missing character_id".to_string()))?;
    CharacterId::new(value)
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
            description: "Replace a character's normalized AIRP v1 lorebook.",
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
            let lorebook: crate::orchestrator::Lorebook = serde_json::from_value(raw)?;
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "character_id": character.as_str(),
                        "action": "update_lorebook",
                        "entries": lorebook.entries.len(),
                        "requires": "confirm=true"
                    }),
                    dry_run: true,
                });
            }
            LorebookService::new(&daemon.data_root).write(&character, &lorebook)?;
            Ok(ToolResult {
                output: serde_json::json!({"updated": character.as_str(), "entries": lorebook.entries.len()}),
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

const MAX_RECENT_CONTEXT: usize = 200;

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

fn optional_session_id(params: &Value) -> Result<Option<SessionId>, AirpError> {
    match params.get("session_id") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let raw = v
                .as_str()
                .ok_or_else(|| AirpError::BadRequest("session_id must be a string".into()))?;
            Ok(Some(SessionId::parse(raw)?))
        }
    }
}

fn required_usize_param(params: &Value, key: &str) -> Result<usize, AirpError> {
    let raw = params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| AirpError::BadRequest(format!("missing {}", key)))?;
    usize::try_from(raw)
        .map_err(|_| AirpError::BadRequest(format!("{} {} exceeds platform usize", key, raw)))
}

fn optional_usize_param(params: &Value, key: &str, default: usize) -> Result<usize, AirpError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => required_usize_param(params, key),
    }
}

// ── M_AGENT-2：会话类 built-in 工具 ─────────────────────────────────────────
//
// 这批工具把 engine 已有的 chat_store / data_dir::session 能力暴露给 agent
// loop，让协调器能自主管会话（列/开/追/读/回滚）。对应 MCP-Server 工具面
// §1 的"会话"行（MCP-SERVER-ABSORPTION.md）。每个工具自携 `Arc<DaemonState>`
// 访问数据层——不改 `Tool` trait 签名（EchoTool 等无状态工具不受影响）。
//
// 设计纪律（守不变式 #3 工具受控）：
// - append returns the persisted position so callers can record their own idempotency keys;
// - rollback 是 destructive → 默认 dry-run，未 confirm 只回"将回滚到 idx N"；
// - 入参/出参均 serde_json::Value，schema 不强约束（开放接入戒律）；
// - 错误透传 AirpError，agent loop 已有 ToolCall failed 分支。

/// `list_sessions`：列某角色的所有命名会话。readonly。
/// params: `{ "character_id": string }` → `[{ "session_id": string }]`
struct ListSessionsTool {
    state: Arc<DaemonState>,
}

impl Tool for ListSessionsTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "list_sessions",
            description: "List all named sessions for a character.",
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
            let cid = CharacterId::new(cid_str)?;
            let sessions = ChatService::new(&state.data_root).list_sessions(&cid)?;
            let out: Vec<Value> = sessions
                .into_iter()
                .map(|s| serde_json::json!({ "session_id": s.to_string() }))
                .collect();
            Ok(ToolResult {
                output: Value::Array(out),
                dry_run: false,
            })
        })
    }
}

/// `start_session`：为角色创建一个新命名会话（自动生成 UUID session_id）。
/// mutate（创建目录 + meta）。session_id 由数据层生成，不接受自定义
/// （`data_dir::create_session` 当前只生成 UUID；未来需要自定义 id 再扩）。
/// params: `{ "character_id": string }`
/// → `{ "session_id": string, "character_id": string }`
struct StartSessionTool {
    state: Arc<DaemonState>,
}

impl Tool for StartSessionTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "start_session",
            description:
                "Create a new named session for a character. session_id is auto-generated (UUID).",
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
            let cid_str = params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let cid = CharacterId::new(cid_str)?;
            let sid = ChatService::new(&state.data_root).create_session(&cid)?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "session_id": sid.to_string(),
                    "character_id": cid.to_string(),
                }),
                dry_run: false,
            })
        })
    }
}

/// `append_message`：向角色当前会话追加一条消息。append（JSONL O(1) 写）。
/// params: `{ "character_id": string, "role": "user"|"assistant"|"system", "content": string }`
/// → `{ "index": number, "total": number }`（追加后的索引与总条数）
struct AppendMessageTool {
    state: Arc<DaemonState>,
}

impl Tool for AppendMessageTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "append_message",
            description: "Append a message to the character's current chat log. role ∈ {user,assistant,system}.",
            side_effect: ToolSideEffect::Append,
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
            let role_str = params
                .get("role")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing role".into()))?;
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing content".into()))?;
            let role = match role_str {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "system" => MessageRole::System,
                other => {
                    return Err(AirpError::BadRequest(format!(
                        "invalid role: {} (expect user|assistant|system)",
                        other
                    )));
                }
            };
            let cid = CharacterId::new(cid_str)?;
            let session_id = optional_session_id(&params)?;
            let service = ChatService::new(&state.data_root);
            if role == MessageRole::System {
                tracing::info!(
                    character_id = %cid,
                    session_id = session_id.map(|sid| sid.to_string()).as_deref().unwrap_or("default"),
                    "append_message writes a system message"
                );
            }
            let (log, total_before) = service.append(
                &cid,
                session_id.as_ref(),
                ChatMessage {
                    role,
                    content: content.to_string(),
                },
            )?;
            let total = log.messages.len();
            let truncated_count = total_before.saturating_add(1).saturating_sub(total);
            let index = total.checked_sub(1).ok_or_else(|| {
                AirpError::Internal("append_message produced an empty log".into())
            })?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "index": index,
                    "total": total,
                    "truncated": truncated_count > 0,
                    "truncated_count": truncated_count,
                    "session_id": session_id.map(|sid| sid.to_string()),
                }),
                dry_run: false,
            })
        })
    }
}

/// `get_recent_context`：取角色最近 N 条消息。readonly。
/// params: `{ "character_id": string, "n"?: number }`（n 默认 20）
/// → `{ "messages": [{ "role": string, "content": string }] }`
struct GetRecentContextTool {
    state: Arc<DaemonState>,
}

impl Tool for GetRecentContextTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_recent_context",
            description: "Get the most recent N messages of a character's chat log (default N=20).",
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
            let n = optional_usize_param(&params, "n", 20)?;
            if n > MAX_RECENT_CONTEXT {
                return Err(AirpError::BadRequest(format!(
                    "n {} exceeds max {}",
                    n, MAX_RECENT_CONTEXT
                )));
            }
            let cid = CharacterId::new(cid_str)?;
            let session_id = optional_session_id(&params)?;
            let recent = ChatService::new(&state.data_root).recent(&cid, session_id.as_ref(), n)?;
            let msgs: Vec<Value> = recent
                .into_iter()
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect();
            Ok(ToolResult {
                output: serde_json::json!({
                    "messages": msgs,
                    "session_id": session_id.map(|sid| sid.to_string()),
                }),
                dry_run: false,
            })
        })
    }
}

/// `rollback_messages`：回滚角色会话到指定索引（保留 0..=index）。
/// **destructive** → 未 confirm 时 dry-run，只回"将回滚到 idx N，丢弃 M 条"。
/// params: `{ "character_id": string, "index": number }`
/// confirm=true → 真回滚。
/// → `{ "rolled_back_to": number, "dropped": number }`（dry_run=true 时 dropped 为预览）
struct RollbackMessagesTool {
    state: Arc<DaemonState>,
}

impl Tool for RollbackMessagesTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "rollback_messages",
            description: "Rollback the chat log to keep only messages [0..=index]. Destructive — dry-run unless confirm=true.",
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
            let index = required_usize_param(&params, "index")?;
            let cid = CharacterId::new(cid_str)?;
            let session_id = optional_session_id(&params)?;
            let service = ChatService::new(&state.data_root);
            let dropped = service.rollback_preview(&cid, session_id.as_ref(), index)?;
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "rolled_back_to": index,
                        "dropped": dropped,
                        "session_id": session_id.map(|sid| sid.to_string()),
                        "preview": "pass confirm=true to execute",
                    }),
                    dry_run: true,
                });
            }
            tracing::warn!(
                character_id = %cid,
                session_id = session_id.map(|sid| sid.to_string()).as_deref().unwrap_or("default"),
                index,
                dropped,
                "rollback_messages executed"
            );
            let _ = service.rollback(&cid, session_id.as_ref(), index)?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "rolled_back_to": index,
                    "dropped": dropped,
                    "session_id": session_id.map(|sid| sid.to_string()),
                }),
                dry_run: false,
            })
        })
    }
}

// ── M_AGENT-2 第二批：角色类 3 工具 ─────────────────────────────────────────
//
// list/get/delete characters。list 是 readonly；get 是 readonly（读 card.json）；
// delete 是 destructive（删整个角色目录）→ 默认 dry-run。
// 对应 MCP-SERVER-ABSORPTION.md §1 "角色" 行的 3 个 ✅ 工具。
// analyze_card / decompose_character 是 🆕 需移植，不在本批。

/// `list_characters`：列所有角色 id。readonly。
/// params: `{}` → `["alice", "bob", ...]`
struct ListCharactersTool {
    state: Arc<DaemonState>,
}

impl Tool for ListCharactersTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "list_characters",
            description: "List all available character ids (folder names under data/characters/).",
            side_effect: ToolSideEffect::Readonly,
        }
    }

    fn call(
        &self,
        _params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let ids = data_dir::list_characters(&state.data_root)?;
            Ok(ToolResult {
                output: Value::Array(ids.into_iter().map(Value::String).collect()),
                dry_run: false,
            })
        })
    }
}

/// `get_character`：读角色 card.json（原始 JSON 文本，解析后返回 object）。
/// readonly。兼容迁移后 `card/card.json` 与旧 `card.json`。
/// params: `{ "character_id": string }` → `{ "card": <parsed card.json object> }`
struct GetCharacterTool {
    state: Arc<DaemonState>,
}

impl Tool for GetCharacterTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_character",
            description: "Read a character's card.json as a parsed JSON object. Invalid JSON is reported as data corruption.",
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
            let cid = CharacterId::new(cid_str)?;
            let card = data_dir::get_character_card(&state.data_root, &cid)?;
            Ok(ToolResult {
                output: serde_json::json!({ "card": card }),
                dry_run: false,
            })
        })
    }
}

/// `delete_character`：删整个角色目录子树（所有 files under data/characters/{id}/）。
/// **destructive** → 默认 dry-run，未 confirm 只回结构化 preview。
/// params: `{ "character_id": string }` → `{ "deleted": string }` 或 dry-run preview
struct DeleteCharacterTool {
    state: Arc<DaemonState>,
}

impl Tool for DeleteCharacterTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "delete_character",
            description: "Delete a character's entire directory subtree (all files under data/characters/{id}/). Destructive — dry-run unless confirm=true.",
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
            let cid = CharacterId::new(cid_str)?;
            if !confirm {
                let exists = data_dir::list_characters(&state.data_root)?
                    .iter()
                    .any(|id| id == cid.as_str());
                if !exists {
                    return Err(AirpError::NotFound(format!(
                        "character {} does not exist",
                        cid
                    )));
                }
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "character_id": cid.to_string(),
                        "action": "delete_character",
                        "will_delete": ["card", "state", "memory", "sessions", "all future files under character subtree"],
                        "requires": "confirm=true",
                    }),
                    dry_run: true,
                });
            }
            // 破坏性删除：拿角色级写锁，独占排斥该角色下所有会话的 append/rollback
            // （它们持 read 锁）。旧实现用 per-session Mutex 的 key=character，与命名
            // 会话写的 key=character/{sid} 不互斥，故可交错半删（issue #22）。
            ChatService::new(&state.data_root).delete_character(&cid)?;
            tracing::warn!(character_id = %cid, "delete_character executed");
            Ok(ToolResult {
                output: serde_json::json!({ "deleted": cid.to_string() }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{BackendEngine, Provider};
    use crate::chat_store::MAX_MESSAGES;
    use crate::config::VolumeConfig;
    use crate::daemon::{DaemonState, MutableConfig};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// 最小可运行 DaemonState，data_root 指向临时目录（照 chat_pipeline/tests 模板）。
    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
            }),
        })
    }

    #[tokio::test]
    async fn session_tools_roundtrip_append_recent_rollback() {
        // 端到端：start → list → append×2 → recent → rollback(dry-run) → rollback(真)
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        // start_session
        let start = reg.get("start_session").unwrap();
        let r = start
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        assert!(!r.dry_run);
        assert!(r.output["session_id"].is_string());
        let session_id = r.output["session_id"].as_str().unwrap().to_string();

        // list_sessions → 至少 1
        let list = reg.get("list_sessions").unwrap();
        let r = list
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        let arr = r.output.as_array().unwrap();
        assert!(
            !arr.is_empty(),
            "list_sessions should find the started session"
        );

        // append_message ×2 (user + assistant)
        let append = reg.get("append_message").unwrap();
        for (role, content) in [("user", "hello"), ("assistant", "hi there")] {
            let r = append
                .call(
                    serde_json::json!({
                        "character_id": "alice",
                        "session_id": session_id.clone(),
                        "role": role,
                        "content": content,
                    }),
                    false,
                )
                .await
                .unwrap();
            assert!(r.output["total"].as_u64().unwrap() >= 1);
        }

        // get_recent_context n=10 → 2 条
        let recent = reg.get("get_recent_context").unwrap();
        let r = recent
            .call(
                serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "n": 10}),
                false,
            )
            .await
            .unwrap();
        let msgs = r.output["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["content"], "hi there");

        // rollback index=0 dry-run → dropped=1, dry_run=true
        let rb = reg.get("rollback_messages").unwrap();
        let r = rb
            .call(
                serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "index": 0}),
                false,
            )
            .await
            .unwrap();
        assert!(r.dry_run);
        assert_eq!(r.output["dropped"].as_u64().unwrap(), 1);

        // rollback index=0 confirm=true → 真回滚，剩 1 条
        let r = rb
            .call(
                serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "index": 0}),
                true,
            )
            .await
            .unwrap();
        assert!(!r.dry_run);
        let r = recent
            .call(
                serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "n": 10}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(r.output["messages"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn session_history_isolated_from_character_history() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state);

        let start = reg.get("start_session").unwrap();
        let session = start
            .call(serde_json::json!({"character_id": "scope"}), false)
            .await
            .unwrap()
            .output["session_id"]
            .as_str()
            .unwrap()
            .to_string();

        let append = reg.get("append_message").unwrap();
        append
            .call(
                serde_json::json!({"character_id": "scope", "role": "user", "content": "global"}),
                false,
            )
            .await
            .unwrap();
        append
            .call(
                serde_json::json!({
                    "character_id": "scope",
                    "session_id": session.clone(),
                    "role": "user",
                    "content": "session",
                }),
                false,
            )
            .await
            .unwrap();

        let recent = reg.get("get_recent_context").unwrap();
        let global = recent
            .call(serde_json::json!({"character_id": "scope", "n": 10}), false)
            .await
            .unwrap();
        assert_eq!(global.output["messages"][0]["content"], "global");

        let scoped = recent
            .call(
                serde_json::json!({"character_id": "scope", "session_id": session.clone(), "n": 10}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(scoped.output["messages"][0]["content"], "session");
    }

    #[tokio::test]
    async fn append_reports_full_history_index_after_context_threshold() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let mut log = ChatLog::load_or_create(&state.data_root, "overflow").unwrap();
        for i in 0..MAX_MESSAGES {
            log.append(
                &state.data_root,
                ChatMessage {
                    role: MessageRole::User,
                    content: format!("seed-{i}"),
                },
            )
            .unwrap();
        }

        let reg = default_registry(state);
        let append = reg.get("append_message").unwrap();
        let r = append
            .call(
                serde_json::json!({
                    "character_id": "overflow",
                    "role": "assistant",
                    "content": "after-cap",
                }),
                false,
            )
            .await
            .unwrap();

        assert_eq!(r.output["index"], MAX_MESSAGES);
        assert_eq!(r.output["total"], MAX_MESSAGES + 1);
        assert_eq!(r.output["truncated"], false);
        assert_eq!(r.output["truncated_count"], 0);
    }

    #[tokio::test]
    async fn recent_context_rejects_over_cap() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state);
        let recent = reg.get("get_recent_context").unwrap();
        let err = recent
            .call(
                serde_json::json!({"character_id": "cap", "n": MAX_RECENT_CONTEXT + 1}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn rollback_rejects_out_of_range_index() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state);
        let append = reg.get("append_message").unwrap();
        append
            .call(
                serde_json::json!({"character_id": "bob", "role": "user", "content": "x"}),
                false,
            )
            .await
            .unwrap();
        let rb = reg.get("rollback_messages").unwrap();
        let err = rb
            .call(
                serde_json::json!({"character_id": "bob", "index": 99}),
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn append_rejects_invalid_role() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state);
        let append = reg.get("append_message").unwrap();
        let err = append
            .call(
                serde_json::json!({"character_id": "cat", "role": "narrator", "content": "x"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn echo_still_works_after_registry_change() {
        // default_registry 改签名不应破坏 M_AGENT-1 的 echo
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let reg = default_registry(state);
        let echo = reg.get("echo").unwrap();
        let r = echo
            .call(serde_json::json!({"probe": "still-here"}), false)
            .await
            .unwrap();
        assert_eq!(r.output["probe"], "still-here");
    }

    #[test]
    fn default_registry_includes_expected_tool_names() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let reg = default_registry(state);

        for name in [
            "echo",
            "list_sessions",
            "start_session",
            "append_message",
            "get_recent_context",
            "rollback_messages",
            "list_characters",
            "get_character",
            "delete_character",
            "get_character_state",
            "update_character_state",
            "get_lorebook",
            "update_lorebook",
            "apply_lorebook",
            "merge_lorebooks",
            "seal_volume",
            "export_context_bundle",
            "enhance_analysis",
            "apply_enhanced_analysis",
        ] {
            assert!(reg.get(name).is_some(), "missing tool: {name}");
        }
    }

    #[tokio::test]
    async fn state_and_lorebook_tools_roundtrip_with_confirmation() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        let update_state = reg.get("update_character_state").unwrap();
        let updated = update_state
            .call(
                serde_json::json!({"character_id": "alice", "state": {"hp": 90}}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(updated.output["revision"], 1);

        let get_state = reg.get("get_character_state").unwrap();
        let current = get_state
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        assert_eq!(current.output["hp"], 90);

        let lorebook = serde_json::json!({
            "entries": [{
                "keys": ["AIRP"],
                "content": "Open runtime",
                "enabled": true,
                "priority": 10,
                "comment": null
            }]
        });
        let update_lorebook = reg.get("update_lorebook").unwrap();
        let preview = update_lorebook
            .call(
                serde_json::json!({"character_id": "alice", "lorebook": lorebook.clone()}),
                false,
            )
            .await
            .unwrap();
        assert!(preview.dry_run);
        assert_eq!(preview.output["requires"], "confirm=true");
        assert!(!crate::data_dir::char_world_lorebook_path(&state.data_root, "alice").exists());

        let written = update_lorebook
            .call(
                serde_json::json!({"character_id": "alice", "lorebook": lorebook}),
                true,
            )
            .await
            .unwrap();
        assert!(!written.dry_run);
        assert_eq!(written.output["entries"], 1);

        let get_lorebook = reg.get("get_lorebook").unwrap();
        let current = get_lorebook
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        assert_eq!(current.output["entries"][0]["content"], "Open runtime");
    }

    #[tokio::test]
    async fn lorebook_apply_and_merge_are_readonly() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let service = LorebookService::new(&state.data_root);
        for (character, entries) in [
            (
                "alice",
                vec![crate::orchestrator::LorebookEntry {
                    keys: vec!["moon".to_string()],
                    content: "Moon knowledge".to_string(),
                    enabled: Some(true),
                    priority: Some(20),
                    comment: None,
                }],
            ),
            (
                "bob",
                vec![
                    crate::orchestrator::LorebookEntry {
                        keys: vec!["moon".to_string()],
                        content: "Moon knowledge".to_string(),
                        enabled: Some(true),
                        priority: Some(20),
                        comment: None,
                    },
                    crate::orchestrator::LorebookEntry {
                        keys: vec!["gate".to_string()],
                        content: "Gate knowledge".to_string(),
                        enabled: Some(true),
                        priority: Some(10),
                        comment: None,
                    },
                ],
            ),
        ] {
            service
                .write(
                    &CharacterId::new(character).unwrap(),
                    &crate::orchestrator::Lorebook { entries },
                )
                .unwrap();
        }
        let reg = default_registry(state);

        let applied = reg
            .get("apply_lorebook")
            .unwrap()
            .call(
                serde_json::json!({"character_id": "alice", "text": "the moon rises"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(applied.output["matched"], true);
        assert!(applied.output["context"]
            .as_str()
            .unwrap()
            .contains("Moon knowledge"));

        let empty = reg
            .get("apply_lorebook")
            .unwrap()
            .call(
                serde_json::json!({"character_id": "charlie", "text": "moon"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(empty.output["matched"], false);

        let merged = reg
            .get("merge_lorebooks")
            .unwrap()
            .call(
                serde_json::json!({"character_ids": ["alice", "bob"], "strategy": "union"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(merged.output["entries"], 2);
        assert!(!merged.dry_run);

        let merged_with_missing = reg
            .get("merge_lorebooks")
            .unwrap()
            .call(
                serde_json::json!({"character_ids": ["alice", "charlie"], "strategy": "union"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(merged_with_missing.output["entries"], 1);
    }

    #[tokio::test]
    async fn export_context_bundle_output_directs_isolated_subagent() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let card_dir = state.data_root.join("characters/alice/card");
        std::fs::create_dir_all(&card_dir).unwrap();
        std::fs::write(
            card_dir.join("card.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "spec": "chara_card_v2",
                "spec_version": "2.0",
                "data": {
                    "name": "Alice",
                    "description": "A test character",
                    "personality": "Curious",
                    "scenario": "An observatory",
                    "extensions": {"depth_prompt": "raw extension"}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        StateService::new(&state.data_root)
            .write(
                &CharacterId::new("alice").unwrap(),
                &serde_json::json!({"hp": 9}),
            )
            .unwrap();
        LorebookService::new(&state.data_root)
            .write(
                &CharacterId::new("alice").unwrap(),
                &crate::orchestrator::Lorebook {
                    entries: vec![crate::orchestrator::LorebookEntry {
                        keys: vec!["observatory".to_string()],
                        content: "Stable world fact".to_string(),
                        enabled: Some(true),
                        priority: Some(10),
                        comment: None,
                    }],
                },
            )
            .unwrap();
        let preset_dir = state.data_root.join("presets/story");
        std::fs::create_dir_all(&preset_dir).unwrap();
        std::fs::write(preset_dir.join("preset.json"), r#"{"prompts":[]}"#).unwrap();

        let result = default_registry(state.clone())
            .get("export_context_bundle")
            .unwrap()
            .call(
                serde_json::json!({
                    "character_id": "alice",
                    "preset_id": "story",
                    "include_lorebook": true,
                    "thinking_mode_text": "Stay immersed"
                }),
                false,
            )
            .await
            .unwrap();
        assert!(!result.dry_run);
        let bundle = state.data_root.join("exports/context-bundles/alice");
        let context = std::fs::read_to_string(bundle.join("context.md")).unwrap();
        assert!(context.contains("ISOLATED subagent"));
        assert!(context.contains("fresh context"));
        assert!(context.contains("Stable world fact"));
        assert!(context.contains("\"hp\": 9"));
        assert!(
            context.find("Stable character context").unwrap()
                < context.find("Current state (volatile").unwrap()
        );
        assert!(bundle.join("preset_raw.json").exists());
        assert!(bundle.join("extensions.json").exists());
    }

    #[tokio::test]
    async fn seal_volume_dry_run_then_confirm() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let archive = "<卷索引>\n- 卷标题: Test\n</卷索引>\n<卷内容>\nArchived scene\n</卷内容>\n<全局index更新>\n</全局index更新>";
        let event = serde_json::json!({"choices": [{"delta": {"content": archive}}]});
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!("data: {event}\n\ndata: [DONE]\n\n")),
            )
            .mount(&server)
            .await;

        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        state.config.write().unwrap().endpoint = format!("{}/v1/chat/completions", server.uri());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let card_dir = state.data_root.join("characters/alice/card");
        std::fs::create_dir_all(&card_dir).unwrap();
        std::fs::write(card_dir.join("card.json"), r#"{"name":"Alice"}"#).unwrap();
        let memory = crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
        crate::volume_store::append_to_current(&memory, "A scene to archive").unwrap();
        let reg = default_registry(state);
        let tool = reg.get("seal_volume").unwrap();

        let preview = tool
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        assert!(preview.dry_run);
        assert_eq!(preview.output["requires"], "confirm=true");
        assert!(crate::volume_store::list_volume_numbers(&memory).is_empty());

        let sealed = tool
            .call(serde_json::json!({"character_id": "alice"}), true)
            .await
            .unwrap();
        assert!(!sealed.dry_run);
        assert_eq!(sealed.output["volume"], 1);
        assert_eq!(crate::volume_store::list_volume_numbers(&memory), vec![1]);
        assert!(crate::volume_store::read_current(&memory)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn registry_capability_and_allowlist_are_authoritative() {
        let tmp = tempdir().unwrap();
        let reg = default_registry(make_state(tmp.path().to_path_buf()));
        assert!(!reg.allowed("echo", &[], None));
        assert!(reg.allowed("echo", &[Capability::CallTool], None));
        assert!(!reg.allowed(
            "echo",
            &[Capability::CallTool],
            Some(&["list_characters".to_string()])
        ));
        assert!(!reg.allowed("not_registered", &[Capability::CallTool], None));
    }

    #[tokio::test]
    async fn character_tools_list_get_delete() {
        // 端到端：list(空) → 写 fixture card → list(1) → get → delete(dry-run) → delete(真) → list(空)
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        // 非法目录名不应出现在 list_characters 中，否则 list/get/delete 契约不对称。
        std::fs::create_dir_all(state.data_root.join("characters").join(".bad")).unwrap();

        // list 初始空
        let list = reg.get("list_characters").unwrap();
        let r = list.call(serde_json::json!({}), false).await.unwrap();
        assert_eq!(r.output.as_array().unwrap().len(), 0);

        // 写 fixture 角色卡
        let char_dir = state.data_root.join("characters").join("alice");
        std::fs::create_dir_all(char_dir.join("card")).unwrap();
        std::fs::write(
            char_dir.join("card").join("card.json"),
            r#"{"name":"Alice","description":"test char"}"#,
        )
        .unwrap();

        // list → 1
        let r = list.call(serde_json::json!({}), false).await.unwrap();
        assert_eq!(r.output.as_array().unwrap().len(), 1);
        assert_eq!(r.output[0], "alice");

        // get → card object
        let get = reg.get("get_character").unwrap();
        let r = get
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        assert_eq!(r.output["card"]["name"], "Alice");

        // get 不存在角色 → NotFound
        let err = get
            .call(serde_json::json!({"character_id": "ghost"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::NotFound(_)));

        // delete dry-run → preview, dry_run=true
        let del = reg.get("delete_character").unwrap();
        let r = del
            .call(serde_json::json!({"character_id": "alice"}), false)
            .await
            .unwrap();
        assert!(r.dry_run);
        assert_eq!(r.output["action"], "delete_character");
        assert_eq!(r.output["requires"], "confirm=true");
        assert!(r.output["will_delete"].is_array());
        assert!(
            char_dir.exists(),
            "dry-run must not delete the character dir"
        );

        // delete dry-run 对不存在角色也应报 NotFound，避免误导 agent 决策。
        let err = del
            .call(serde_json::json!({"character_id": "ghost"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::NotFound(_)));

        // delete confirm=true → 真删
        let r = del
            .call(serde_json::json!({"character_id": "alice"}), true)
            .await
            .unwrap();
        assert!(!r.dry_run);
        assert_eq!(r.output["deleted"], "alice");

        // list → 0
        let r = list.call(serde_json::json!({}), false).await.unwrap();
        assert_eq!(r.output.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_character_reads_legacy_card_json_path() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        let char_dir = state.data_root.join("characters").join("legacy");
        std::fs::create_dir_all(&char_dir).unwrap();
        std::fs::write(
            char_dir.join("card.json"),
            r#"{"name":"Legacy","description":"old layout"}"#,
        )
        .unwrap();

        let get = reg.get("get_character").unwrap();
        let r = get
            .call(serde_json::json!({"character_id": "legacy"}), false)
            .await
            .unwrap();

        assert_eq!(r.output["card"]["name"], "Legacy");
    }

    #[tokio::test]
    async fn get_character_rejects_invalid_card_json() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        let char_dir = state
            .data_root
            .join("characters")
            .join("broken")
            .join("card");
        std::fs::create_dir_all(&char_dir).unwrap();
        std::fs::write(char_dir.join("card.json"), "not json").unwrap();

        let get = reg.get("get_character").unwrap();
        let err = get
            .call(serde_json::json!({"character_id": "broken"}), false)
            .await
            .unwrap_err();

        assert!(matches!(err, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn enhance_analysis_returns_preview_and_rejects_world_book() {
        // L2 修复（issue #92）：用 wiremock mock LLM upstream。
        // L3：enhance 真正调 LLM，测试需 mock，否则烧 token + DNS 失败。
        // A1：enhance 只读返回 diff 预览，不写盘
        // A2：world_book/ 前缀拒绝
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let enhanced_content = "# Basic Info\n\nName: Alice\nDescription: A brave knight\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n\ndata: [DONE]\n\n",
                serde_json::to_string(enhanced_content).unwrap()
            )))
            .mount(&mock_server)
            .await;

        let tmp = tempdir().unwrap();
        let state = Arc::new(DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: format!("{}/v1/chat/completions", mock_server.uri()),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
            }),
        });
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        // 写一个 fixture analysis MD 文件
        let analysis_dir = state
            .data_root
            .join("characters")
            .join("alice")
            .join("analysis");
        std::fs::create_dir_all(&analysis_dir).unwrap();
        let original = "# Basic Info\n\nName: Alice\n";
        std::fs::write(analysis_dir.join("basic_info.md"), original).unwrap();

        let enhance = reg.get("enhance_analysis").unwrap();
        let r = enhance
            .call(
                serde_json::json!({"character_id": "alice", "filename": "basic_info.md"}),
                false,
            )
            .await
            .unwrap();
        assert!(!r.dry_run, "enhance is readonly, never dry-run");
        assert_eq!(r.output["filename"], "basic_info.md");
        assert_eq!(r.output["original_md"], original);
        // L3：enhanced_md 来自 LLM mock，has_changes=true
        // 注意：enhance 会 trim LLM 输出，故比较时用 trim
        assert_eq!(
            r.output["enhanced_md"].as_str().unwrap().trim(),
            enhanced_content.trim()
        );
        assert_eq!(r.output["has_changes"], true);

        // enhance 不写盘（A1：readonly）
        assert_eq!(
            std::fs::read_to_string(analysis_dir.join("basic_info.md")).unwrap(),
            original,
            "enhance is readonly — must not write to disk"
        );

        // A2: world_book/ 前缀拒绝
        let err = enhance
            .call(
                serde_json::json!({"character_id": "alice", "filename": "world_book/entry_001.md"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));

        // 不存在文件 → NotFound
        let err = enhance
            .call(
                serde_json::json!({"character_id": "alice", "filename": "ghost.md"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::NotFound(_)));
    }

    #[tokio::test]
    async fn apply_enhanced_analysis_dry_run_then_confirm() {
        // A1：apply 是 destructive → dry-run 默认，confirm=true 才写盘
        // A2：world_book/ 前缀拒绝
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

        let analysis_dir = state
            .data_root
            .join("characters")
            .join("alice")
            .join("analysis");
        std::fs::create_dir_all(&analysis_dir).unwrap();
        std::fs::write(analysis_dir.join("personality.md"), "old content").unwrap();

        let apply = reg.get("apply_enhanced_analysis").unwrap();
        let enhanced = "# Personality\n\nBrave and curious\n";

        // dry-run → 不写盘
        let r = apply
            .call(
                serde_json::json!({
                    "character_id": "alice",
                    "filename": "personality.md",
                    "enhanced_md": enhanced,
                }),
                false,
            )
            .await
            .unwrap();
        assert!(r.dry_run);
        assert_eq!(r.output["action"], "apply_enhanced_analysis");
        assert_eq!(r.output["requires"], "confirm=true");
        assert_eq!(
            std::fs::read_to_string(analysis_dir.join("personality.md")).unwrap(),
            "old content",
            "dry-run must not write to disk"
        );

        // confirm=true → 写盘
        let r = apply
            .call(
                serde_json::json!({
                    "character_id": "alice",
                    "filename": "personality.md",
                    "enhanced_md": enhanced,
                }),
                true,
            )
            .await
            .unwrap();
        assert!(!r.dry_run);
        assert_eq!(r.output["status"], "applied");
        assert_eq!(
            std::fs::read_to_string(analysis_dir.join("personality.md")).unwrap(),
            enhanced,
            "confirm=true must write enhanced_md to disk"
        );

        // A2: world_book/ 前缀拒绝
        let err = apply
            .call(
                serde_json::json!({
                    "character_id": "alice",
                    "filename": "world_book/entry_001.md",
                    "enhanced_md": "evil",
                }),
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AirpError::BadRequest(_)));
    }

    #[test]
    fn register_rejects_duplicate_tool_name() {
        // 同名工具二次注册必须报错，绝不静默覆盖（issue #24）。
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(EchoTool))
            .expect("first echo registers");
        let err = reg
            .register(Box::new(EchoTool))
            .expect_err("duplicate echo must be rejected");
        assert!(matches!(err, AirpError::Config(_)));
        // 首个注册仍在，未被顶掉。
        assert!(reg.get("echo").is_some());
    }

    #[test]
    fn delete_write_lock_excludes_session_writes() {
        // issue #22：delete_character 的角色级写锁必须与 append/rollback 的读锁
        // 互斥。持一把 read guard 时 delete 侧 write() 必须阻塞，直到 read 释放才
        // 推进——证明二者走同一把角色锁，不再各锁各的（旧实现 delete 与命名会话
        // 写属不同 Mutex entry，互不排斥）。用独立 key 避免污染并行测试的角色锁。
        use std::sync::atomic::{AtomicBool, Ordering};
        let key = "issue22-delete-lock-probe";
        let reader = crate::domain::character_lock(key);
        let read_guard = reader.read().unwrap();

        let writer = crate::domain::character_lock(key);
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired2 = acquired.clone();
        let handle = std::thread::spawn(move || {
            let _w = writer.write().unwrap();
            acquired2.store(true, Ordering::SeqCst);
        });

        // read guard 仍持有：write 不可能拿到。
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !acquired.load(Ordering::SeqCst),
            "write lock must not be acquired while a read guard is held"
        );

        // 释放 read → write 应推进。
        drop(read_guard);
        handle.join().unwrap();
        assert!(acquired.load(Ordering::SeqCst));
    }
}
