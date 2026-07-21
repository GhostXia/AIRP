//! Public request/response types for the daemon HTTP API.
use crate::adapter::{ChatMessage, Provider};
use crate::types::{CharacterId, PersonaId, PresetId, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// `POST /v1/chat/completions` 请求体。除 `message` 与 `user_profile` 必填外，
/// 其余字段均可省略；缺失字段由 daemon 三层合并配置补齐（见 [`crate::config::AppConfig`]）。
#[derive(Debug, Deserialize, Clone)]
pub struct ChatCompletionRequest {
    /// 角色目录名（data/characters/{id}/）。M5.0a：用 `CharacterId` 类型
    /// 替代裸 String，反序列化时自动调用 `validate_id_segment`。
    pub character_id: Option<CharacterId>,
    /// 直接指定角色卡 PNG 路径或内联 JSON 字符串（legacy 兼容）。
    pub character_card_id: Option<String>,
    /// 世界书路径或内联 JSON 字符串。
    pub lorebook_path: Option<String>,
    /// 终端用户画像（姓名 + 变量表）。
    pub user_profile: UserProfile,
    /// 本轮用户消息文本。
    pub message: String,
    /// 历史消息列表；为 `None` 时 R-04 自动从 ChatLog 加载最近 50 条。
    pub messages_history: Option<Vec<ChatMessage>>,
    /// 客户端自定义流过滤正则集合（叠加在 daemon 默认 `<卷评估/>` 之上）。
    pub regex_filters: Option<Vec<String>>,
    /// 预设文件名（不含 .json 后缀）。M5.0a：`PresetId` 类型校验。
    pub preset_id: Option<PresetId>,
    /// 预设内启用的 prompt identifier 子集；为 `None` 时启用全部。
    pub enabled_presets: Option<Vec<String>>,
    /// M5.1：可选 session ID。若指定则卷系统路径走
    /// `data/characters/{id}/sessions/{session_id}/`；省略则回退到 legacy
    /// `session/` 单 session 路径，确保旧客户端零迁移。
    pub session_id: Option<SessionId>,

    /// 请求级 provider 覆盖；缺失时取 daemon `MutableConfig.provider`。
    pub provider: Option<Provider>,
    /// 请求级端点覆盖。
    pub endpoint: Option<String>,
    /// 请求级 API key 覆盖。
    pub api_key: Option<String>,
    /// 请求级模型覆盖。
    pub model: Option<String>,
    /// 请求级温度覆盖。
    pub temperature: Option<f32>,
    /// 请求级 max_tokens 覆盖。
    pub max_tokens: Option<u32>,
    /// MS-6：场景 ID；设置后忽略 `character_id`，走多角色场景分支。
    pub scene_id: Option<String>,
    /// DX-1：可选用户 ID；设置后数据根切换为 `data/users/{user_id}/`，实现多租户隔离。
    /// 为 None 时使用全局 `data/`（单用户向后兼容模式）。
    pub user_id: Option<String>,
    /// A1b：显式指定本次对话激活的 Persona id。
    ///
    /// 仅在请求带 `user_id` 时生效；`user_id` 缺失时此字段被忽略（保持单用户
    /// 向后兼容）。`default` 大小写不敏感；其他 id 不存在时返 `404`，与 plural
    /// GET 契约一致。缺省时由 `chat_pipeline` 按 precedence contract 自动解析
    /// （`find_for_character` 绑定 → `default` persona），详见
    /// [docs/PERSONA-HTTP-API-PLAN.md](../../../docs/PERSONA-HTTP-API-PLAN.md)。
    ///
    /// #153 E1：用 `PersonaId` newtype 替代裸 `String`，反序列化时自动调
    /// `validate_id_segment`，与 `character_id: Option<CharacterId>` /
    /// `preset_id: Option<PresetId>` 一致。原 service 层兜底校验
    /// （`PersonaService::get` 内调 `validate_persona_id`）保留作为
    /// defense-in-depth。
    pub persona_id: Option<PersonaId>,
    /// #249 Swipe：regen 时捕获的旧候选列表。内部使用，HTTP 请求不提供此字段
    ///（serde(default) 给空 Vec）。非空时 finalizer 会将新生成文本追加为最后一个候选。
    #[serde(default)]
    pub swipe_candidates: Vec<String>,
    /// 分支对话树：从指定消息 durable ID 分叉。
    ///
    /// - `None`（默认）→ 线性追加（parent = 当前 active_leaf）。
    /// - `Some(id)` → 新消息的 parent = 该 ID（从任意消息分叉）。
    ///   ID 必须存在于当前 session 的 message_ids 中，否则 BadRequest。
    #[serde(default)]
    pub branch_from: Option<String>,
}

/// 用户画像：用于 `{{user}}` 等变量替换。
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct UserProfile {
    /// 用户显示名。
    pub name: String,
    /// 自定义变量表，键名对应 prompt 中 `{{key}}` 占位符。
    pub variables: HashMap<String, String>,
}

/// SSE 事件的 JSON payload。
#[derive(Debug, Serialize)]
pub struct ChatResponseChunk {
    /// 本帧已清洗的文本片段。
    pub text: String,
}

/// `POST /v1/chat/rollback` 请求体。
///
/// #37 durable message-id contract：
/// - `message_id`（新）→ 走 ID 寻址 rollback（推荐）；ID 不存在 → `BadRequest`。
/// - `message_index`（legacy）→ 走数组下标 rollback（向后兼容保留）。
/// - 同时传两个 → `BadRequest`（显式二义）。
/// - 都不传 → `BadRequest`（必填其一）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 回滚到的消息索引（保留 [0, message_index) 区间，丢弃其后所有消息）。
    /// legacy 路径，向后兼容保留；与 `message_id` 二选一。
    pub message_index: Option<usize>,
    /// #37：回滚到的消息 durable ID（保留该 ID 及其之前所有消息，丢弃其后）。
    /// 推荐路径；与 `message_index` 二选一。
    pub message_id: Option<String>,
    /// A6：可选 session ID。指定则操作 `characters/{id}/sessions/{session_id}/history/`；
    /// 省略则回退 legacy per-character `characters/{id}/history/`。
    pub session_id: Option<SessionId>,
}

impl RollbackRequest {
    /// 校验 `message_index` / `message_id` 二选一规则。返回 `Ok(())` 或 `BadRequest`。
    pub fn validate_rollback_target(&self) -> Result<(), String> {
        match (self.message_index, self.message_id.as_deref()) {
            (Some(_), Some(_)) => Err(
                "rollback target is ambiguous: pass exactly one of message_index or message_id"
                    .to_string(),
            ),
            (None, None) => Err(
                "rollback target is required: pass message_id (preferred) or message_index"
                    .to_string(),
            ),
            _ => Ok(()),
        }
    }
}

/// `POST /v1/chat/regen` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenRequest {
    /// 目标角色 ID；删除该角色 ChatLog 的最后一条消息。
    pub character_id: CharacterId,
    /// A6：可选 session ID（语义同 `RollbackRequest.session_id`）。
    pub session_id: Option<SessionId>,
    /// DX-1：可选用户 ID（per-user 数据隔离）。
    pub user_id: Option<String>,
}

/// `POST /v1/chat/continue` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinueRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 可选 session ID。
    pub session_id: Option<SessionId>,
    /// DX-1：可选用户 ID。
    pub user_id: Option<String>,
}

/// `POST /v1/chat/delete` 请求体。删除单条消息。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteMessageRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 可选 session ID。
    pub session_id: Option<SessionId>,
    /// 要删除的消息 durable ID。
    pub message_id: String,
    /// DX-1：可选用户 ID（per-user 数据隔离）。
    pub user_id: Option<String>,
}

/// `POST /v1/chat/swipe` 请求体。#249 Swipe：切换指定消息的激活候选。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwipeRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 可选 session ID。
    pub session_id: Option<SessionId>,
    /// 要切换的消息 durable ID。
    pub message_id: String,
    /// 新激活候选的下标（0-based）。
    pub index: usize,
    /// DX-1：可选用户 ID（per-user 数据隔离）。
    pub user_id: Option<String>,
}

/// `PUT /v1/chat/message` 请求体。编辑指定 durable ID 消息的 content。
///
/// 约束：只允许编辑 `role=user` 消息（assistant 编辑 = regen/swipe 语义）。
/// ID/timestamp/role 不变，仅替换 content。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditMessageRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 可选 session ID。
    pub session_id: Option<SessionId>,
    /// 要编辑的消息 durable ID。
    pub message_id: String,
    /// 新的消息内容。
    pub content: String,
    /// DX-1：可选用户 ID（per-user 数据隔离）。
    pub user_id: Option<String>,
}

/// `POST /v1/chat/branch/switch` 请求体。切换激活分支。
///
/// 将 `active_leaf` 设为指定的叶节点 durable ID，不删除其他分支数据。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchBranchRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 可选 session ID。
    pub session_id: Option<SessionId>,
    /// 目标叶节点的 durable ID（切换后成为 active_leaf）。
    pub target_leaf_id: String,
    /// DX-1：可选用户 ID（per-user 数据隔离）。
    pub user_id: Option<String>,
}

/// `POST /v1/chat/history` 请求体。
///
/// #37 durable message-id contract：
/// - 不传 `limit` / `before` → 全量返回（向后兼容旧客户端）。
/// - 传 `limit` → 最多返回 `limit` 条（clamp [1, 200]，默认 50）。
/// - 传 `before` → cursor 分页：返回该 durable ID 严格之前（更早）的消息。
///   `before` 必须命中当前 session 的某条 durable ID，否则 `BadRequest`
///   （cursor 不能跨 character/session）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    /// 待查询角色 ID。
    pub character_id: CharacterId,
    /// A6：可选 session ID（语义同 `RollbackRequest.session_id`）。
    pub session_id: Option<SessionId>,
    /// #37：本次最多返回多少条（clamp [1, 200]，默认 50）。不传 → 全量返回（兼容）。
    pub limit: Option<usize>,
    /// #37：cursor；某条消息的 durable ID，返回该 ID 严格之前（更早）的消息。
    pub before: Option<String>,
}
