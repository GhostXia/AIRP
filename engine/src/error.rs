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

/// Schema version for the public JSON error envelope.
pub const AIRP_ERROR_SCHEMA_VERSION: u32 = 1;

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

    /// 乐观并发冲突：read-modify-write 期间检测到资源被并发改动。
    /// 映射到 HTTP 409 Conflict。
    #[error("冲突: {0}")]
    Conflict(String),

    /// Workspace optimistic-concurrency failure with machine-readable revisions.
    #[error("workspace revision conflict: expected {expected}, current {current}")]
    WorkspaceRevisionConflict { expected: u64, current: u64 },

    /// A persisted workspace was written by a newer incompatible schema.
    #[error("unsupported workspace schema major {actual}; supported major is {supported}")]
    WorkspaceUnsupportedMajor { actual: u16, supported: u16 },

    /// Migration backup is durable but the following Workspace commit failed.
    #[error(
        "workspace migration recovery commit failed; verified backup {backup_id} was retained"
    )]
    WorkspaceMigrationCommitFailed { backup_id: String },

    /// The commit returned an error and durable authority could not be read
    /// back conclusively. Clients must re-read before choosing recovery.
    #[error(
        "workspace migration recovery outcome is unknown; verified backup {backup_id} was retained"
    )]
    WorkspaceMigrationOutcomeUnknown { backup_id: String },

    /// A restore failed after its recovery backup became durable, but before
    /// the authoritative data swap started.
    #[error("backup restore failed; verified recovery backup {backup_id} was retained")]
    BackupRestoreFailed { backup_id: String },

    /// A restore failed after entering the authoritative data swap. The
    /// retained recovery backup is durable, but callers must inspect current
    /// state before choosing recovery.
    #[error(
        "backup restore outcome is unknown; verified recovery backup {backup_id} was retained"
    )]
    BackupRestoreOutcomeUnknown { backup_id: String },

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

    /// 4.3 FTS5：SQLite 数据库操作失败。
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// 项目内约定的 Result 别名。
pub type AirpResult<T> = Result<T, AirpError>;

impl AirpError {
    /// M2.3：错误到 HTTP 状态码的映射。
    pub fn status(&self) -> StatusCode {
        match self {
            AirpError::BadRequest(_) | AirpError::PathEscape(_) => StatusCode::BAD_REQUEST,
            AirpError::NotFound(_) => StatusCode::NOT_FOUND,
            AirpError::Conflict(_)
            | AirpError::WorkspaceRevisionConflict { .. }
            | AirpError::WorkspaceUnsupportedMajor { .. } => StatusCode::CONFLICT,
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
            | AirpError::Sqlite(_)
            | AirpError::WorkspaceMigrationCommitFailed { .. }
            | AirpError::WorkspaceMigrationOutcomeUnknown { .. }
            | AirpError::BackupRestoreFailed { .. }
            | AirpError::BackupRestoreOutcomeUnknown { .. }
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
            AirpError::Conflict(_) => "conflict",
            AirpError::WorkspaceRevisionConflict { .. } => "workspace_revision_conflict",
            AirpError::WorkspaceUnsupportedMajor { .. } => "workspace_unsupported_major",
            AirpError::WorkspaceMigrationCommitFailed { .. } => "workspace_migration_commit_failed",
            AirpError::WorkspaceMigrationOutcomeUnknown { .. } => {
                "workspace_migration_outcome_unknown"
            }
            AirpError::BackupRestoreFailed { .. } => "backup_restore_failed",
            AirpError::BackupRestoreOutcomeUnknown { .. } => "backup_restore_outcome_unknown",
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
            AirpError::Sqlite(_) => "sqlite_error",
            AirpError::Internal(_) => "internal_error",
        }
    }

    /// Stable client-facing message that never includes internal or upstream details.
    pub fn public_message(&self) -> String {
        match self {
            AirpError::Upstream { .. } => "upstream request failed".to_string(),
            AirpError::PathEscape(_) => "invalid path".to_string(),
            error if error.status() == StatusCode::INTERNAL_SERVER_ERROR => {
                "internal error".to_string()
            }
            error => error.to_string(),
        }
    }

    /// Stable client recovery category. It deliberately does not include
    /// provider-private or internal diagnostic details.
    pub fn recovery_str(&self) -> &'static str {
        match self {
            AirpError::BadRequest(_) | AirpError::PathEscape(_) | AirpError::NotFound(_) => {
                "correct_request"
            }
            AirpError::Conflict(_) => "refresh_and_retry",
            AirpError::WorkspaceRevisionConflict { .. } => "refresh_and_retry",
            AirpError::WorkspaceUnsupportedMajor { .. } => "export_or_upgrade",
            AirpError::WorkspaceMigrationCommitFailed { .. } => "retain_backup_and_inspect",
            AirpError::WorkspaceMigrationOutcomeUnknown { .. } => "refresh_before_recovery",
            AirpError::BackupRestoreFailed { .. } => "retain_backup_and_inspect",
            AirpError::BackupRestoreOutcomeUnknown { .. } => "refresh_before_recovery",
            AirpError::Upstream { .. } => "retry_with_backoff",
            AirpError::QuotaExceeded(_) => "wait_or_reduce_usage",
            AirpError::Io(_)
            | AirpError::Json(_)
            | AirpError::Regex(_)
            | AirpError::Config(_)
            | AirpError::Orchestrator(_)
            | AirpError::Volume(_)
            | AirpError::Fsm(_)
            | AirpError::Sqlite(_)
            | AirpError::Http(_)
            | AirpError::Internal(_) => "inspect_server_logs",
        }
    }
}

/// #67 #9 / PR #74 方案 A：JSON envelope body。
///
/// 与 `daemon::handlers::ModelsProxyError` 同结构（code/message + 可选 upstream_*/detail），
/// 让 webui `formatError` 白名单统一处理 engine 所有错误响应。
#[derive(Debug, Serialize)]
struct AirpErrorBody {
    schema_version: u32,
    code: &'static str,
    message: String,
    recovery: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual_major: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported_major: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_id: Option<String>,
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
        let recovery = self.recovery_str();
        let internal_message = self.to_string();
        let message = self.public_message();
        let (current_revision, actual_major, supported_major, backup_id) = match &self {
            AirpError::WorkspaceRevisionConflict { current, .. } => {
                (Some(current.to_string()), None, None, None)
            }
            AirpError::WorkspaceUnsupportedMajor { actual, supported } => {
                (None, Some(*actual), Some(*supported), None)
            }
            AirpError::WorkspaceMigrationCommitFailed { backup_id }
            | AirpError::WorkspaceMigrationOutcomeUnknown { backup_id }
            | AirpError::BackupRestoreFailed { backup_id }
            | AirpError::BackupRestoreOutcomeUnknown { backup_id } => {
                (None, None, None, Some(backup_id.clone()))
            }
            _ => (None, None, None, None),
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(err = %internal_message, "internal error");
        }
        let body = AirpErrorResponse {
            error: AirpErrorBody {
                schema_version: AIRP_ERROR_SCHEMA_VERSION,
                code,
                message,
                recovery,
                current_revision,
                actual_major,
                supported_major,
                backup_id,
            },
        };
        (status, Json(body)).into_response()
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

    #[test]
    fn public_message_hides_internal_and_upstream_details() {
        let io = AirpError::Io(std::io::Error::other("secret path"));
        assert_eq!(io.public_message(), "internal error");
        let upstream = AirpError::Upstream {
            status: 502,
            body: "provider secret".to_string(),
        };
        assert_eq!(upstream.public_message(), "upstream request failed");
        let path = AirpError::PathEscape(PathBuf::from("/srv/private/users/alice"));
        assert_eq!(path.public_message(), "invalid path");
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
        assert_eq!(v["error"]["schema_version"], AIRP_ERROR_SCHEMA_VERSION);
        assert_eq!(v["error"]["code"], "not_found");
        assert_eq!(v["error"]["recovery"], "correct_request");
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
        assert_eq!(v["error"]["recovery"], "inspect_server_logs");
        assert_eq!(v["error"]["message"], "internal error");
        assert!(
            !bytes.windows(b"hunter2".len()).any(|w| w == b"hunter2"),
            "500 响应不得泄露内部细节"
        );
    }

    #[tokio::test]
    async fn migration_commit_failure_exposes_only_the_recovery_backup_id() {
        use axum::body::to_bytes;
        let resp = AirpError::WorkspaceMigrationCommitFailed {
            backup_id: "0123456789abcdef0123456789abcdef".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["code"], "workspace_migration_commit_failed");
        assert_eq!(value["error"]["recovery"], "retain_backup_and_inspect");
        assert_eq!(
            value["error"]["backup_id"],
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(value["error"]["message"], "internal error");
    }

    #[tokio::test]
    async fn migration_unknown_outcome_requires_refresh_and_exposes_backup_id() {
        use axum::body::to_bytes;
        let resp = AirpError::WorkspaceMigrationOutcomeUnknown {
            backup_id: "fedcba9876543210fedcba9876543210".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["error"]["code"],
            "workspace_migration_outcome_unknown"
        );
        assert_eq!(value["error"]["recovery"], "refresh_before_recovery");
        assert_eq!(
            value["error"]["backup_id"],
            "fedcba9876543210fedcba9876543210"
        );
        assert_eq!(value["error"]["message"], "internal error");
    }

    #[tokio::test]
    async fn restore_failure_exposes_only_retained_backup_recovery_fields() {
        use axum::body::to_bytes;
        let resp = AirpError::BackupRestoreFailed {
            backup_id: "0123456789abcdef0123456789abcdef".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["code"], "backup_restore_failed");
        assert_eq!(value["error"]["recovery"], "retain_backup_and_inspect");
        assert_eq!(
            value["error"]["backup_id"],
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(value["error"]["message"], "internal error");
    }

    #[tokio::test]
    async fn restore_unknown_outcome_requires_inspection_and_exposes_backup_id() {
        use axum::body::to_bytes;
        let resp = AirpError::BackupRestoreOutcomeUnknown {
            backup_id: "fedcba9876543210fedcba9876543210".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["code"], "backup_restore_outcome_unknown");
        assert_eq!(value["error"]["recovery"], "refresh_before_recovery");
        assert_eq!(
            value["error"]["backup_id"],
            "fedcba9876543210fedcba9876543210"
        );
        assert_eq!(value["error"]["message"], "internal error");
    }

    #[tokio::test]
    async fn into_response_path_escape_redacts_server_path() {
        use axum::body::to_bytes;
        let resp = AirpError::PathEscape(PathBuf::from("/srv/private/users/alice")).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["code"], "path_escape");
        assert_eq!(value["error"]["message"], "invalid path");
        assert!(!String::from_utf8_lossy(&bytes).contains("/srv/private"));
    }

    #[tokio::test]
    async fn upstream_error_envelope_is_versioned_recoverable_and_redacted() {
        use axum::body::to_bytes;
        let resp = AirpError::Upstream {
            status: 503,
            body: "provider request id and private body".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["schema_version"], AIRP_ERROR_SCHEMA_VERSION);
        assert_eq!(value["error"]["code"], "upstream");
        assert_eq!(value["error"]["recovery"], "retry_with_backoff");
        assert_eq!(value["error"]["message"], "upstream request failed");
        assert!(!String::from_utf8_lossy(&bytes).contains("private body"));
    }
}
