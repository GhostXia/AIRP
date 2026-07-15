//! Settings HTTP handlers — read and update daemon runtime config.
//!
//! #155 PR5：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 config orchestration；`SettingsView` 脱敏和 `PartialAppConfig` 合并在 `config` 模块。
//!
//! 端点：
//! - `GET  /v1/settings` — 返回当前运行时配置（api_key 脱敏）
//! - `POST /v1/settings` — 用 `PartialAppConfig` 合并 + 落盘 `settings.json`

use crate::daemon::DaemonState;
use crate::daemon::SettingsView;
use crate::error::AirpError;
use axum::Json;
use std::fs;
use std::sync::Arc;

/// GET /v1/settings — 返回当前 daemon 运行时配置（api_key 脱敏）。
pub(in crate::daemon) async fn get_settings(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<SettingsView>, AirpError> {
    let cfg = state
        .config
        .read()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
    Ok(Json(SettingsView::from_config(&cfg, &state.data_root)))
}

/// POST /v1/settings — 用 `PartialAppConfig` 合并到当前运行时配置 + 落盘
/// `data/settings.json`。空字符串视为未设置，避免抹掉合法上层值。
pub(in crate::daemon) async fn update_settings(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(patch): Json<crate::config::PartialAppConfig>,
) -> Result<Json<SettingsView>, AirpError> {
    // 1) Production access-key 门控：只读锁，不修改任何字段，保证拒绝时不留下
    //    部分更新（与 #165 SET-01 的原子性边界一致）。
    if patch
        .access_api_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
    {
        let cfg = state
            .config
            .read()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        if cfg.deployment_mode == crate::config::DeploymentMode::Production {
            return Err(AirpError::BadRequest(
                "AIRP_ACCESS_KEY cannot be changed through /v1/settings in production; rotate the gateway and engine secret together, then restart"
                    .to_string(),
            ));
        }
    }

    // 2) 任何可失败的 patch 校验都必须在拿写锁前完成。否则无效 patch 会先留下
    //    provider/endpoint/api_key/model 等字段的内存更新，而 `settings.json`
    //    因 `?` 提前返回不会落盘，形成内存/磁盘不一致（#165 SET-01）。
    //    当前唯一可失败的可观察校验是 `volume.validate()`；其它字段在反序列化
    //    时已完成类型校验，应用阶段是确定性无失败路径。
    if let Some(v) = patch.volume.as_ref() {
        v.validate()
            .map_err(|e| AirpError::BadRequest(format!("VolumeConfig 不合法: {}", e)))?;
    }

    // 3) 合并到内存：写锁内只做确定性字段替换，不再有 `?` 失败路径。
    let merged = {
        let mut cfg = state
            .config
            .write()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        if let Some(p) = patch.provider {
            cfg.provider = p;
        }
        if let Some(e) = patch.endpoint.filter(|s| !s.is_empty()) {
            cfg.endpoint = e;
        }
        if let Some(k) = patch.api_key.filter(|s| !s.is_empty()) {
            cfg.api_key = Some(k);
        }
        if let Some(m) = patch.model.filter(|s| !s.is_empty()) {
            cfg.model = m;
        }
        // volume 已在步骤 2 校验；此处只做确定性赋值，不再重复 validate。
        if let Some(v) = patch.volume {
            cfg.volume_config = v;
        }
        if let Some(k) = patch.access_api_key.filter(|s| !s.is_empty()) {
            cfg.access_api_key = Some(k);
        }
        if let Some(e) = patch.engine {
            cfg.engine = e;
        }
        if let Some(q) = patch.quota {
            cfg.quota = q;
        }
        cfg.clone()
    };

    // 4) Persist only non-secret settings. Provider and daemon credentials are
    // runtime-only and must be supplied through AIRP_* env or this request.
    let on_disk = serde_json::json!({
        "provider": merged.provider,
        "endpoint": merged.endpoint,
        "model": merged.model,
        "volume": merged.volume_config,
        "engine": merged.engine,
        "quota": merged.quota,
    });
    let path = state.data_root.join("settings.json");
    fs::write(&path, serde_json::to_string_pretty(&on_disk)?)?;

    Ok(Json(SettingsView::from_config(&merged, &state.data_root)))
}
