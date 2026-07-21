//! Style handlers: 风格系统 API（4.1 Style Review + 4.2 Soul-Drift）。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::CharacterId;

#[derive(Debug, Deserialize)]
pub struct StyleReviewRequest {
    pub character_id: String,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StyleReviewResponse {
    pub report: crate::style::StyleReviewReport,
    pub drift_applied: bool,
}

#[derive(Debug, Serialize)]
pub struct DriftResponse {
    pub content: String,
    pub char_count: usize,
    pub capacity: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDriftRequest {
    pub content: String,
}

/// POST /v1/style/review
pub async fn style_review(
    State(state): State<Arc<DaemonState>>,
    Json(payload): Json<StyleReviewRequest>,
) -> impl IntoResponse {
    let result = run_style_review_handler(&state, payload).await;
    match result {
        Ok(resp) => match serde_json::to_value(resp) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => AirpError::from(e).into_response(),
        },
        Err(e) => e.into_response(),
    }
}

async fn run_style_review_handler(
    state: &DaemonState,
    payload: StyleReviewRequest,
) -> Result<StyleReviewResponse, AirpError> {
    let cid = CharacterId::new(&payload.character_id)?;
    let sid = payload
        .session_id
        .as_ref()
        .map(|s| crate::types::SessionId::parse(s))
        .transpose()?;

    // 审计修复：校验 profile_id 防止路径遍历，仅允许字母数字下划线连字符。
    let profile_id = payload.profile_id.as_deref().unwrap_or("default");
    let profile_id = if profile_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        profile_id
    } else {
        "default"
    };
    let profile_path = state
        .data_root
        .join("styles")
        .join("profiles")
        .join(format!("{}.md", profile_id));
    // 审计修复：NotFound 返回空 profile，其他 I/O 错误向上传播。
    let style_profile = std::fs::read_to_string(&profile_path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(AirpError::from(e))
        }
    })?;

    let history = crate::domain::ChatService::new(&state.data_root).history(&cid, sid.as_ref())?;
    let recent_messages: Vec<String> = history
        .messages
        .iter()
        .filter(|m| m.role == crate::adapter::MessageRole::Assistant)
        .rev()
        .take(10)
        .map(|m| m.content.clone())
        .collect();

    let current_drift = crate::style::read_soul_drift(&state.data_root, cid.as_str())?;

    let snapshot = state
        .config
        .read()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?
        .clone();

    let provider_config = Arc::new(crate::adapter::ProviderConfig {
        provider: snapshot.provider.clone(),
        endpoint: snapshot.endpoint.clone(),
        api_key: snapshot.api_key.clone(),
    });

    let gen_params = crate::adapter::GenerationParams {
        model: snapshot.model.clone(),
        temperature: Some(0.2),
        max_tokens: Some(1000),
    };

    let report = crate::style::run_style_review(
        &state.http_client,
        provider_config,
        gen_params,
        &style_profile,
        &recent_messages,
        &current_drift,
    )
    .await?;

    let mut drift_applied = false;
    if !report.drift_patch.trim().is_empty() {
        crate::style::append_soul_drift(&state.data_root, cid.as_str(), &report.drift_patch)?;
        drift_applied = true;
    }

    Ok(StyleReviewResponse {
        report,
        drift_applied,
    })
}

/// GET /v1/characters/:character_id/drift
pub async fn get_drift(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> impl IntoResponse {
    let result = (|| -> Result<DriftResponse, AirpError> {
        let cid = CharacterId::new(&character_id)?;
        let content = crate::style::read_soul_drift(&state.data_root, cid.as_str())?;
        let config = crate::style::SoulDriftConfig::default();
        Ok(DriftResponse {
            char_count: content.chars().count(),
            content,
            capacity: config.capacity_chars,
        })
    })();
    match result {
        Ok(resp) => match serde_json::to_value(resp) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => AirpError::from(e).into_response(),
        },
        Err(e) => e.into_response(),
    }
}

/// PUT /v1/characters/:character_id/drift
pub async fn update_drift(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Json(payload): Json<UpdateDriftRequest>,
) -> impl IntoResponse {
    let result = (|| -> Result<(), AirpError> {
        let cid = CharacterId::new(&character_id)?;
        crate::style::write_soul_drift(&state.data_root, cid.as_str(), &payload.content)
    })();
    match result {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response(),
        Err(e) => e.into_response(),
    }
}
