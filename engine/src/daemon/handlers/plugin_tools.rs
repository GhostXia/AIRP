//! Phase 5.3: Plugin / Custom Agent Tools HTTP handlers.
//!
//! 端点：
//! - `GET    /v1/plugin-tools` — 列出所有插件工具（headers 值脱敏，仅返回 `headers_set` 与 `headers_keys`）
//! - `POST   /v1/plugin-tools` — 注册或更新单个插件工具（upsert 语义，按 `name` 替换）
//!   body limit 2MB；webhook headers 中的密钥写入 `data/plugin_tool_headers.json`，
//!   不写入 `data/plugin_tools.json`。
//! - `DELETE /v1/plugin-tools/:name` — 删除指定 name 的插件工具
//! - `POST   /v1/plugin-tools/:name/test` — 测试调用插件工具（dry-run；params 可选）
//!
//! 所有写操作都是 atomic：先校验 → 持久化到磁盘 → 提交到内存 `RwLock<Vec<PluginToolConfig>>`。
//! 持久化失败时不会污染内存状态。

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::plugin_tool::{
    save_plugin_tools, PluginInvocation, PluginSideEffect, PluginToolConfig, DNS_BUDGET_SECS,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;

// ── 视图模型（脱敏） ─────────────────────────────────────────────────────────

/// `GET /v1/plugin-tools` 返回的单条插件工具（webhook headers 脱敏）。
#[derive(Debug, Serialize)]
pub struct PluginToolView {
    pub name: String,
    pub description: String,
    pub side_effect: PluginSideEffect,
    pub enabled: bool,
    /// 调用方式（webhook 的 headers 值被剥离，仅保留 `headers_set` 与 `headers_keys`）。
    pub invocation: PluginInvocationView,
}

/// `PluginInvocation` 的脱敏视图。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginInvocationView {
    Webhook {
        url: String,
        /// 是否设置了自定义 headers（不返回 headers 本体）。
        headers_set: bool,
        /// 已设置的 header 名，按字典序排列（不返回任何 value）。
        headers_keys: Vec<String>,
        timeout_secs: Option<u32>,
    },
    Script {
        relative_path: String,
        args: Vec<String>,
        timeout_secs: Option<u32>,
    },
}

impl PluginToolView {
    fn from_config(config: &PluginToolConfig) -> Self {
        let invocation = match &config.invocation {
            PluginInvocation::Webhook {
                url,
                headers,
                timeout_secs,
            } => {
                let mut headers_keys: Vec<String> = headers.keys().cloned().collect();
                headers_keys.sort();
                PluginInvocationView::Webhook {
                    url: url.clone(),
                    headers_set: !headers.is_empty(),
                    headers_keys,
                    timeout_secs: *timeout_secs,
                }
            }
            PluginInvocation::Script {
                relative_path,
                args,
                timeout_secs,
            } => PluginInvocationView::Script {
                relative_path: relative_path.clone(),
                args: args.clone(),
                timeout_secs: *timeout_secs,
            },
        };
        Self {
            name: config.name.clone(),
            description: config.description.clone(),
            side_effect: config.side_effect,
            enabled: config.enabled,
            invocation,
        }
    }
}

/// `GET /v1/plugin-tools` 返回体。
#[derive(Debug, Serialize)]
pub struct PluginToolsResponse {
    pub tools: Vec<PluginToolView>,
    pub total: usize,
    pub enabled: usize,
}

// ── 请求模型 ─────────────────────────────────────────────────────────────────

/// `POST /v1/plugin-tools` 请求体（upsert 语义）。
///
/// `headers` 字段可选；更新既有 webhook 时缺省表示保留原值，显式 Map（包括空 Map）按请求替换。
/// `headers` 不会持久化到 `data/plugin_tools.json`，由
/// `data/plugin_tool_headers.json` 单独存储。
#[derive(Debug, Deserialize)]
pub struct PluginToolUpsert {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub side_effect: PluginSideEffect,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub invocation: PluginInvocationUpsert,
}

fn default_enabled() -> bool {
    true
}

/// upsert 请求中的 invocation（与 `PluginInvocation` 类似）。webhook 的
/// `headers: None` 表示请求体缺省该字段，`Some` 表示显式提供 Map。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginInvocationUpsert {
    Webhook {
        url: String,
        #[serde(default)]
        headers: Option<BTreeMap<String, String>>,
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
    Script {
        relative_path: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
}

impl PluginToolUpsert {
    /// 转换为 `PluginToolConfig`，并返回 webhook headers 是否由请求显式提供。
    /// 校验由 `PluginToolConfig::validate` 完成（在 `save_plugin_tools` 内）。
    fn into_config(self) -> (PluginToolConfig, bool) {
        let (invocation, headers_provided) = match self.invocation {
            PluginInvocationUpsert::Webhook {
                url,
                headers,
                timeout_secs,
            } => {
                let headers_provided = headers.is_some();
                (
                    PluginInvocation::Webhook {
                        url,
                        headers: headers.unwrap_or_default(),
                        timeout_secs,
                    },
                    headers_provided,
                )
            }
            PluginInvocationUpsert::Script {
                relative_path,
                args,
                timeout_secs,
            } => (
                PluginInvocation::Script {
                    relative_path,
                    args,
                    timeout_secs,
                },
                false,
            ),
        };
        (
            PluginToolConfig {
                name: self.name,
                description: self.description,
                side_effect: self.side_effect,
                enabled: self.enabled,
                invocation,
            },
            headers_provided,
        )
    }
}

/// `save_plugin_tools` 保留历史上的空 headers 语义；请求显式提供空 Map 时，
/// 在该原子保存完成后移除目标工具的独立 headers 记录，使请求语义仍是替换为空。
fn clear_persisted_plugin_tool_headers(
    data_root: &FsPath,
    tool_name: &str,
) -> Result<(), AirpError> {
    let mut headers = crate::plugin_tool::load_plugin_tool_headers(data_root)?;
    if headers.remove(tool_name).is_none() {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "headers": headers,
    }))?;
    let path = crate::plugin_tool::plugin_tool_headers_file_path(data_root);
    crate::data_dir::replace_file(&path, &bytes)?;
    // `replace_file` creates a new inode; retain the credential-file permission hardening.
    #[cfg(unix)]
    if let Ok(mut permissions) = std::fs::metadata(&path).map(|metadata| metadata.permissions()) {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(&path, permissions);
    }
    Ok(())
}

/// `POST /v1/plugin-tools/:name/test` 请求体。
#[derive(Debug, Deserialize, Default)]
pub struct PluginToolTestRequest {
    /// 测试参数；缺省为 `{}`。
    #[serde(default)]
    pub params: serde_json::Value,
    /// 是否模拟破坏性确认。缺省 false（dry-run）。
    #[serde(default)]
    pub confirm: bool,
}

/// `POST /v1/plugin-tools/:name/test` 返回体。
#[derive(Debug, Serialize)]
pub struct PluginToolTestResponse {
    pub name: String,
    pub output: serde_json::Value,
    pub dry_run: bool,
}

// ── handlers ────────────────────────────────────────────────────────────────

/// `GET /v1/plugin-tools` — 列出所有插件工具（headers 脱敏）。
pub(in crate::daemon) async fn list_plugin_tools_endpoint(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<PluginToolsResponse>, AirpError> {
    let tools = state
        .plugin_tools
        .read()
        .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
    let views: Vec<PluginToolView> = tools.iter().map(PluginToolView::from_config).collect();
    let total = views.len();
    let enabled = views.iter().filter(|v| v.enabled).count();
    Ok(Json(PluginToolsResponse {
        tools: views,
        total,
        enabled,
    }))
}

/// `POST /v1/plugin-tools` — 注册或更新单个插件工具（upsert 语义）。
///
/// 流程：
/// 1. 反序列化 + 转换为 `PluginToolConfig`。
/// 2. 校验（`PluginToolConfig::validate`）。
/// 3. 在 `plugin_tools` 内存列表中 upsert（按 name 替换或追加）。
/// 4. 持久化到 `data/plugin_tools.json` + `data/plugin_tool_headers.json`（atomic）。
///
/// 持久化失败时不会污染内存（先校验和持久化，再提交到内存）。
///
/// **Major1, 2026-07-26**：通过 `plugin_tools_update` 协调器串行化
/// read-persist-commit，避免并发 upsert / delete 造成盘-内存不一致。
pub(in crate::daemon) async fn upsert_plugin_tool_endpoint(
    State(state): State<Arc<DaemonState>>,
    Json(upsert): Json<PluginToolUpsert>,
) -> Result<(StatusCode, Json<PluginToolView>), AirpError> {
    let (mut config, headers_provided) = upsert.into_config();
    // 校验单条配置（在持久化前）。
    // N2（PR #384 审计）：validate 含同步 DNS 解析与文件 canonicalize，
    // 必须在 spawn_blocking 中执行以免阻塞 tokio 异步运行时。
    // CR-new（CodeRabbit 2026-07-31 复审）：spawn_blocking 不受超时约束，
    // 显式套 DNS_BUDGET_SECS timeout 防止挂起；超时归 Internal（环境问题）。
    let data_root_for_validate = state.data_root.clone();
    let config_for_validate = config.clone();
    tokio::time::timeout(
        Duration::from_secs(DNS_BUDGET_SECS as u64),
        tokio::task::spawn_blocking(move || config_for_validate.validate(&data_root_for_validate)),
    )
    .await
    .map_err(|_| AirpError::Internal(format!("plugin_tools 校验超时 ({}s)", DNS_BUDGET_SECS)))?
    .map_err(|e| AirpError::Internal(format!("plugin_tools 校验任务失败: {e}")))?
    .map_err(|e| AirpError::BadRequest(format!("插件工具配置不合法: {e}")))?;
    // Minor2: 与内建工具命名空间冲突检查由 validate_tool_name 完成（前缀保护）。

    // Major1: 串行化 read-persist-commit。
    let _lock = state.plugin_tools_update.lock().await;

    // 读取当前列表，upsert（按 name 替换或追加）。
    let mut new_tools: Vec<PluginToolConfig> = {
        let tools = state
            .plugin_tools
            .read()
            .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
        tools.clone()
    };
    // 前端无法回填密钥；请求缺省 headers 时沿用当前 webhook 的内存值。
    // 显式空 Map 不进入此分支，表示用户要求替换为空。
    if !headers_provided {
        if let Some(existing) = new_tools.iter().find(|tool| tool.name == config.name) {
            if let (
                PluginInvocation::Webhook {
                    headers: existing_headers,
                    ..
                },
                PluginInvocation::Webhook { headers, .. },
            ) = (&existing.invocation, &mut config.invocation)
            {
                *headers = existing_headers.clone();
            }
        }
    }
    let clear_headers_for = if headers_provided
        && matches!(
            &config.invocation,
            PluginInvocation::Webhook { headers, .. } if headers.is_empty()
        ) {
        Some(config.name.clone())
    } else {
        None
    };
    if let Some(existing) = new_tools.iter_mut().find(|t| t.name == config.name) {
        *existing = config.clone();
    } else {
        new_tools.push(config.clone());
    }

    // 持久化（spawn_blocking 避免 tokio I/O 阻塞）。
    let data_root = state.data_root.clone();
    let tools_clone = new_tools.clone();
    tokio::task::spawn_blocking(move || {
        save_plugin_tools(&data_root, &tools_clone)?;
        if let Some(tool_name) = clear_headers_for {
            clear_persisted_plugin_tool_headers(&data_root, &tool_name)?;
        }
        Ok::<(), AirpError>(())
    })
    .await
    .map_err(|e| AirpError::Internal(format!("plugin_tools 持久化任务失败: {e}")))??;

    // 提交到内存。
    let mut tools_lock = state
        .plugin_tools
        .write()
        .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
    *tools_lock = new_tools;

    let view = PluginToolView::from_config(&config);
    Ok((StatusCode::OK, Json(view)))
}

/// `DELETE /v1/plugin-tools/:name` — 删除指定 name 的插件工具。
///
/// 不存在时返回 404。
///
/// **Major1, 2026-07-26**：与 upsert 共享 `plugin_tools_update` 协调器，
/// 串行化 read-persist-commit。
pub(in crate::daemon) async fn delete_plugin_tool_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AirpError> {
    // Major1: 串行化 read-persist-commit。
    let _lock = state.plugin_tools_update.lock().await;

    let mut new_tools: Vec<PluginToolConfig> = {
        let tools = state
            .plugin_tools
            .read()
            .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
        tools.clone()
    };
    let before = new_tools.len();
    new_tools.retain(|t| t.name != name);
    if new_tools.len() == before {
        return Err(AirpError::NotFound(format!("插件工具 '{}' 不存在", name)));
    }

    // 持久化。
    let data_root = state.data_root.clone();
    let tools_clone = new_tools.clone();
    tokio::task::spawn_blocking(move || save_plugin_tools(&data_root, &tools_clone))
        .await
        .map_err(|e| AirpError::Internal(format!("plugin_tools 持久化任务失败: {e}")))??;

    let mut tools_lock = state
        .plugin_tools
        .write()
        .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
    *tools_lock = new_tools;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/plugin-tools/:name/test` — 测试调用插件工具（dry-run）。
///
/// 流程：
/// 1. 从 `plugin_tools` 内存列表查找 name。
/// 2. 构造 `PluginTool` 并调用 `Tool::call(params, confirm)`。
/// 3. 返回 output。
///
/// 注意：此端点直接调用插件工具，可能产生真实副作用（HTTP 调用/脚本执行）。
/// 调用方应明确知道 `confirm=false` 仅是建议而非强制 dry-run。
pub(in crate::daemon) async fn test_plugin_tool_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginToolTestRequest>,
) -> Result<Json<PluginToolTestResponse>, AirpError> {
    let config = {
        let tools = state
            .plugin_tools
            .read()
            .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
        tools
            .iter()
            .find(|t| t.name == name)
            .cloned()
            .ok_or_else(|| AirpError::NotFound(format!("插件工具 '{}' 不存在", name)))?
    };

    let tool = crate::plugin_tool::PluginTool::new(
        config.clone(),
        state.http_client.clone(),
        state.data_root.clone(),
    );
    let result = crate::agent::tools::Tool::call(&tool, req.params, req.confirm)
        .await
        .map_err(|e| AirpError::Internal(format!("插件工具 '{}' 测试调用失败: {}", name, e)))?;

    Ok(Json(PluginToolTestResponse {
        name: config.name,
        output: result.output,
        dry_run: result.dry_run,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::tests::make_state_no_key as make_state_for_http_test;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    async fn post_plugin_tool(
        app: axum::Router,
        body: serde_json::Value,
    ) -> (StatusCode, String, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/plugin-tools")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, raw, value)
    }

    async fn get_plugin_tools(app: axum::Router) -> (StatusCode, String, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/plugin-tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, raw, value)
    }

    #[test]
    fn webhook_view_serializes_sorted_header_names_without_values() {
        let config = PluginToolConfig {
            name: "header_probe".to_string(),
            description: "header probe".to_string(),
            side_effect: PluginSideEffect::Readonly,
            enabled: true,
            invocation: PluginInvocation::Webhook {
                url: "https://example.test/hook".to_string(),
                headers: BTreeMap::from([
                    ("z-last".to_string(), "secret-z".to_string()),
                    ("A-first".to_string(), "secret-a".to_string()),
                ]),
                timeout_secs: None,
            },
        };

        let serialized = serde_json::to_string(&PluginToolView::from_config(&config)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        let invocation = &value["invocation"];
        assert_eq!(invocation["headers_set"], serde_json::json!(true));
        assert_eq!(
            invocation["headers_keys"],
            serde_json::json!(["A-first", "z-last"])
        );
        assert!(!serialized.contains("secret-a"));
        assert!(!serialized.contains("secret-z"));
        assert!(!serialized.contains("\"headers\""));
    }

    #[tokio::test]
    async fn post_webhook_preserves_omitted_headers_in_memory_and_persistence() {
        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let (status, raw, view) = post_plugin_tool(
            app.clone(),
            serde_json::json!({
                "name": "signed_hook",
                "description": "signed webhook",
                "side_effect": "readonly",
                "enabled": true,
                "invocation": {
                    "kind": "webhook",
                    "url": "https://1.1.1.1/hook",
                    "headers": {
                        "Authorization": "previous-auth-token",
                        "X-Request-ID": "previous-request-id"
                    }
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            view["invocation"]["headers_keys"],
            serde_json::json!(["Authorization", "X-Request-ID"])
        );
        assert!(view["invocation"].get("headers").is_none());
        assert!(!raw.contains("previous-auth-token"));
        assert!(!raw.contains("previous-request-id"));

        let (status, raw, _) = post_plugin_tool(
            app.clone(),
            serde_json::json!({
                "name": "signed_hook",
                "description": "edited webhook",
                "side_effect": "readonly",
                "enabled": true,
                "invocation": {
                    "kind": "webhook",
                    "url": "https://1.1.1.1/hook"
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!raw.contains("previous-auth-token"));
        assert!(!raw.contains("previous-request-id"));

        let in_memory = state
            .plugin_tools
            .read()
            .unwrap()
            .iter()
            .find(|tool| tool.name == "signed_hook")
            .unwrap()
            .clone();
        if let PluginInvocation::Webhook { headers, .. } = &in_memory.invocation {
            assert_eq!(
                headers.get("Authorization").map(String::as_str),
                Some("previous-auth-token")
            );
            assert_eq!(
                headers.get("X-Request-ID").map(String::as_str),
                Some("previous-request-id")
            );
        } else {
            panic!("expected webhook invocation");
        }

        let restored = crate::plugin_tool::load_plugin_tools(&state.data_root).unwrap();
        let restored = restored
            .iter()
            .find(|tool| tool.name == "signed_hook")
            .unwrap();
        if let PluginInvocation::Webhook { headers, .. } = &restored.invocation {
            assert_eq!(
                headers.get("Authorization").map(String::as_str),
                Some("previous-auth-token")
            );
            assert_eq!(
                headers.get("X-Request-ID").map(String::as_str),
                Some("previous-request-id")
            );
        } else {
            panic!("expected restored webhook invocation");
        }

        let (status, raw, view) = get_plugin_tools(app.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            view["tools"][0]["invocation"]["headers_keys"],
            serde_json::json!(["Authorization", "X-Request-ID"])
        );
        assert!(view["tools"][0]["invocation"].get("headers").is_none());
        assert!(!raw.contains("previous-auth-token"));
        assert!(!raw.contains("previous-request-id"));

        let (status, raw, _) = post_plugin_tool(
            app.clone(),
            serde_json::json!({
                "name": "signed_hook",
                "description": "replaced webhook",
                "side_effect": "readonly",
                "enabled": true,
                "invocation": {
                    "kind": "webhook",
                    "url": "https://1.1.1.1/hook",
                    "headers": {
                        "Authorization": "replacement-auth-token",
                        "X-New": "replacement-request-id"
                    }
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!raw.contains("replacement-auth-token"));
        assert!(!raw.contains("replacement-request-id"));

        let replaced = state
            .plugin_tools
            .read()
            .unwrap()
            .iter()
            .find(|tool| tool.name == "signed_hook")
            .unwrap()
            .clone();
        if let PluginInvocation::Webhook { headers, .. } = &replaced.invocation {
            assert_eq!(
                headers.get("Authorization").map(String::as_str),
                Some("replacement-auth-token")
            );
            assert_eq!(
                headers.get("X-New").map(String::as_str),
                Some("replacement-request-id")
            );
            assert!(!headers.contains_key("X-Request-ID"));
        } else {
            panic!("expected webhook invocation");
        }

        let (status, raw, view) = post_plugin_tool(
            app.clone(),
            serde_json::json!({
                "name": "signed_hook",
                "description": "cleared webhook",
                "side_effect": "readonly",
                "enabled": true,
                "invocation": {
                    "kind": "webhook",
                    "url": "https://1.1.1.1/hook",
                    "headers": {}
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["invocation"]["headers_set"], serde_json::json!(false));
        assert!(!raw.contains("replacement-auth-token"));
        assert!(!raw.contains("replacement-request-id"));
        let live_cleared = state
            .plugin_tools
            .read()
            .unwrap()
            .iter()
            .find(|tool| tool.name == "signed_hook")
            .unwrap()
            .clone();
        if let PluginInvocation::Webhook { headers, .. } = live_cleared.invocation {
            assert!(headers.is_empty());
        } else {
            panic!("expected cleared live webhook invocation");
        }
        let cleared = crate::plugin_tool::load_plugin_tools(&state.data_root)
            .unwrap()
            .into_iter()
            .find(|tool| tool.name == "signed_hook")
            .unwrap();
        if let PluginInvocation::Webhook { headers, .. } = cleared.invocation {
            assert!(headers.is_empty());
        } else {
            panic!("expected cleared webhook invocation");
        }

        let (status, _, _) = post_plugin_tool(
            app,
            serde_json::json!({
                "name": "empty_hook",
                "description": "empty webhook",
                "side_effect": "readonly",
                "enabled": true,
                "invocation": {
                    "kind": "webhook",
                    "url": "https://1.1.1.1/hook"
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let tools = state.plugin_tools.read().unwrap();
        let created = tools.iter().find(|tool| tool.name == "empty_hook").unwrap();
        if let PluginInvocation::Webhook { headers, .. } = &created.invocation {
            assert!(headers.is_empty());
        } else {
            panic!("expected created webhook invocation");
        }
        let restored_created = crate::plugin_tool::load_plugin_tools(&state.data_root)
            .unwrap()
            .into_iter()
            .find(|tool| tool.name == "empty_hook")
            .unwrap();
        if let PluginInvocation::Webhook { headers, .. } = restored_created.invocation {
            assert!(headers.is_empty());
        } else {
            panic!("expected restored created webhook invocation");
        }
    }
}
