//! Character-family built-in Agent tools.
//!
//! 设计纪律（#155 PR 2）：
//! - 3 个 tool struct 保持私有；对 facade 只暴露 [`register`]。
//! - 不改任何 `ToolMeta` 文案、side_effect 或入参/出参形状。
//! - `get/delete` 通过 [`super::params::required_character_id`] 复用跨 family
//!   的 character id 合同，不在 family 内复制解析规则。
//!
//! 工具清单：
//! - `list_characters`：列所有角色 id（readonly）
//! - `get_character`：读角色 card.json 为 parsed JSON object（readonly）
//! - `delete_character`：删整个角色目录子树（destructive，默认 dry-run）

use super::params::required_character_id;
use super::*;
use crate::daemon::DaemonState;
use crate::data_dir;
use crate::domain::ChatService;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
            let cid = required_character_id(&params)?;
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
            let cid = required_character_id(&params)?;
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

/// 由 facade `default_registry` 集中调用，注册本 family 全部 3 个工具。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(ListCharactersTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetCharacterTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(DeleteCharacterTool { state }))
        .expect(COLLISION);
}
