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

/// 构造默认注册表。M_AGENT-1 仅 echo；M_AGENT-2 起注入 built-in 工具。
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
    // Decompose Agent Flow（Task 4）：analysis enhance/apply 工具。
    reg.register(Box::new(EnhanceAnalysisTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ApplyEnhancedAnalysisTool { state }))
        .expect(COLLISION);
    reg
}

const MAX_RECENT_CONTEXT: usize = 200;

// 两级锁保证「破坏性整角色删除」与「单会话写」互斥（issue #22）：
//
// - `CHAT_LOG_LOCKS`（per-session `Mutex`，key = `character` 或 `character/{sid}`）
//   保证同一会话的 append/rollback 串行。
// - `CHARACTER_LOCKS`（per-character `RwLock`，key = `character`）是角色级破坏性锁：
//   append/rollback 持 `read()`（同角色不同会话仍可并发），delete_character 持
//   `write()`（独占，排斥该角色下**所有**会话写）。
//
// 旧实现只有 per-session `Mutex`，delete 用 key=`character`、命名会话写用
// key=`character/{sid}`，属不同 entry 互不排斥 → delete 能与会话写交错，
// 造成半删 / IO race。两级锁获取顺序统一为「先 character 再 session」，无环、无死锁。
type ChatLogLockMap = StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

static CHAT_LOG_LOCKS: OnceLock<ChatLogLockMap> = OnceLock::new();

type CharacterLockMap = StdMutex<HashMap<String, Arc<tokio::sync::RwLock<()>>>>;

static CHARACTER_LOCKS: OnceLock<CharacterLockMap> = OnceLock::new();

/// 取角色级破坏性锁。append/rollback 拿 `read()`，delete_character 拿 `write()`。
fn character_lock(character_id: &str) -> Arc<tokio::sync::RwLock<()>> {
    let mut locks = CHARACTER_LOCKS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .expect("character lock map poisoned");
    locks
        .entry(character_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(())))
        .clone()
}

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
            let char_lock = character_lock(cid.as_str());
            let _char_guard = char_lock.read().await;
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
            let char_lock = character_lock(cid.as_str());
            let _char_guard = char_lock.read().await;
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
            let card_text = data_dir::get_character(&state.data_root, &cid)?;
            let card: Value = serde_json::from_str(&card_text).map_err(|e| {
                AirpError::BadRequest(format!(
                    "character {} card.json is invalid JSON: {}",
                    cid, e
                ))
            })?;
            if !card.is_object() {
                return Err(AirpError::BadRequest(format!(
                    "character {} card.json must be a JSON object",
                    cid
                )));
            }
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
            let char_lock = character_lock(cid.as_str());
            let _char_guard = char_lock.write().await;
            data_dir::delete_character(&state.data_root, &cid)?;
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
// 当前实现：enhance 不调 LLM，返回 original_md 作为 enhanced_md 占位（has_changes=false）。
// 真正的 LLM 调用走 agent loop 自身的 subagent 派生路径（计划书 §4.2 两平面隔离）——
// 协调器派生纯净 subagent 让 LLM 增强 analysis MD，然后调 apply 工具写入。
// 此处的 EnhanceAnalysisTool 供非 loop 路径（如 MCP 工具直调）使用，先返回只读预览。

/// `enhance_analysis`：读 analysis MD，返回 diff 预览（A1：不写盘）。
/// readonly。A2：拒绝 world_book/ 前缀。
/// params: `{ "character_id": string, "filename": string }`
/// → `{ "filename": string, "original_md": string, "enhanced_md": string, "has_changes": bool }`
struct EnhanceAnalysisTool {
    state: Arc<DaemonState>,
}

impl Tool for EnhanceAnalysisTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "enhance_analysis",
            description: "Read a character analysis MD file and return a diff preview (readonly, no write). World book entries are read-only and rejected.",
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
            let path =
                data_dir::char_analysis_file_path(&state.data_root, cid.as_str(), filename)?;
            if !path.exists() {
                return Err(AirpError::NotFound(format!(
                    "analysis file {} not found for character {}",
                    filename, cid
                )));
            }
            let original_md = std::fs::read_to_string(&path)?;
            // 当前占位：不调 LLM，返回原内容。真正的 enhance 走 agent loop subagent 路径。
            let enhanced_md = original_md.clone();
            let has_changes = enhanced_md != original_md;
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
            let path =
                data_dir::char_analysis_file_path(&state.data_root, cid.as_str(), filename)?;
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
            "enhance_analysis",
            "apply_enhanced_analysis",
        ] {
            assert!(reg.get(name).is_some(), "missing tool: {name}");
        }
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
        // A1：enhance 只读返回 diff 预览，不写盘
        // A2：world_book/ 前缀拒绝
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
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
        // 占位实现：enhanced_md == original_md，has_changes=false
        assert_eq!(r.output["enhanced_md"], original);
        assert_eq!(r.output["has_changes"], false);

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

    #[tokio::test]
    async fn delete_write_lock_excludes_session_writes() {
        // issue #22：delete_character 的角色级写锁必须与 append/rollback 的读锁
        // 互斥。持一把 read guard 时 delete 侧 write() 必须阻塞，直到 read 释放才
        // 推进——证明二者走同一把角色锁，不再各锁各的（旧实现 delete 与命名会话
        // 写属不同 Mutex entry，互不排斥）。用独立 key 避免污染并行测试的角色锁。
        use std::sync::atomic::{AtomicBool, Ordering};
        let key = "issue22-delete-lock-probe";
        let reader = character_lock(key);
        let read_guard = reader.read().await;

        let writer = character_lock(key);
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired2 = acquired.clone();
        let handle = tokio::spawn(async move {
            let _w = writer.write().await;
            acquired2.store(true, Ordering::SeqCst);
        });

        // read guard 仍持有：write 不可能拿到。
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !acquired.load(Ordering::SeqCst),
            "write lock must not be acquired while a read guard is held"
        );

        // 释放 read → write 应推进。
        drop(read_guard);
        handle.await.unwrap();
        assert!(acquired.load(Ordering::SeqCst));
    }
}
