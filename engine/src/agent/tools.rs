//! M_AGENT-1: Tool execution surface — minimal skeleton.
//!
//! 协调器（`AgentLoop`）在每一步选择"派生纯净 subagent / 调一个工具 / 收敛结束"。
//! 本模块定义工具的抽象（[`Tool`] trait）、注册表（[`ToolRegistry`]）以及一个
//! 用于 M_AGENT-1 验收的 mock 工具 `echo`。
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
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use crate::adapter::{ChatMessage, MessageRole};
use crate::chat_store::ChatLog;
use crate::daemon::DaemonState;
use crate::data_dir;
use crate::types::{CharacterId, SessionId};

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

/// 工具注册表。M_AGENT-1 仅 mock；M_AGENT-2 注入 built-in；M_AGENT-3 合并 MCP upstream。
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<&'static str, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.meta().name;
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<ToolMeta> {
        self.tools.values().map(|t| t.meta()).collect()
    }

    /// 是否授权调用该工具（M_AGENT-5 补 allowlist；M_AGENT-1 全放行）。
    pub fn allowed(&self, _name: &str) -> bool {
        true
    }
}

// ── mock 工具：echo（M_AGENT-1 验收用）─────────────────────────────────────

/// Echo：原样回传入参，标注 dry_run=false。用于验证 loop → 工具 → 回灌闭环。
pub struct EchoTool;

impl Tool for EchoTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "echo",
            description: "M_AGENT-1 mock: returns its input verbatim. Verifies loop→tool→subagent wiring.",
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

/// 构造默认注册表。M_AGENT-1 仅 echo；M_AGENT-2 起注入 built-in 工具。
///
/// `state` 让 built-in 工具访问数据层（`data_root`）。echo 等无状态工具
/// 忽略它。调用方（`AgentLoop::new`）已有 `Arc<DaemonState>`，传入即可。
pub fn default_registry(state: Arc<DaemonState>) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(EchoTool));
    // M_AGENT-2 第一批：会话类 5 工具。
    reg.register(Box::new(ListSessionsTool { state: state.clone() }));
    reg.register(Box::new(StartSessionTool { state: state.clone() }));
    reg.register(Box::new(AppendMessageTool { state: state.clone() }));
    reg.register(Box::new(GetRecentContextTool { state: state.clone() }));
    reg.register(Box::new(RollbackMessagesTool { state: state.clone() }));
    // M_AGENT-2 第二批：角色类 3 工具（list/get/delete）。
    reg.register(Box::new(ListCharactersTool { state: state.clone() }));
    reg.register(Box::new(GetCharacterTool { state: state.clone() }));
    reg.register(Box::new(DeleteCharacterTool { state }));
    reg
}

const MAX_RECENT_CONTEXT: usize = 200;

type ChatLogLockMap = StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

static CHAT_LOG_LOCKS: OnceLock<ChatLogLockMap> = OnceLock::new();

fn chat_log_lock(
    character_id: &str,
    session_id: Option<&SessionId>,
) -> Arc<tokio::sync::Mutex<()>> {
    let key = match session_id {
        Some(session_id) => format!("{}/{}", character_id, session_id),
        None => character_id.to_string(),
    };
    let mut locks = CHAT_LOG_LOCKS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .expect("chat log lock map poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
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
            let sessions = data_dir::list_sessions(&state.data_root, cid.as_str())?;
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
            description: "Create a new named session for a character. session_id is auto-generated (UUID).",
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
            let sid = data_dir::create_session(&state.data_root, cid.as_str())?;
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
            let lock = chat_log_lock(cid.as_str(), session_id.as_ref());
            let _guard = lock.lock().await;
            let mut log = ChatLog::load_or_create_for_session(
                &state.data_root,
                cid.as_str(),
                session_id.as_ref(),
            )?;
            let total_before = log.messages.len();
            if role == MessageRole::System {
                tracing::info!(
                    character_id = %cid,
                    session_id = session_id.map(|sid| sid.to_string()).as_deref().unwrap_or("default"),
                    "append_message writes a system message"
                );
            }
            log.append(
                &state.data_root,
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
            let log = ChatLog::load_or_create_for_session(
                &state.data_root,
                cid.as_str(),
                session_id.as_ref(),
            )?;
            let recent = log.recent(n);
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
            let lock = chat_log_lock(cid.as_str(), session_id.as_ref());
            let _guard = lock.lock().await;
            let mut log = ChatLog::load_or_create_for_session(
                &state.data_root,
                cid.as_str(),
                session_id.as_ref(),
            )?;
            let total = log.messages.len();
            if index >= total {
                return Err(AirpError::BadRequest(format!(
                    "index {} out of range (total {})",
                    index, total
                )));
            }
            let dropped = total - index - 1;
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
            log.rollback_to(&state.data_root, index)?;
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
            description: "Read a character's card.json (parsed object). Returns card fields like name/description/personality/first_mes/etc.",
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
            let card_text = data_dir::get_character(&state.data_root, cid.as_str())?;
            let card: Value = serde_json::from_str(&card_text).unwrap_or(Value::String(card_text));
            Ok(ToolResult {
                output: serde_json::json!({ "card": card }),
                dry_run: false,
            })
        })
    }
}

/// `delete_character`：删整个角色目录（card + state + memory + sessions）。
/// **destructive** → 默认 dry-run，未 confirm 只回"将删除角色 X 的全部数据"。
/// params: `{ "character_id": string }` → `{ "deleted": string }` 或 dry-run preview
struct DeleteCharacterTool {
    state: Arc<DaemonState>,
}

impl Tool for DeleteCharacterTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "delete_character",
            description: "Delete a character's entire directory (card, state, memory, sessions). Destructive — dry-run unless confirm=true.",
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
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "character_id": cid.to_string(),
                        "preview": "将删除该角色的全部数据（card + state + memory + sessions）。传 confirm=true 执行。",
                    }),
                    dry_run: true,
                });
            }
            data_dir::delete_character(&state.data_root, cid.as_str())?;
            Ok(ToolResult {
                output: serde_json::json!({ "deleted": cid.to_string() }),
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
    async fn append_reports_persisted_index_after_fifo_truncation() {
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

        assert_eq!(r.output["index"], MAX_MESSAGES - 1);
        assert_eq!(r.output["total"], MAX_MESSAGES);
        assert_eq!(r.output["truncated"], true);
        assert_eq!(r.output["truncated_count"], 1);
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

    #[tokio::test]
    async fn character_tools_list_get_delete() {
        // 端到端：list(空) → 写 fixture card → list(1) → get → delete(dry-run) → delete(真) → list(空)
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
        let reg = default_registry(state.clone());

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
        ).unwrap();

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
        assert!(r.output["preview"].is_string());

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
}
