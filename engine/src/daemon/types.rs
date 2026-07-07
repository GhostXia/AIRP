//! Public request/response types for the daemon HTTP API.
use crate::adapter::{ChatMessage, Provider};
use crate::types::{CharacterId, PresetId, SessionId};
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackRequest {
    /// 目标角色 ID。
    pub character_id: CharacterId,
    /// 回滚到的消息索引（保留 [0, message_index) 区间，丢弃其后所有消息）。
    pub message_index: usize,
    /// A6：可选 session ID。指定则操作 `characters/{id}/sessions/{session_id}/history/`；
    /// 省略则回退 legacy per-character `characters/{id}/history/`。
    pub session_id: Option<SessionId>,
}

/// `POST /v1/chat/regen` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenRequest {
    /// 目标角色 ID；删除该角色 ChatLog 的最后一条消息。
    pub character_id: CharacterId,
    /// A6：可选 session ID（语义同 `RollbackRequest.session_id`）。
    pub session_id: Option<SessionId>,
}

/// `POST /v1/chat/history` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    /// 待查询角色 ID。
    pub character_id: CharacterId,
    /// A6：可选 session ID（语义同 `RollbackRequest.session_id`）。
    pub session_id: Option<SessionId>,
}
