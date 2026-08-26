//! State & lorebook family built-in Agent tools.
//!
//! 设计纪律（#155 PR 3）：
//! - 6 个 tool struct 保持私有；对 facade 只暴露 [`register`]，
//!   由 `default_registry` 集中调用，不暴露 struct 类型。
//! - Character State 工具显式暴露 revision/expected_revision CAS 合同；
//!   其余工具保持既有 `ToolMeta`、side_effect 与入参/出参形状。
//! - 共享 helper 走 [`super::params`]，不重复实现。
//! - `read_lorebook_or_empty` 是本 family 内部 helper，不外泄。
//!
//! 工具清单：
//! - `get_character_state`：读取角色状态与 revision 元数据（readonly）
//! - `update_character_state`：按 expected revision 校验并替换角色状态（mutate）
//! - `get_lorebook`：读规范化 AIRP v1 lorebook（readonly）
//! - `update_lorebook`：替换 lorebook，支持 canonical / SillyTavern form（destructive）
//! - `apply_lorebook`：返回被文本触发的 enabled 条目（readonly）
//! - `merge_lorebooks`：合并多角色 lorebook，不写盘（readonly）

use super::params::required_character_id;
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{LorebookService, StateService};
use crate::error::AirpError;
use crate::session_coordinator::SessionCommand;
use crate::types::CharacterId;
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

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

/// `get_character_state`：读取角色当前 state 与 revision 元数据。readonly。
struct GetCharacterStateTool {
    effective_root: PathBuf,
}

impl Tool for GetCharacterStateTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_character_state",
            description: "Read a character's current state with revision metadata.",
            side_effect: ToolSideEffect::Readonly,
        }
    }
    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let effective_root = self.effective_root.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let character_label = character.to_string();
            let loaded = tokio::task::spawn_blocking(move || {
                StateService::new(effective_root).read_surface_state_optional(&character)
            })
            .await
            .map_err(|error| AirpError::Internal(format!("state read task failed: {error}")))??;
            let Some((revision, updated_at, state)) = loaded else {
                return Err(AirpError::NotFound(format!(
                    "state for {character_label} not found"
                )));
            };
            Ok(ToolResult {
                output: serde_json::json!({
                    "revision": revision,
                    "updated_at": updated_at,
                    "state": state,
                }),
                dry_run: false,
            })
        })
    }
}

/// `update_character_state`：按 expected revision 校验并替换角色 live state。mutate。
struct UpdateCharacterStateTool {
    state: Arc<DaemonState>,
    effective_root: PathBuf,
}

impl Tool for UpdateCharacterStateTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "update_character_state",
            description: "Replace a character's whole state using character_id, state, and expected_revision from get_character_state; stale revisions conflict.",
            side_effect: ToolSideEffect::Mutate,
        }
    }
    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let daemon = self.state.clone();
        let effective_root = self.effective_root.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let value = params
                .get("state")
                .ok_or_else(|| AirpError::BadRequest("missing state".to_string()))?;
            let expected_revision = params
                .get("expected_revision")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    AirpError::BadRequest(
                        "expected_revision must be a non-negative integer".to_string(),
                    )
                })?;
            let _operation = daemon.session_coordinators.try_submit(
                &effective_root,
                &character,
                None,
                SessionCommand::AgentToolMutation,
            )?;
            let snapshot = StateService::new(&effective_root).replace_if_revision(
                &character,
                expected_revision,
                value,
            )?;
            Ok(ToolResult {
                output: serde_json::to_value(snapshot)?,
                dry_run: false,
            })
        })
    }
}

/// `get_lorebook`：读规范化 AIRP v1 lorebook。readonly。
struct GetLorebookTool {
    effective_root: PathBuf,
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
        let effective_root = self.effective_root.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let lorebook = LorebookService::new(&effective_root).read(&character)?;
            Ok(ToolResult {
                output: serde_json::to_value(lorebook)?,
                dry_run: false,
            })
        })
    }
}

/// `update_lorebook`：替换角色 lorebook。destructive → 默认 dry-run。
/// 支持 AIRP canonical 或 SillyTavern form，通过共享 WorldbookNormalizer 规范化。
struct UpdateLorebookTool {
    effective_root: PathBuf,
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
        let effective_root = self.effective_root.clone();
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
            LorebookService::new(&effective_root).write(&character, &lorebook)?;
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

/// `apply_lorebook`：返回被文本触发的 enabled 条目。readonly。
struct ApplyLorebookTool {
    effective_root: PathBuf,
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
        let effective_root = self.effective_root.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let text = params
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("missing text".to_string()))?;
            let lorebook = read_lorebook_or_empty(&effective_root, &character)?;
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

/// `merge_lorebooks`：合并多角色 lorebook，不写盘。readonly。
/// strategy：union 或 primary_only。
struct MergeLorebooksTool {
    effective_root: PathBuf,
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
        let effective_root = self.effective_root.clone();
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
                vec![read_lorebook_or_empty(&effective_root, &characters[0])?]
            } else {
                characters
                    .iter()
                    .map(|character| read_lorebook_or_empty(&effective_root, character))
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

/// 由 facade `default_registry` 集中调用，注册本 family 全部 6 个工具。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>, effective_root: PathBuf) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(GetCharacterStateTool {
        effective_root: effective_root.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(UpdateCharacterStateTool {
        state: state.clone(),
        effective_root: effective_root.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetLorebookTool {
        effective_root: effective_root.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(UpdateLorebookTool {
        effective_root: effective_root.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ApplyLorebookTool {
        effective_root: effective_root.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(MergeLorebooksTool { effective_root }))
        .expect(COLLISION);
}
