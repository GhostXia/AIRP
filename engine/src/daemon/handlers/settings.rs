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
    // 1) 任何可失败的 patch 校验都必须在拿写锁前完成。否则无效 patch 会先留下
    //    provider/endpoint/api_key/model 等字段的内存更新，而 `settings.json`
    //    因 `?` 提前返回不会落盘，形成内存/磁盘不一致（#165 SET-01）。
    //    当前唯一可失败的可观察校验是 `volume.validate()`；其它字段在反序列化
    //    时已完成类型校验，应用阶段是确定性无失败路径。
    if let Some(v) = patch.volume.as_ref() {
        v.validate()
            .map_err(|e| AirpError::BadRequest(format!("VolumeConfig 不合法: {}", e)))?;
    }

    // 2) 专用异步锁是 settings update 的单进程事务边界。它串行 candidate
    //    构造、持久化和 live commit，同时不在磁盘 I/O 期间阻塞其他 config readers。
    let _transaction = state.settings_update.transaction.lock().await;
    let mut candidate = state
        .config
        .read()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?
        .clone();
    if patch
        .access_api_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
        && candidate.deployment_mode == crate::config::DeploymentMode::Production
    {
        return Err(AirpError::BadRequest(
            "AIRP_ACCESS_KEY cannot be changed through /v1/settings in production; rotate the gateway and engine secret together, then restart"
                .to_string(),
        ));
    }

    if let Some(p) = patch.provider {
        candidate.provider = p;
    }
    if let Some(e) = patch.endpoint.filter(|s| !s.is_empty()) {
        candidate.endpoint = e;
    }
    let provider_key_update = patch.api_key.filter(|key| !key.is_empty());
    if let Some(key) = provider_key_update.as_ref() {
        candidate.api_key = Some(key.clone());
    }
    if let Some(m) = patch.model.filter(|s| !s.is_empty()) {
        candidate.model = m;
    }
    if let Some(v) = patch.volume {
        candidate.volume_config = v;
    }
    if let Some(k) = patch.access_api_key.filter(|s| !s.is_empty()) {
        candidate.access_api_key = Some(k);
    }
    if let Some(e) = patch.engine {
        candidate.engine = e;
    }
    if let Some(q) = patch.quota {
        candidate.quota = q;
    }

    // 3) settings.json remains non-secret. The portable launcher may opt into
    // one separate data/secrets.json provider-key file; access keys are always
    // runtime-only.
    let on_disk = serde_json::json!({
        "provider": candidate.provider,
        "endpoint": candidate.endpoint,
        "model": candidate.model,
        "volume": candidate.volume_config,
        "engine": candidate.engine,
        "quota": candidate.quota,
    });
    let path = state.data_root.join("settings.json");
    let data_root = state.data_root.clone();
    let bytes = serde_json::to_vec_pretty(&on_disk)?;
    #[cfg(test)]
    let persist_state = state.clone();
    tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        if let Some(result) = persist_state
            .settings_update
            .run_persistence_override(&path, &bytes)
        {
            return result;
        }
        let previous_settings = std::fs::read(&path).ok();
        crate::data_dir::replace_file(&path, &bytes)?;
        if let Some(key) = provider_key_update.as_deref() {
            if let Err(secret_error) = crate::secret_store::persist_provider_key(&data_root, key) {
                let rollback = match previous_settings {
                    Some(previous) => crate::data_dir::replace_file(&path, &previous),
                    None => std::fs::remove_file(&path).map_err(AirpError::Io),
                };
                return match rollback {
                    Ok(()) => Err(secret_error),
                    Err(rollback_error) => Err(AirpError::Internal(format!(
                        "provider key persistence failed ({secret_error}); settings rollback also failed ({rollback_error})"
                    ))),
                };
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| AirpError::Internal(format!("settings persistence task failed: {error}")))??;

    let mut cfg = state
        .config
        .write()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
    *cfg = candidate;
    Ok(Json(SettingsView::from_config(&cfg, &state.data_root)))
}
