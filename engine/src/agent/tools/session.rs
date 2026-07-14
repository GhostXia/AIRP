//! Session-family built-in Agent tools.
//!
//! 设计纪律（#155 PR 2）：
//! - 5 个 tool struct 保持私有；对 facade 只暴露 [`register`]，
//!   由 `default_registry` 集中调用，不暴露 struct 类型。
//! - 不改任何 `ToolMeta` 文案、side_effect 或入参/出参形状。
//! - 共享 helper 走 [`super::params`]，不重复实现。
//!
//! 工具清单：
//! - `list_sessions`：列某角色的所有命名会话（readonly）
//! - `start_session`：创建新命名会话，session_id 自动生成（mutate）
//! - `append_message`：向会话追加消息（append，JSONL O(1) 写）
//! - `get_recent_context`：取最近 N 条消息（readonly，N 上限 200）
//! - `rollback_messages`：回滚到指定索引（destructive，默认 dry-run）

use super::params::{
    optional_session_id, optional_usize_param, required_character_id, required_usize_param,
};
use super::*;
use crate::adapter::{ChatMessage, MessageRole};
use crate::daemon::DaemonState;
use crate::domain::ChatService;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// `get_recent_context` 的 N 上限。超过即 `BadRequest`，避免 agent 拉爆上下文。
///
/// `pub(super)` 让 `tools::tests::session` 能引用此上限做边界断言
///（`recent_context_rejects_over_cap` 用 `MAX_RECENT_CONTEXT + 1` 验证拒绝）。
/// 不外泄到 crate / public 表面积。
pub(super) const MAX_RECENT_CONTEXT: usize = 200;

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
            let cid = required_character_id(&params)?;
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
            let cid = required_character_id(&params)?;
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

/// 由 facade `default_registry` 集中调用，注册本 family 全部 5 个工具。
///
/// family 内 tool struct 保持私有；调用方只能通过 [`ToolRegistry::get`]
/// 按名字拿到 `&dyn Tool`，拿不到具体类型，避免外部代码绑定实现细节。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
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
    reg.register(Box::new(RollbackMessagesTool { state }))
        .expect(COLLISION);
}
