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

/// 必填 usize 参数；缺失 / null / 非非负整数 / 超 usize → `BadRequest`。
///
/// #158 CR-01：区分"缺失"与"存在但不是非负整数"，为调用方提供可诊断的精确错误。
/// 注意 `None`（key 不存在）保留 `"missing {key}"` 文本，兼容现有调用方断言；
/// `Some(Null)`（显式 null）单独诊断，因为 required 不允许显式无值。
pub(super) fn required_usize_param(params: &Value, key: &str) -> Result<usize, AirpError> {
    match params.get(key) {
        None => Err(AirpError::BadRequest(format!("missing {}", key))),
        Some(Value::Null) => Err(AirpError::BadRequest(format!(
            "{} must be a non-negative integer, got null",
            key
        ))),
        Some(Value::Number(n)) => {
            // as_u64 对负数（u64 不能表示）与非整数 float 返回 None；分别诊断。
            if let Some(u) = n.as_u64() {
                return usize::try_from(u).map_err(|_| {
                    AirpError::BadRequest(format!("{} {} exceeds platform usize", key, u))
                });
            }
            // as_u64 失败：负数或非整数 float。用 as_f64 判定具体 kind。
            let kind = if n.as_f64().map(|f| f < 0.0).unwrap_or(false) {
                "negative"
            } else {
                "non-integer float"
            };
            Err(AirpError::BadRequest(format!(
                "{} must be a non-negative integer, got {} ({})",
                key, kind, n
            )))
        }
        Some(Value::Bool(_)) => Err(AirpError::BadRequest(format!(
            "{} must be a non-negative integer, got boolean",
            key
        ))),
        Some(Value::String(_)) => Err(AirpError::BadRequest(format!(
            "{} must be a non-negative integer, got string",
            key
        ))),
        Some(Value::Array(_)) => Err(AirpError::BadRequest(format!(
            "{} must be a non-negative integer, got array",
            key
        ))),
        Some(Value::Object(_)) => Err(AirpError::BadRequest(format!(
            "{} must be a non-negative integer, got object",
            key
        ))),
    }
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

// ── 单元测试 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 提取 `AirpError::BadRequest` 内的消息，便于精确断言。
    fn bad_msg(res: Result<usize, AirpError>) -> String {
        match res {
            Err(AirpError::BadRequest(msg)) => msg,
            other => panic!("expected BadRequest, got {:?}", other),
        }
    }

    // ── required_usize_param：成功路径 ──────────────────────────────────

    #[test]
    fn required_usize_param_accepts_zero() {
        let params = serde_json::json!({"n": 0});
        assert_eq!(required_usize_param(&params, "n").unwrap(), 0);
    }

    #[test]
    fn required_usize_param_accepts_positive() {
        let params = serde_json::json!({"n": 42});
        assert_eq!(required_usize_param(&params, "n").unwrap(), 42);
    }

    // ── required_usize_param：缺失 / null（验收要求两个独立分支） ────────

    #[test]
    fn required_usize_param_missing_key_reports_missing() {
        // None（key 不存在）保留 "missing {key}" 文本，兼容现有调用方断言
        // （见 tests/session.rs:302 `message == "missing index"`）。
        let params = serde_json::json!({});
        assert_eq!(
            bad_msg(required_usize_param(&params, "index")),
            "missing index"
        );
    }

    #[test]
    fn required_usize_param_explicit_null_reports_got_null() {
        // Some(Value::Null) 单独诊断：required 不允许显式无值。
        let params = serde_json::json!({"index": null});
        assert_eq!(
            bad_msg(required_usize_param(&params, "index")),
            "index must be a non-negative integer, got null"
        );
    }

    // ── required_usize_param：数值类型错误分支 ──────────────────────────

    #[test]
    fn required_usize_param_negative_reports_negative() {
        let params = serde_json::json!({"n": -5});
        let msg = bad_msg(required_usize_param(&params, "n"));
        assert!(
            msg.contains("negative"),
            "negative branch must mention 'negative', got: {msg}"
        );
        assert!(
            msg.contains("-5"),
            "negative branch must include the raw value, got: {msg}"
        );
    }

    #[test]
    fn required_usize_param_non_integer_float_reports_non_integer_float() {
        // 用 2.5 而非 3.14 避开 clippy::approx_constant（3.14 近似 π）。
        let params = serde_json::json!({"n": 2.5});
        let msg = bad_msg(required_usize_param(&params, "n"));
        assert!(
            msg.contains("non-integer float"),
            "float branch must mention 'non-integer float', got: {msg}"
        );
        assert!(
            msg.contains("2.5"),
            "float branch must include the raw value, got: {msg}"
        );
    }

    #[test]
    fn required_usize_param_integer_value_float_still_rejected() {
        // serde_json::Number::as_u64 对浮点字面量 3.0 返回 None（即使值是整数），
        // 因为 serde_json 内部区分整数与浮点 Number。本函数对 3.0 走
        // "non-integer float" 分支——这是保守行为：JSON spec 不区分 int/float，
        // 但 Rust 端 usize 参数期望整数 JSON token，避免隐式 float→int 截断。
        // 此测试固化该行为，防止未来误把整数 float 归入成功路径。
        let params = serde_json::json!({"n": 3.0});
        let msg = bad_msg(required_usize_param(&params, "n"));
        assert!(
            msg.contains("non-integer float"),
            "integer-value float 3.0 must be rejected as non-integer float, got: {msg}"
        );
    }

    #[test]
    fn required_usize_param_string_reports_string() {
        let params = serde_json::json!({"n": "42"});
        assert_eq!(
            bad_msg(required_usize_param(&params, "n")),
            "n must be a non-negative integer, got string"
        );
    }

    #[test]
    fn required_usize_param_bool_reports_boolean() {
        let params = serde_json::json!({"n": true});
        assert_eq!(
            bad_msg(required_usize_param(&params, "n")),
            "n must be a non-negative integer, got boolean"
        );
    }

    #[test]
    fn required_usize_param_array_reports_array() {
        let params = serde_json::json!({"n": [1, 2]});
        assert_eq!(
            bad_msg(required_usize_param(&params, "n")),
            "n must be a non-negative integer, got array"
        );
    }

    #[test]
    fn required_usize_param_object_reports_object() {
        let params = serde_json::json!({"n": {"v": 1}});
        assert_eq!(
            bad_msg(required_usize_param(&params, "n")),
            "n must be a non-negative integer, got object"
        );
    }

    // ── required_usize_param：u64/usize overflow 分支 ─────────────────
    //
    // 注意：在 64-bit 平台 `usize::try_from(u64)` 永不失败（usize >= u64），
    // 故 overflow 分支只在 32-bit 平台可达。此测试用 `#[cfg]` 门控，确保在
    // 32-bit 平台运行时验证分支正确性；64-bit 平台该分支不可达，无需测试。
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn required_usize_param_u64_overflow_reports_exceeds_platform_usize() {
        // 用一个明确超过 32-bit usize 上限（2^32-1）的 u64 值。
        let overflow_val: u64 = u32::MAX as u64 + 1;
        let params = serde_json::json!({"n": overflow_val});
        let msg = bad_msg(required_usize_param(&params, "n"));
        assert!(
            msg.contains("exceeds platform usize"),
            "overflow branch must mention 'exceeds platform usize', got: {msg}"
        );
        assert!(
            msg.contains(&overflow_val.to_string()),
            "overflow branch must include the raw value, got: {msg}"
        );
    }

    /// 64-bit 平台的 overflow 分支不可达性契约：u64::MAX 必须被成功接受为 usize，
    /// 因为 64-bit 上 usize::MAX == u64::MAX。此测试固化该平台行为，防止未来
    /// 误把 u64::MAX 当作错误。32-bit 平台由上面的 cfg 测试覆盖。
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn required_usize_param_u64_max_accepted_on_64bit_platform() {
        let params = serde_json::json!({"n": u64::MAX});
        assert_eq!(
            required_usize_param(&params, "n").unwrap(),
            u64::MAX as usize
        );
    }

    // ── optional_usize_param：null → default 行为不变 ──────────────────

    #[test]
    fn optional_usize_param_null_returns_default() {
        // null 必须返回 default，而不是走 required 的 "got null" 错误。
        // 这是 optional 的既有合同，CR-01 不得破坏。
        let params = serde_json::json!({"n": null});
        assert_eq!(optional_usize_param(&params, "n", 20).unwrap(), 20);
    }

    #[test]
    fn optional_usize_param_missing_returns_default() {
        let params = serde_json::json!({});
        assert_eq!(optional_usize_param(&params, "n", 20).unwrap(), 20);
    }

    #[test]
    fn optional_usize_param_present_uses_required_path() {
        // present 且非 null → 走 required_usize_param 的精确诊断。
        let params = serde_json::json!({"n": "bad"});
        assert_eq!(
            bad_msg(optional_usize_param(&params, "n", 20)),
            "n must be a non-negative integer, got string"
        );
    }

    #[test]
    fn optional_usize_param_present_valid_returns_value() {
        let params = serde_json::json!({"n": 7});
        assert_eq!(optional_usize_param(&params, "n", 20).unwrap(), 7);
    }
}
