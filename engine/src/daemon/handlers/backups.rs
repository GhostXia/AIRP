//! Backup HTTP handlers — create / list / get / verify / delete（#342 E-P2-1）。
//!
//! 端点：
//! - `POST   /v1/backups` — 创建 backup（body: `{ source, scope }`）
//! - `GET    /v1/backups` — 列出所有 backup manifest 摘要
//! - `GET    /v1/backups/:backup_id` — 返回完整 manifest
//! - `POST   /v1/backups/:backup_id/verify` — 校验 backup 完整性
//! - `DELETE /v1/backups/:backup_id` — 删除 backup（不可恢复）
//!
//! handler 只做 HTTP extraction + 调用 `crate::backup` 业务逻辑；staging / hash /
//! 原子 rename / path sandbox 全在 `backup` 模块。
//!
//! ## 并发模型
//!
//! - 全局 `BACKUP_LOCK`（进程内 `std::sync::Mutex`）串行化 backup vs backup /
//!   backup vs verify / backup vs delete。restore（Slice 2）将复用同一锁。
//! - 调用 `backup::*` 的同步函数时，handler 用 `tokio::task::spawn_blocking` 包装，
//!   避免阻塞 tokio worker 线程（与 `search.rs` 既定惯例一致）。

use crate::backup::{
    create_backup, delete_backup, list_backups, read_backup_manifest, restore_backup,
    verify_backup, BackupScope, BackupSource, CreateBackupOptions,
};
use crate::daemon::DaemonState;
use crate::error::AirpError;
use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Request / Response types ─────────────────────────────────────────────────

/// `POST /v1/backups` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct CreateBackupRequest {
    /// backup 来源。仅接受 `"manual"`（HTTP API 不允许直接创建 pre_delete /
    /// pre_restore_rollback / pre_migration backup，那些由系统内部创建）。
    pub source: String,
    /// backup 范围：`"full"` / `"character"` / `"session"`。
    pub scope: BackupScopeRequest,
}

/// `POST /v1/backups` 请求体的 scope 字段。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(in crate::daemon) enum BackupScopeRequest {
    /// 全量 backup（排除 backups/ 自身 + secret 文件）。
    Full,
    /// 单角色子树：`characters/{character_id}/`。
    Character { character_id: String },
    /// 单会话子树：`characters/{character_id}/sessions/{session_id}/`。
    Session {
        character_id: String,
        session_id: String,
    },
}

/// `POST /v1/backups` 响应体。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct CreateBackupResponse {
    pub backup_id: String,
    pub created_at: String,
    pub source: String,
    pub scope: BackupScopeSummary,
    pub files_count: usize,
    pub total_bytes: u64,
    pub tree_sha256: String,
}

/// `GET /v1/backups` 列表项摘要。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct BackupListItem {
    pub backup_id: String,
    pub created_at: String,
    pub source: String,
    pub scope: BackupScopeSummary,
    pub files_count: usize,
    pub total_bytes: u64,
    /// 完整性校验状态：`null` 表示尚未校验，`true` 校验通过，`false` 校验失败。
    /// list 接口不主动校验，恒返回 `null`；客户端按需调 `POST /verify`。
    pub verified: Option<bool>,
}

/// `GET /v1/backups/:backup_id` 完整 manifest 响应。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct BackupManifestResponse {
    pub schema: u32,
    pub backup_id: String,
    pub created_at: String,
    pub engine_version: String,
    pub data_schema_version: u32,
    pub source: String,
    pub scope: BackupScopeSummary,
    pub secrets_excluded: bool,
    pub files: Vec<BackupFileEntry>,
    pub tree_sha256: String,
}

/// `POST /v1/backups/:backup_id/verify` 响应体。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct VerifyBackupResponse {
    pub verified: bool,
    pub checked_files: usize,
    pub tree_sha256: String,
}

/// `POST /v1/backups/:backup_id/restore` 响应体。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct RestoreBackupResponse {
    /// 从哪个 backup 恢复。
    pub restored_from: String,
    /// 自动创建的回滚 backup id（用户可用它恢复到 restore 前的状态）。
    pub rollback_backup_id: String,
    /// post-restore 完整性校验是否通过。
    pub verified: bool,
}

/// scope 的 HTTP 序列化形式（与 `BackupScope` 的 serde 形式一致，但显式定义
/// 避免依赖内部 enum 的 tag 序列化细节）。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::daemon) enum BackupScopeSummary {
    Full,
    Character {
        character_id: String,
    },
    Session {
        character_id: String,
        session_id: String,
    },
    Workspace {
        revision: u64,
    },
}

/// manifest.files 单项的 HTTP 序列化形式。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct BackupFileEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// 校验 `CreateBackupRequest::source` 仅接受 `"manual"`。
fn validate_http_source(source: &str) -> Result<BackupSource, AirpError> {
    match source {
        "manual" => Ok(BackupSource::Manual),
        other => Err(AirpError::BadRequest(format!(
            "POST /v1/backups 仅接受 source=\"manual\"，收到 {:?}。pre_delete / pre_restore_rollback / pre_migration 由系统内部创建",
            other
        ))),
    }
}

/// 把 `BackupScopeRequest` 转成内部 `BackupScope`，并校验 ID 段安全性。
fn parse_scope(req: BackupScopeRequest) -> Result<BackupScope, AirpError> {
    match req {
        BackupScopeRequest::Full => Ok(BackupScope::Full),
        BackupScopeRequest::Character { character_id } => {
            validate_id_segment(&character_id, "character_id")?;
            Ok(BackupScope::Character { character_id })
        }
        BackupScopeRequest::Session {
            character_id,
            session_id,
        } => {
            validate_id_segment(&character_id, "character_id")?;
            validate_id_segment(&session_id, "session_id")?;
            Ok(BackupScope::Session {
                character_id,
                session_id,
            })
        }
    }
}

/// 校验 ID 段：非空、无路径分隔符、无 `..` / `.` / 空字节。
///
/// 复用 `data_dir::security::validate_id_segment` 的语义，但 backup 场景下
/// 不依赖 character 是否存在（删除已删 character 的 backup 仍应能创建）。
fn validate_id_segment(id: &str, field: &str) -> Result<(), AirpError> {
    if id.is_empty() {
        return Err(AirpError::BadRequest(format!("{field} 不能为空")));
    }
    if id.contains('/') || id.contains('\\') {
        return Err(AirpError::BadRequest(format!(
            "{field} 含路径分隔符: {id:?}"
        )));
    }
    if id == "." || id == ".." {
        return Err(AirpError::BadRequest(format!(
            "{field} 不能为 . 或 ..: {id:?}"
        )));
    }
    if id.contains('\0') {
        return Err(AirpError::BadRequest(format!("{field} 含空字节: {id:?}")));
    }
    Ok(())
}

/// 把内部 `BackupScope` 转成 HTTP 响应 summary。
fn scope_to_summary(scope: &BackupScope) -> BackupScopeSummary {
    match scope {
        BackupScope::Full => BackupScopeSummary::Full,
        BackupScope::Character { character_id } => BackupScopeSummary::Character {
            character_id: character_id.clone(),
        },
        BackupScope::Session {
            character_id,
            session_id,
        } => BackupScopeSummary::Session {
            character_id: character_id.clone(),
            session_id: session_id.clone(),
        },
        BackupScope::Workspace { revision } => BackupScopeSummary::Workspace {
            revision: *revision,
        },
    }
}

/// 计算manifest files 的总字节数。
fn total_bytes(files: &[crate::revision::manifest::ApprovedFile]) -> u64 {
    files.iter().map(|f| f.bytes).sum()
}

/// backup_id 路径段校验（防 traversal）。
///
/// 复用 `manifest::validate_backup_id` 作为单一来源，确保 HTTP 入口与 manifest
/// 加载阶段校验规则一致（#450）。manifest 校验更严格（仅允许 alphanumeric +
/// `-` + `_`，拒绝 `:` 等 Windows 非法文件名字符），HTTP 入口采用同一规则。
fn validate_backup_id_segment(backup_id: &str) -> Result<(), AirpError> {
    crate::backup::manifest::validate_backup_id(backup_id)
        .map_err(|e| AirpError::BadRequest(format!("backup_id 非法: {e}")))
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /v1/backups` — 创建一个 backup。
///
/// 同步 IO（文件 walk + 复制 + hash）在 `spawn_blocking` 中执行。
pub(in crate::daemon) async fn create_backup_endpoint(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<CreateBackupRequest>,
) -> Result<Json<CreateBackupResponse>, AirpError> {
    let source = validate_http_source(&req.source)?;
    let scope = parse_scope(req.scope)?;

    let data_root = state.data_root.clone();
    let opts = CreateBackupOptions {
        data_root: data_root.clone(),
        source,
        scope,
    };

    let created = tokio::task::spawn_blocking(move || create_backup(&opts))
        .await
        .map_err(|e| AirpError::Internal(format!("backup create task panicked: {e}")))??;

    Ok(Json(CreateBackupResponse {
        backup_id: created.backup_id,
        created_at: created.manifest.created_at.clone(),
        source: created.manifest.source.as_str().to_string(),
        scope: scope_to_summary(&created.manifest.scope),
        files_count: created.manifest.files.len(),
        total_bytes: total_bytes(&created.manifest.files),
        tree_sha256: created.manifest.tree_sha256.clone(),
    }))
}

/// `GET /v1/backups` — 列出所有 backup 摘要（按 created_at 降序）。
pub(in crate::daemon) async fn list_backups_endpoint(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<BackupListItem>>, AirpError> {
    let data_root = state.data_root.clone();
    let manifests = tokio::task::spawn_blocking(move || list_backups(&data_root))
        .await
        .map_err(|e| AirpError::Internal(format!("backup list task panicked: {e}")))??;

    let items = manifests
        .into_iter()
        .map(|m| BackupListItem {
            backup_id: m.backup_id.clone(),
            created_at: m.created_at.clone(),
            source: m.source.as_str().to_string(),
            scope: scope_to_summary(&m.scope),
            files_count: m.files.len(),
            total_bytes: total_bytes(&m.files),
            verified: None,
        })
        .collect();

    Ok(Json(items))
}

/// `GET /v1/backups/:backup_id` — 返回完整 manifest。
pub(in crate::daemon) async fn get_backup_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(backup_id): Path<String>,
) -> Result<Json<BackupManifestResponse>, AirpError> {
    validate_backup_id_segment(&backup_id)?;

    let data_root = state.data_root.clone();
    let bid = backup_id.clone();
    let manifest = tokio::task::spawn_blocking(move || read_backup_manifest(&data_root, &bid))
        .await
        .map_err(|e| AirpError::Internal(format!("backup get task panicked: {e}")))??;

    Ok(Json(BackupManifestResponse {
        schema: manifest.schema,
        backup_id: manifest.backup_id,
        created_at: manifest.created_at,
        engine_version: manifest.engine_version,
        data_schema_version: manifest.data_schema_version,
        source: manifest.source.as_str().to_string(),
        scope: scope_to_summary(&manifest.scope),
        secrets_excluded: manifest.secrets_excluded,
        files: manifest
            .files
            .into_iter()
            .map(|f| BackupFileEntry {
                path: f.path,
                sha256: f.sha256,
                bytes: f.bytes,
            })
            .collect(),
        tree_sha256: manifest.tree_sha256,
    }))
}

/// `POST /v1/backups/:backup_id/verify` — 校验 backup 完整性。
pub(in crate::daemon) async fn verify_backup_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(backup_id): Path<String>,
) -> Result<Json<VerifyBackupResponse>, AirpError> {
    validate_backup_id_segment(&backup_id)?;

    let data_root = state.data_root.clone();
    let bid = backup_id.clone();
    let (checked, tree) = tokio::task::spawn_blocking(move || verify_backup(&data_root, &bid))
        .await
        .map_err(|e| AirpError::Internal(format!("backup verify task panicked: {e}")))??;

    Ok(Json(VerifyBackupResponse {
        verified: true,
        checked_files: checked,
        tree_sha256: tree,
    }))
}

/// `POST /v1/backups/:backup_id/restore` — 从指定 backup 恢复 data_root。
///
/// 自动创建回滚 backup → staging → swap → post-verify。
/// 失败时保留 staging + rollback backup，不清理现场，返回 Internal。
///
/// **注意**：restore 会覆盖 data_root 下所有非 backup 文件。调用方（WebUI）
/// 应在确认对话框中明确警告用户：secrets 需手动重配、建议 restore 后重启 daemon。
pub(in crate::daemon) async fn restore_backup_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(backup_id): Path<String>,
) -> Result<Json<RestoreBackupResponse>, AirpError> {
    validate_backup_id_segment(&backup_id)?;

    let data_root = state.data_root.clone();
    let bid = backup_id.clone();
    let (restored_from, rollback_id) =
        tokio::task::spawn_blocking(move || restore_backup(&data_root, &bid))
            .await
            .map_err(|e| AirpError::Internal(format!("backup restore task panicked: {e}")))??;

    Ok(Json(RestoreBackupResponse {
        restored_from,
        rollback_backup_id: rollback_id,
        verified: true,
    }))
}

/// `DELETE /v1/backups/:backup_id` — 删除指定 backup（不可恢复）。
pub(in crate::daemon) async fn delete_backup_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(backup_id): Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    validate_backup_id_segment(&backup_id)?;

    let data_root = state.data_root.clone();
    let bid = backup_id.clone();
    tokio::task::spawn_blocking(move || delete_backup(&data_root, &bid))
        .await
        .map_err(|e| AirpError::Internal(format!("backup delete task panicked: {e}")))??;

    Ok(Json(serde_json::json!({
        "deleted": true,
        "backup_id": backup_id,
    })))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::daemon::tests::make_state_no_key as make_state_for_http_test;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    async fn post_backup(
        app: axum::Router,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/backups")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, v)
    }

    async fn list_backups(app: axum::Router) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/backups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, v)
    }

    #[tokio::test]
    async fn create_full_backup_returns_manifest_summary() {
        let (state, _tmp) = make_state_for_http_test();
        std::fs::write(state.data_root.join("a.txt"), "hello").unwrap();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (status, v) = post_backup(app, body).await;

        assert_eq!(status, StatusCode::OK, "create should succeed: {:?}", v);
        assert_eq!(v["source"], "manual");
        assert_eq!(v["scope"]["kind"], "full");
        assert_eq!(v["files_count"], 1);
        assert!(v["total_bytes"].as_u64().unwrap() > 0);
        assert!(v["backup_id"].as_str().unwrap().len() == 32);
        assert!(v["tree_sha256"].as_str().unwrap().len() == 64);
    }

    #[tokio::test]
    async fn create_character_scoped_backup() {
        let (state, _tmp) = make_state_for_http_test();
        let char_dir = state.data_root.join("characters").join("alice");
        std::fs::create_dir_all(&char_dir).unwrap();
        std::fs::write(char_dir.join("card.json"), "{}").unwrap();
        // 其他角色不应被包含
        let bob_dir = state.data_root.join("characters").join("bob");
        std::fs::create_dir_all(&bob_dir).unwrap();
        std::fs::write(bob_dir.join("card.json"), "{}").unwrap();

        let app = crate::daemon::create_router(state.clone());
        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "character", "character_id": "alice" }
        });
        let (status, v) = post_backup(app, body).await;

        assert_eq!(status, StatusCode::OK, "{:?}", v);
        assert_eq!(v["scope"]["kind"], "character");
        assert_eq!(v["scope"]["character_id"], "alice");
        assert_eq!(v["files_count"], 1);
    }

    #[tokio::test]
    async fn create_session_scoped_backup() {
        let (state, _tmp) = make_state_for_http_test();
        let sess_dir = state
            .data_root
            .join("characters")
            .join("alice")
            .join("sessions")
            .join("sess1");
        std::fs::create_dir_all(&sess_dir).unwrap();
        std::fs::write(sess_dir.join("current.md"), "x").unwrap();

        let app = crate::daemon::create_router(state.clone());
        let body = serde_json::json!({
            "source": "manual",
            "scope": {
                "kind": "session",
                "character_id": "alice",
                "session_id": "sess1"
            }
        });
        let (status, v) = post_backup(app, body).await;

        assert_eq!(status, StatusCode::OK, "{:?}", v);
        assert_eq!(v["scope"]["kind"], "session");
        assert_eq!(v["files_count"], 1);
    }

    #[tokio::test]
    async fn create_backup_rejects_non_manual_source() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "pre_delete",
            "scope": { "kind": "full" }
        });
        let (status, v) = post_backup(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(v["error"]["message"].as_str().unwrap().contains("manual"));
    }

    #[tokio::test]
    async fn create_backup_rejects_internal_workspace_scope() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "workspace", "revision": 7 }
        });
        let (status, _v) = post_backup(app, body).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn workspace_scope_summary_is_transparent() {
        let summary =
            super::scope_to_summary(&crate::backup::BackupScope::Workspace { revision: 7 });
        let value = serde_json::to_value(summary).unwrap();

        assert_eq!(value["kind"], "workspace");
        assert_eq!(value["revision"], 7);
    }

    #[tokio::test]
    async fn create_backup_rejects_traversal_character_id() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "character", "character_id": "../escape" }
        });
        let (status, _v) = post_backup(app, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_backups_returns_created_backups_desc() {
        let (state, _tmp) = make_state_for_http_test();
        std::fs::write(state.data_root.join("a.txt"), "x").unwrap();
        let app = crate::daemon::create_router(state.clone());

        // 创建 2 个 backup
        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (s1, v1) = post_backup(app.clone(), body.clone()).await;
        assert_eq!(s1, StatusCode::OK);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let (s2, v2) = post_backup(app.clone(), body).await;
        assert_eq!(s2, StatusCode::OK);

        let (status, v) = list_backups(app).await;
        assert_eq!(status, StatusCode::OK);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // 降序：v2 在前
        assert_eq!(arr[0]["backup_id"], v2["backup_id"]);
        assert_eq!(arr[1]["backup_id"], v1["backup_id"]);
        // verified 字段为 null
        assert!(arr[0]["verified"].is_null());
    }

    #[tokio::test]
    async fn list_backups_empty_returns_empty_array() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());
        let (status, v) = list_backups(app).await;
        assert_eq!(status, StatusCode::OK);
        assert!(v.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_backup_returns_full_manifest() {
        let (state, _tmp) = make_state_for_http_test();
        std::fs::write(state.data_root.join("a.txt"), "hello").unwrap();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (_, created) = post_backup(app.clone(), body).await;
        let backup_id = created["backup_id"].as_str().unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/backups/{backup_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["backup_id"], backup_id);
        assert_eq!(v["schema"], 1);
        assert_eq!(v["source"], "manual");
        assert_eq!(v["secrets_excluded"], true);
        assert!(v["files"].is_array());
        assert_eq!(v["files"].as_array().unwrap().len(), 1);
        assert_eq!(v["files"][0]["path"], "a.txt");
    }

    #[tokio::test]
    async fn get_backup_returns_404_for_missing() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/backups/nonexistent123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_backup_rejects_traversal_backup_id() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/backups/..%2Fescape")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum 会先路由拒绝（路径含 ..），或 handler 校验拒绝
        assert!(resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn verify_backup_passes_for_fresh_backup() {
        let (state, _tmp) = make_state_for_http_test();
        std::fs::write(state.data_root.join("a.txt"), "hello").unwrap();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (_, created) = post_backup(app.clone(), body).await;
        let backup_id = created["backup_id"].as_str().unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/backups/{backup_id}/verify"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["verified"], true);
        assert_eq!(v["checked_files"], 1);
    }

    #[tokio::test]
    async fn verify_backup_detects_tampered_file() {
        let (state, _tmp) = make_state_for_http_test();
        std::fs::write(state.data_root.join("a.txt"), "hello").unwrap();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (_, created) = post_backup(app.clone(), body).await;
        let backup_id = created["backup_id"].as_str().unwrap();

        // 篡改 backup 内的文件
        let backup_dir = state.data_root.join("backups").join(backup_id);
        std::fs::write(backup_dir.join("files").join("a.txt"), "tampered").unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/backups/{backup_id}/verify"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn delete_backup_removes_directory() {
        let (state, _tmp) = make_state_for_http_test();
        std::fs::write(state.data_root.join("a.txt"), "x").unwrap();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (_, created) = post_backup(app.clone(), body).await;
        let backup_id = created["backup_id"].as_str().unwrap();
        let backup_dir = state.data_root.join("backups").join(backup_id);
        assert!(backup_dir.is_dir());

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/backups/{backup_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!backup_dir.exists());

        // list 不再包含
        let (_, v) = list_backups(app).await;
        assert!(v.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_backup_returns_404_for_missing() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/backups/nonexistent123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn backup_excludes_secrets_from_files_list() {
        let (state, _tmp) = make_state_for_http_test();
        // 写入 secret 文件 + 普通文件
        std::fs::write(state.data_root.join("secrets.json"), r#"{"key":"x"}"#).unwrap();
        std::fs::write(state.data_root.join("settings.json"), r#"{"api_key":"y"}"#).unwrap();
        std::fs::write(state.data_root.join("providers.json"), "{}").unwrap();

        let app = crate::daemon::create_router(state.clone());
        let body = serde_json::json!({
            "source": "manual",
            "scope": { "kind": "full" }
        });
        let (_, created) = post_backup(app.clone(), body).await;
        let backup_id = created["backup_id"].as_str().unwrap();

        // get full manifest
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/backups/{backup_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let files = v["files"].as_array().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert!(!paths.contains(&"secrets.json"), "secrets.json 必须被排除");
        assert!(
            !paths.contains(&"settings.json"),
            "settings.json 必须被排除"
        );
        assert!(paths.contains(&"providers.json"), "providers.json 应保留");
        assert_eq!(v["secrets_excluded"], true);
    }
}
