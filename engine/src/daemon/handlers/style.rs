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
    pub revision: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDriftRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct RollbackDriftRequest {
    pub revision: u64,
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
        provider_config.clone(),
        gen_params.clone(),
        &style_profile,
        &recent_messages,
        &current_drift,
    )
    .await?;

    let mut drift_applied = false;
    if !report.drift_patch.trim().is_empty() {
        crate::style::append_soul_drift_with_compression(
            &state.http_client,
            provider_config,
            gen_params,
            &state.data_root,
            cid.as_str(),
            &report.drift_patch,
        )
        .await?;
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
        let (content, revision) =
            crate::style::read_soul_drift_with_revision(&state.data_root, cid.as_str())?;
        let config = crate::style::SoulDriftConfig::default();
        Ok(DriftResponse {
            char_count: content.chars().count(),
            content,
            capacity: config.capacity_chars,
            revision,
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
    let result = (|| -> Result<u64, AirpError> {
        let cid = CharacterId::new(&character_id)?;
        crate::style::write_soul_drift(&state.data_root, cid.as_str(), &payload.content)
    })();
    match result {
        Ok(revision) => (
            StatusCode::OK,
            Json(serde_json::json!({ "success": true, "revision": revision })),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// POST /v1/characters/:character_id/drift/rollback
pub async fn rollback_drift(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Json(payload): Json<RollbackDriftRequest>,
) -> impl IntoResponse {
    let result = (|| -> Result<u64, AirpError> {
        let cid = CharacterId::new(&character_id)?;
        crate::style::rollback_soul_drift(&state.data_root, cid.as_str(), payload.revision)
    })();
    match result {
        Ok(revision) => (
            StatusCode::OK,
            Json(serde_json::json!({ "success": true, "revision": revision })),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// Phase 4.2: `POST /v1/style/learn` — 从文本样本提取风格特征写入 profile。
///
/// 请求体：`{ text, profile_id?, character_id? }`
/// 行为：
/// 1. 校验 profile_id 防路径遍历
/// 2. 调用 LLM 提取 6 维度风格特征
/// 3. 渲染为 markdown profile，原子写入 `styles/profiles/{profile_id}.md`
/// 4. 若提供 character_id，额外写入 `characters/{cid}/style-profile.md`
pub async fn style_learn(
    State(state): State<Arc<DaemonState>>,
    Json(payload): Json<crate::style::StyleLearnRequest>,
) -> impl IntoResponse {
    match run_style_learn_handler(&state, payload).await {
        Ok(resp) => match serde_json::to_value(resp) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => AirpError::from(e).into_response(),
        },
        Err(e) => e.into_response(),
    }
}

async fn run_style_learn_handler(
    state: &DaemonState,
    payload: crate::style::StyleLearnRequest,
) -> Result<crate::style::StyleLearnResponse, AirpError> {
    // 1. 校验 profile_id
    crate::style::validate_profile_id(&payload.profile_id)?;

    // 2. 校验 character_id（若提供）
    if let Some(cid) = payload.character_id.as_deref() {
        let _ = CharacterId::new(cid)?;
    }

    // 3. 构造 provider config + gen params
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
        temperature: Some(0.3),
        max_tokens: Some(800),
    };

    // 4. 调用 LLM 提取风格特征
    let features = crate::style::run_style_learn(
        &state.http_client,
        provider_config,
        gen_params,
        &payload.text,
    )
    .await?;

    // 5. 渲染 markdown profile
    let profile_md = crate::style::render_profile_markdown(&features, &payload.profile_id);
    let count = crate::style::features_count(&features);

    // 6. 写入全局 profile
    let global_path = crate::style::global_profile_path(&state.data_root, &payload.profile_id);
    let relative_path = crate::style::write_profile(&state.data_root, &global_path, &profile_md)?;

    // 7. 若提供 character_id，额外写入角色专属 profile
    if let Some(cid) = payload.character_id.as_deref() {
        let char_path = crate::style::character_profile_path(&state.data_root, cid);
        // 角色专属 profile 只保留 feature 条目部分，去掉全局 profile_md 的
        // `# Style Profile: {profile_id}` 头部和 `来源：...（{timestamp}）` 行，
        // 改用角色专属头部。否则会出现两层 # 头部和两个来源时间戳，污染 prompt。
        let entries = profile_md
            .find("\n- ")
            .map(|i| &profile_md[i + 1..])
            .unwrap_or(profile_md.as_str());
        let char_md = {
            let now = chrono::Utc::now().to_rfc3339();
            let mut md = String::with_capacity(512);
            md.push_str(&format!("# Character Style Profile: {}\n\n", cid));
            md.push_str(&format!("来源：用户文本样本学习（{}）\n\n", now));
            md.push_str(entries);
            md
        };
        // R1: propagate write failure — requests with character_id must not report
        // success when the character profile was not persisted. 与全局 profile
        // write_profile(? operator above) 的错误传播纪律对齐。
        crate::style::write_profile(&state.data_root, &char_path, &char_md)?;
    }

    Ok(crate::style::StyleLearnResponse {
        success: true,
        profile_path: relative_path,
        features_count: count,
        profile_content: profile_md,
    })
}

/// Phase 4.2: `GET /v1/style/profiles/:profile_id` — 读取已学习的风格 profile。
pub async fn get_style_profile(
    State(state): State<Arc<DaemonState>>,
    Path(profile_id): Path<String>,
) -> impl IntoResponse {
    let result = (|| -> Result<serde_json::Value, AirpError> {
        crate::style::validate_profile_id(&profile_id)?;
        let path = crate::style::global_profile_path(&state.data_root, &profile_id);
        let content = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AirpError::NotFound(format!("style profile '{}' not found", profile_id))
            } else {
                AirpError::from(e)
            }
        })?;
        Ok(serde_json::json!({
            "profile_id": profile_id,
            "content": content,
            "char_count": content.chars().count(),
        }))
    })();
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Phase 4.2: `GET /v1/style/profiles` — 列出所有已学习的风格 profile。
pub async fn list_style_profiles(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let result = (|| -> Result<Vec<serde_json::Value>, AirpError> {
        let profiles_dir = state.data_root.join("styles").join("profiles");
        if !profiles_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&profiles_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let metadata = entry.metadata()?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            entries.push(serde_json::json!({
                "profile_id": stem,
                "size_bytes": metadata.len(),
                "modified_timestamp": modified,
            }));
        }
        entries.sort_by(|a, b| {
            b.get("profile_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(a.get("profile_id").and_then(|v| v.as_str()).unwrap_or(""))
        });
        Ok(entries)
    })();
    match result {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => e.into_response(),
    }
}
