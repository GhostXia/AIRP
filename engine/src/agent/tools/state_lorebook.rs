//! State & lorebook family built-in Agent tools.
//!
//! 设计纪律（#155 PR 3）：
//! - 6 个 tool struct 保持私有；对 facade 只暴露 [`register`]，
//!   由 `default_registry` 集中调用，不暴露 struct 类型。
//! - 不改任何 `ToolMeta` 文案、side_effect 或入参/出参形状。
//! - 共享 helper 走 [`super::params`]，不重复实现。
//! - `read_lorebook_or_empty` 是本 family 内部 helper，不外泄。
//!
//! 工具清单：
//! - `get_character_state`：读角色 live.json（readonly）
//! - `update_character_state`：校验并替换角色状态，生成 revision 快照（mutate）
//! - `get_lorebook`：读规范化 AIRP v1 lorebook（readonly）
//! - `update_lorebook`：替换 lorebook，支持 canonical / SillyTavern form（destructive）
//! - `apply_lorebook`：返回被文本触发的 enabled 条目（readonly）
//! - `merge_lorebooks`：合并多角色 lorebook，不写盘（readonly）

use super::params::required_character_id;
use super::*;
use crate::daemon::DaemonState;
use crate::data_dir;
use crate::domain::{LorebookService, StateService};
use crate::error::AirpError;
use crate::types::CharacterId;
use serde_json::Value;
use std::future::Future;
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

/// `get_character_state`：读角色当前 state/live.json。readonly。
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

/// `update_character_state`：校验并替换角色 live state，生成 revisioned 快照。mutate。
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

/// `get_lorebook`：读规范化 AIRP v1 lorebook。readonly。
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

/// `update_lorebook`：替换角色 lorebook。destructive → 默认 dry-run。
/// 支持 AIRP canonical 或 SillyTavern form，通过共享 WorldbookNormalizer 规范化。
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

/// `apply_lorebook`：返回被文本触发的 enabled 条目。readonly。
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

/// `merge_lorebooks`：合并多角色 lorebook，不写盘。readonly。
/// strategy：union 或 primary_only。
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

/// 由 facade `default_registry` 集中调用，注册本 family 全部 6 个工具。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
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
    reg.register(Box::new(MergeLorebooksTool { state }))
        .expect(COLLISION);
}
