//! Cross-family parameter parsing helpers shared by built-in Agent tools.
//!
//! 设计纪律（#155 PR 2）：本模块只放真正跨 family 的小 helper；
//! family 内部专用解析（如 `append_message` 的 role enum 转换）留在
//! 各 family 自己的文件里。所有 helper 都是 `pub(super)`：对 `tools`
//! 父模块的兄弟子模块（`session` / `character` / 未来的 `state_lorebook`
//! 等）可见，绝不外泄到 crate / public 表面积。

use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};
use serde_json::Value;

/// 从 `params.character_id`（字符串）构造 `CharacterId`。
/// 缺失或非字符串 → `BadRequest`；非法字符 → 透传 `CharacterId::new` 的错误。
pub(super) fn required_character_id(params: &Value) -> Result<CharacterId, AirpError> {
    let value = params
        .get("character_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AirpError::BadRequest("missing character_id".to_string()))?;
    CharacterId::new(value)
}

/// 从 `params.session_id` 构造可选 `SessionId`。
/// 缺失 / 显式 null → `None`；非字符串 → `BadRequest`；非法 → 透传。
pub(super) fn optional_session_id(params: &Value) -> Result<Option<SessionId>, AirpError> {
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

/// 必填 usize 参数；缺失 / 非 u64 / 超 usize → `BadRequest`。
pub(super) fn required_usize_param(params: &Value, key: &str) -> Result<usize, AirpError> {
    let raw = params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| AirpError::BadRequest(format!("missing {}", key)))?;
    usize::try_from(raw)
        .map_err(|_| AirpError::BadRequest(format!("{} {} exceeds platform usize", key, raw)))
}

/// 可选 usize 参数；缺失 / null → `default`，否则走 `required_usize_param`。
pub(super) fn optional_usize_param(
    params: &Value,
    key: &str,
    default: usize,
) -> Result<usize, AirpError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => required_usize_param(params, key),
    }
}
