//! 集中错误类型。M2.1 引入。
//!
//! 各模块逐步从 `Result<T, String>` 迁移到 `Result<T, AirpError>`（M2.2）。
//! HTTP 层通过 `IntoResponse` 实现统一映射（M2.3）。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::path::PathBuf;
use thiserror::Error;

/// 项目统一错误类型。所有公开 API 在 M2 收敛后均返回 `Result<T, AirpError>`。
///
/// 每个变体对应一个语义类别，HTTP 映射规则由 [`AirpError::status`] 决定，
/// `Display` 实现由 `thiserror` 自动生成的中文模板提供给客户端 / 日志。
#[derive(Error, Debug)]
pub enum AirpError {
    /// 文件 I/O 失败（读 / 写 / 创建目录）。从 `std::io::Error` 自动转换。
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    /// JSON 解析或序列化失败。
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),

    /// 上游 HTTP 调用本身失败（连接 / DNS / 超时）。区别于 [`Upstream`]：
    /// 后者是上游返回了非 2xx 状态码。
    ///
    /// [`Upstream`]: AirpError::Upstream
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),

    /// 正则编译失败（用户输入过滤规则非法时）。
    #[error("正则编译错误: {0}")]
    Regex(#[from] regex::Error),

    /// 客户端请求形式不合法（缺字段 / ID 非法 / payload 错型）。映射到 HTTP 400。
    #[error("非法请求: {0}")]
    BadRequest(String),

    /// 客户端请求的资源（角色 / 预设 / 卷 / session）不存在。映射到 HTTP 404。
    #[error("资源不存在: {0}")]
    NotFound(String),

    /// 路径遍历攻击保护：用户提供的路径 canonicalize 后越出 `data_root` 子树。
    /// 映射到 HTTP 400。
    #[error("路径越出 data_root: {0:?}")]
    PathEscape(PathBuf),

    /// 上游 LLM API 返回非 2xx 状态码。包含原始状态码 + 响应 body 便于排错。
    /// 映射到 HTTP 502 Bad Gateway。
    #[error("上游 API 返回 {status}: {body}")]
    Upstream {
        /// 上游返回的 HTTP 状态码。
        status: u16,
        /// 上游响应 body（用于诊断；500 路径不向客户端透出）。
        body: String,
    },

    /// 启动配置或运行时配置违反不变量（如 `soft >= hard`、非法 endpoint）。
    #[error("配置错误: {0}")]
    Config(String),

    /// 编排器（system prompt 组装、card / lorebook / preset 处理）失败。
    #[error("编排器错误: {0}")]
    Orchestrator(String),

    /// 卷系统（封卷流程、index 维护、current.md I/O）失败。
    #[error("卷系统错误: {0}")]
    Volume(String),

    /// 流式 FSM 过滤器内部错误（罕见，通常表示状态机违例）。
    #[error("FSM 错误: {0}")]
    Fsm(String),

    /// 其他内部不变量违反。映射到 HTTP 500，错误细节仅入 tracing，不返客户端。
    #[error("内部错误: {0}")]
    Internal(String),

    /// DX-3：用户每日配额已达上限。映射到 HTTP 429 Too Many Requests。
    #[error("配额超限: {0}")]
    QuotaExceeded(String),
}

/// 项目内约定的 Result 别名。
pub type AirpResult<T> = Result<T, AirpError>;

impl AirpError {
    /// M2.3：错误到 HTTP 状态码的映射。
    pub fn status(&self) -> StatusCode {
        match self {
            AirpError::BadRequest(_) | AirpError::PathEscape(_) => StatusCode::BAD_REQUEST,
            AirpError::NotFound(_) => StatusCode::NOT_FOUND,
            AirpError::Upstream { .. } => StatusCode::BAD_GATEWAY,
            AirpError::QuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
            AirpError::Io(_)
            | AirpError::Json(_)
            | AirpError::Http(_)
            | AirpError::Regex(_)
            | AirpError::Config(_)
            | AirpError::Orchestrator(_)
            | AirpError::Volume(_)
            | AirpError::Fsm(_)
            | AirpError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// #67 #9 / PR #74 方案 A：错误 code 字符串，用于 JSON envelope 的 `code` 字段。
    ///
    /// 与 `models_proxy_error` 的 `code: &'static str` 风格对齐，便于 webui
    /// `formatError` 白名单统一展开。snake_case，稳定不变。
    pub fn code_str(&self) -> &'static str {
        match self {
            AirpError::BadRequest(_) => "bad_request",
            AirpError::PathEscape(_) => "path_escape",
            AirpError::NotFound(_) => "not_found",
            AirpError::Upstream { .. } => "upstream",
            AirpError::QuotaExceeded(_) => "quota_exceeded",
            AirpError::Io(_) => "io_error",
            AirpError::Json(_) => "json_error",
            AirpError::Http(_) => "http_error",
            AirpError::Regex(_) => "regex_error",
            AirpError::Config(_) => "config_error",
            AirpError::Orchestrator(_) => "orchestrator_error",
            AirpError::Volume(_) => "volume_error",
            AirpError::Fsm(_) => "fsm_error",
            AirpError::Internal(_) => "internal_error",
        }
    }
}

/// #67 #9 / PR #74 方案 A：JSON envelope body。
///
/// 与 `daemon::handlers::ModelsProxyError` 同结构（code/message + 可选 upstream_*/detail），
/// 让 webui `formatError` 白名单统一处理 engine 所有错误响应。
#[derive(Debug, Serialize)]
struct AirpErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AirpErrorResponse {
    error: AirpErrorBody,
}

/// M2.3：axum handler 可直接返回 `Result<T, AirpError>`，错误自动映射。
///
/// #67 #9 / PR #74 方案 A：改为 JSON envelope 输出（`{"error":{"code","message"}}`），
/// 让 webui `formatError` 白名单 + extras 折叠生效（之前返回 plain text，白名单
/// 是 dead code）。500 内部错误仍不暴露细节，仅返回通用 message。
impl IntoResponse for AirpError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code_str();
        let message = self.to_string();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(err = %message, "internal error");
            // 500 不暴露细节，仅返回通用 message（与原行为一致）
            let body = AirpErrorResponse {
                error: AirpErrorBody {
                    code,
                    message: "internal error".to_string(),
                },
            };
            (status, Json(body)).into_response()
        } else {
            let body = AirpErrorResponse {
                error: AirpErrorBody { code, message },
            };
            (status, Json(body)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let e = AirpError::BadRequest("missing field".to_string());
        assert!(e.to_string().contains("missing field"));

        let io = AirpError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "x"));
        assert!(io.to_string().contains("I/O"));
    }

    #[test]
    fn test_error_from_io() {
        fn produces() -> AirpResult<()> {
            std::fs::read_to_string("/definitely/does/not/exist/here")?;
            Ok(())
        }
        let r = produces();
        assert!(matches!(r, Err(AirpError::Io(_))));
    }

    // #67 #9 / PR #74 方案 A：envelope 形状回归。webui formatError 依赖此结构。
    #[tokio::test]
    async fn into_response_emits_json_envelope() {
        use axum::body::to_bytes;
        let resp =
            AirpError::NotFound("lorebook for character foo not found".to_string()).into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["code"], "not_found");
        assert_eq!(
            v["error"]["message"],
            "资源不存在: lorebook for character foo not found"
        );
    }

    // 500 不暴露细节（仅 "internal error"），但 code 仍按 variant 输出。
    #[tokio::test]
    async fn into_response_500_redacts_message() {
        use axum::body::to_bytes;
        let resp = AirpError::Internal("db password is hunter2".to_string()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["code"], "internal_error");
        assert_eq!(v["error"]["message"], "internal error");
        assert!(
            !bytes.windows(b"hunter2".len()).any(|w| w == b"hunter2"),
            "500 响应不得泄露内部细节"
        );
    }
}
