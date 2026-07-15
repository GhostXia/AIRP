//! Preset family built-in Agent tools（#115 P1 第二阶段）。
//!
//! 与 `state_lorebook.rs` 对齐的设计纪律：
//! - 2 个 tool struct 保持私有；对 facade 只暴露 [`register`]。
//! - `update_preset` 是 destructive → 默认 dry-run，需 `confirm=true` 才写盘。
//! - 共享 helper 走 [`super::params`]，preset_id 解析是本 family 内部 helper。
//! - 写盘逻辑走 `PresetService`（`orchestrator::preset`），与 `LorebookService` 对齐。
//!
//! 工具清单：
//! - `get_preset`：读 canonical preset 的 prompts 数组（readonly）
//! - `update_preset`：替换 preset，走 normalize_preset + raw sidecar（destructive）

use super::*;
use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::orchestrator::preset::PresetService;
use crate::types::PresetId;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// 从 `params.preset_id`（字符串）构造 `PresetId`。
/// 缺失或非字符串 → `BadRequest`；非法字符 → 透传 `PresetId::new` 的错误。
fn required_preset_id(params: &Value) -> Result<PresetId, AirpError> {
    let value = params
        .get("preset_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AirpError::BadRequest("missing preset_id".to_string()))?;
    PresetId::new(value)
}

/// `get_preset`：读 canonical preset 的 prompts 数组。readonly。
struct GetPresetTool {
    state: Arc<DaemonState>,
}

impl Tool for GetPresetTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_preset",
            description: "Read a preset's canonical prompts array by preset_id. Returns AIRP v1 normalized prompts.",
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
            let id = required_preset_id(&params)?;
            let preset = PresetService::new(&state.data_root).read(&id)?;
            let prompts = preset.prompts.unwrap_or_default();
            Ok(ToolResult {
                output: serde_json::json!({
                    "preset_id": id.as_str(),
                    "prompts_count": prompts.len(),
                    "prompts": prompts,
                }),
                dry_run: false,
            })
        })
    }
}

/// `update_preset`：替换 preset。destructive → 默认 dry-run。
/// 接受 SillyTavern 或 AIRP canonical form，通过共享 `normalize_preset` 规范化。
struct UpdatePresetTool {
    state: Arc<DaemonState>,
}

impl Tool for UpdatePresetTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "update_preset",
            description: "Replace a preset. Accepts SillyTavern or AIRP canonical form; normalizes via shared normalizer. Destructive: requires confirm=true.",
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
            let id = required_preset_id(&params)?;
            let source_json = params
                .get("preset_json")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing preset_json".to_string()))?;

            if !confirm {
                // dry-run：只做归一化 + 诊断，不写盘。对齐 update_lorebook 的 dry-run 语义。
                let cleaned = crate::data_dir::strip_utf8_bom(source_json).to_owned();
                let source: Value = serde_json::from_str(&cleaned)
                    .map_err(|e| AirpError::BadRequest(format!("preset JSON 无效: {}", e)))?;
                let (_, report) = crate::orchestrator::preset::normalize_preset(&source);
                if let Some(reason) = report.replacement_error() {
                    return Err(AirpError::BadRequest(format!("preset 无法导入: {reason}")));
                }
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "preset_id": id.as_str(),
                        "action": "update_preset",
                        "converted": report.converted,
                        "import_report": report,
                        "requires": "confirm=true"
                    }),
                    dry_run: true,
                });
            }

            let (_, report) = PresetService::new(&state.data_root).write(&id, source_json)?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "updated": id.as_str(),
                    "import_report": report
                }),
                dry_run: false,
            })
        })
    }
}

/// 由 facade `default_registry` 集中调用，注册本 family 全部 2 个工具。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(GetPresetTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(UpdatePresetTool { state }))
        .expect(COLLISION);
}
