//! Authenticated HTTP adapter for the durable, layout-only Workspace asset.

use std::{collections::BTreeMap, sync::Arc};

use airp_state_protocol::{
    Area, Blueprint, Capability, Layout, LayoutKind, SurfaceRevision, Theme, WidgetInstance,
    WORKSPACE_MAX_DOCUMENT_BYTES,
};
use axum::{
    body::Body,
    extract::{rejection::JsonRejection, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    daemon::DaemonState,
    domain::{WorkspaceCommand, WorkspaceHistoryEntry, WorkspaceService},
    error::AirpError,
    types::UserId,
};

pub(in crate::daemon) const WORKSPACE_HTTP_MAX_BODY_BYTES: usize = 64 * 1024;
pub(in crate::daemon) const WORKSPACE_MIGRATION_HTTP_MAX_BODY_BYTES: usize =
    WORKSPACE_MAX_DOCUMENT_BYTES;
const WORKSPACE_HISTORY_DEFAULT_LIMIT: usize = 50;
const WORKSPACE_HISTORY_MAX_LIMIT: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceScopeQuery {
    user_id: Option<UserId>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceHistoryQuery {
    user_id: Option<UserId>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceCommandRequest {
    expected_revision: SurfaceRevision,
    command: WorkspaceCommand,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceRollbackRequest {
    expected_revision: SurfaceRevision,
    target_revision: SurfaceRevision,
}

#[derive(Debug, Serialize)]
struct WorkspaceHistoryResponse {
    entries: Vec<WorkspaceHistoryEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintV1Source {
    version: String,
    profile: Option<String>,
    theme: Option<BlueprintV1Theme>,
    layout: BlueprintV1Layout,
    widgets: Vec<BlueprintV1Widget>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintV1Theme {
    name: String,
    tokens: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintV1Layout {
    #[serde(rename = "type")]
    kind: LayoutKind,
    areas: Vec<BlueprintV1Area>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintV1Area {
    id: String,
    widgets: Vec<String>,
    props: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintV1Widget {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    props: Option<serde_json::Value>,
    state: Option<String>,
    capabilities: Option<Vec<Capability>>,
}

impl From<BlueprintV1Source> for Blueprint {
    fn from(source: BlueprintV1Source) -> Self {
        Self {
            version: source.version,
            profile: source.profile,
            theme: source.theme.map(|theme| Theme {
                name: theme.name,
                tokens: theme.tokens,
            }),
            layout: Layout {
                kind: source.layout.kind,
                areas: source
                    .layout
                    .areas
                    .into_iter()
                    .map(|area| Area {
                        id: area.id,
                        widgets: area.widgets,
                        props: area.props,
                    })
                    .collect(),
            },
            widgets: source
                .widgets
                .into_iter()
                .map(|widget| WidgetInstance {
                    id: widget.id,
                    kind: widget.kind,
                    props: widget.props,
                    state: widget.state,
                    capabilities: widget.capabilities,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceMigrationDryRunRequest {
    source: BlueprintV1Source,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceMigrationApplyRequest {
    expected_revision: SurfaceRevision,
    source: BlueprintV1Source,
    planned_source_sha256: String,
    planned_candidate_sha256: String,
    planned_converter_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct WorkspaceMigrationRollbackRequest {
    expected_revision: SurfaceRevision,
    backup_id: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceMigrationDryRunResponse {
    source: String,
    source_sha256: String,
    candidate_sha256: String,
    converter_version: String,
    workspace: airp_state_protocol::WorkspaceDocumentV1,
    warnings: Vec<String>,
    writes_performed: bool,
    recovery: &'static str,
}

#[derive(Debug, Serialize)]
struct WorkspaceMigrationApplyResponse {
    workspace: airp_state_protocol::WorkspaceDocumentV1,
    backup_id: String,
    source_sha256: String,
    candidate_sha256: String,
    converter_version: String,
    recovery: &'static str,
}

#[derive(Debug, Serialize)]
struct WorkspaceMigrationRollbackResponse {
    workspace: airp_state_protocol::WorkspaceDocumentV1,
    backup_id: String,
    recovery: &'static str,
}

pub(in crate::daemon) async fn get_workspace_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    match blocking(move || service.read()).await {
        Ok(document) => Json(document).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn workspace_command_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
    payload: Result<Json<WorkspaceCommandRequest>, JsonRejection>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return workspace_request_rejection(rejection),
    };
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    match blocking(move || service.execute(request.expected_revision.value(), request.command))
        .await
    {
        Ok(document) => Json(document).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn get_workspace_history_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceHistoryQuery>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let limit = query.limit.unwrap_or(WORKSPACE_HISTORY_DEFAULT_LIMIT);
    if !(1..=WORKSPACE_HISTORY_MAX_LIMIT).contains(&limit) {
        return AirpError::BadRequest("workspace history limit must be between 1 and 256".into())
            .into_response();
    }
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    match blocking(move || service.history(limit)).await {
        Ok(entries) => Json(WorkspaceHistoryResponse { entries }).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn export_workspace_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    match blocking(move || service.export()).await {
        Ok(export) => {
            let mut response = Response::new(Body::from(export.raw_json));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            response.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"airp-workspace-default.json\""),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response.headers_mut().insert(
                "x-airp-workspace-schema",
                HeaderValue::from_str(&export.schema.to_string())
                    .expect("numeric schema creates a valid header"),
            );
            response.headers_mut().insert(
                "x-airp-workspace-sha256",
                HeaderValue::from_str(&export.sha256).expect("hex digest creates a valid header"),
            );
            response
        }
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn rollback_workspace_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
    payload: Result<Json<WorkspaceRollbackRequest>, JsonRejection>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return workspace_request_rejection(rejection),
    };
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    match blocking(move || {
        service.rollback(
            request.expected_revision.value(),
            request.target_revision.value(),
        )
    })
    .await
    {
        Ok(document) => Json(document).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn dry_run_workspace_migration_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
    payload: Result<Json<WorkspaceMigrationDryRunRequest>, JsonRejection>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return workspace_request_rejection(rejection),
    };
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    match blocking(move || service.plan_v1_migration(&request.source.into())).await {
        Ok(plan) => Json(WorkspaceMigrationDryRunResponse {
            source: plan.source,
            source_sha256: plan.source_sha256,
            candidate_sha256: plan.candidate_sha256,
            converter_version: plan.converter_version,
            workspace: plan.candidate,
            warnings: plan.warnings,
            writes_performed: plan.writes_performed,
            recovery: "review_and_apply",
        })
        .into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn apply_workspace_migration_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
    payload: Result<Json<WorkspaceMigrationApplyRequest>, JsonRejection>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return workspace_request_rejection(rejection),
    };
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    let candidate_sha256 = request.planned_candidate_sha256.clone();
    let converter_version = request.planned_converter_version.clone();
    match blocking(move || {
        service.apply_v1_migration(
            request.expected_revision.value(),
            &request.source.into(),
            &request.planned_source_sha256,
            &request.planned_candidate_sha256,
            &request.planned_converter_version,
        )
    })
    .await
    {
        Ok(applied) => Json(WorkspaceMigrationApplyResponse {
            workspace: applied.document,
            backup_id: applied.backup_id,
            source_sha256: applied.source_sha256,
            candidate_sha256,
            converter_version,
            recovery: "rollback_with_backup_id",
        })
        .into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn rollback_workspace_migration_endpoint(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WorkspaceScopeQuery>,
    payload: Result<Json<WorkspaceMigrationRollbackRequest>, JsonRejection>,
) -> Response {
    if !workspace_auth_is_configured(&state) {
        return workspace_auth_unavailable();
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return workspace_request_rejection(rejection),
    };
    let service = match workspace_service(&state, query.user_id.as_ref()) {
        Ok(service) => service,
        Err(error) => return error.into_response(),
    };
    let backup_id = request.backup_id.clone();
    match blocking(move || {
        service.rollback_migration_backup(request.expected_revision.value(), &request.backup_id)
    })
    .await
    {
        Ok(workspace) => Json(WorkspaceMigrationRollbackResponse {
            workspace,
            backup_id,
            recovery: "forward_rollback_completed",
        })
        .into_response(),
        Err(error) => error.into_response(),
    }
}

fn workspace_service(
    state: &DaemonState,
    user_id: Option<&UserId>,
) -> Result<WorkspaceService, AirpError> {
    let root =
        crate::data_dir::resolve_effective_root(&state.data_root, user_id.map(UserId::as_str))?;
    Ok(WorkspaceService::new(root))
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, AirpError> + Send + 'static,
) -> Result<T, AirpError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| AirpError::Internal(format!("workspace task failed: {error}")))?
}

fn workspace_auth_is_configured(state: &DaemonState) -> bool {
    state
        .read_config()
        .access_api_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
}

fn workspace_auth_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": {
                "code": "workspace_auth_unavailable",
                "message": "Workspace API requires daemon bearer authentication",
                "recovery": "configure_access_key"
            }
        })),
    )
        .into_response()
}

fn workspace_request_rejection(rejection: JsonRejection) -> Response {
    let status = if rejection.status() == StatusCode::UNPROCESSABLE_ENTITY {
        StatusCode::BAD_REQUEST
    } else {
        rejection.status()
    };
    (
        status,
        Json(serde_json::json!({
            "error": {
                "code": if status == StatusCode::PAYLOAD_TOO_LARGE { "payload_too_large" } else { "bad_request" },
                "message": if status == StatusCode::PAYLOAD_TOO_LARGE { "Workspace request body is too large" } else { "Invalid Workspace request body" },
                "recovery": "correct_request"
            }
        })),
    )
        .into_response()
}
