//! Phase 5.3: Plugin / Custom Agent Tools HTTP handlers.
//!
//! 端点：
//! - `GET    /v1/plugin-tools` — 列出所有插件工具（headers 字段脱敏为 `headers_set: bool`）
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
use crate::plugin_tool::{save_plugin_tools, PluginInvocation, PluginSideEffect, PluginToolConfig};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

// ── 视图模型（脱敏） ─────────────────────────────────────────────────────────

/// `GET /v1/plugin-tools` 返回的单条插件工具（webhook headers 脱敏）。
#[derive(Debug, Serialize)]
pub struct PluginToolView {
    pub name: String,
    pub description: String,
    pub side_effect: PluginSideEffect,
    pub enabled: bool,
    /// 调用方式（webhook 的 headers 字段被剥离，仅保留 `headers_set: bool`）。
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
            } => PluginInvocationView::Webhook {
                url: url.clone(),
                headers_set: !headers.is_empty(),
                timeout_secs: *timeout_secs,
            },
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
/// `headers` 字段可选；为空 Map 或缺省时视为未设置。
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

/// upsert 请求中的 invocation（与 `PluginInvocation` 类似，但 webhook 的
/// headers 是必填字段——因为这是请求体而非磁盘 schema）。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginInvocationUpsert {
    Webhook {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
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
    /// 转换为 `PluginToolConfig`，并返回提取出的 webhook headers（如有）。
    /// 校验由 `PluginToolConfig::validate` 完成（在 `save_plugin_tools` 内）。
    fn into_config(self) -> PluginToolConfig {
        let invocation = match self.invocation {
            PluginInvocationUpsert::Webhook {
                url,
                headers,
                timeout_secs,
            } => PluginInvocation::Webhook {
                url,
                headers,
                timeout_secs,
            },
            PluginInvocationUpsert::Script {
                relative_path,
                args,
                timeout_secs,
            } => PluginInvocation::Script {
                relative_path,
                args,
                timeout_secs,
            },
        };
        PluginToolConfig {
            name: self.name,
            description: self.description,
            side_effect: self.side_effect,
            enabled: self.enabled,
            invocation,
        }
    }
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
pub(in crate::daemon) async fn upsert_plugin_tool_endpoint(
    State(state): State<Arc<DaemonState>>,
    Json(upsert): Json<PluginToolUpsert>,
) -> Result<(StatusCode, Json<PluginToolView>), AirpError> {
    let config = upsert.into_config();
    // 校验单条配置（在持久化前）。
    config
        .validate(&state.data_root)
        .map_err(|e| AirpError::BadRequest(format!("插件工具配置不合法: {e}")))?;

    // 读取当前列表，upsert（按 name 替换或追加）。
    let mut new_tools: Vec<PluginToolConfig> = {
        let tools = state
            .plugin_tools
            .read()
            .map_err(|_| AirpError::Internal("plugin_tools lock poisoned".to_string()))?;
        tools.clone()
    };
    if let Some(existing) = new_tools.iter_mut().find(|t| t.name == config.name) {
        *existing = config.clone();
    } else {
        new_tools.push(config.clone());
    }

    // 持久化（spawn_blocking 避免 tokio I/O 阻塞）。
    let data_root = state.data_root.clone();
    let tools_clone = new_tools.clone();
    tokio::task::spawn_blocking(move || save_plugin_tools(&data_root, &tools_clone))
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
pub(in crate::daemon) async fn delete_plugin_tool_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AirpError> {
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
        return Err(AirpError::NotFound(format!(
            "插件工具 '{}' 不存在",
            name
        )));
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
    let result = crate::agent::tools::Tool::call(
        &tool,
        req.params,
        req.confirm,
    )
    .await
    .map_err(|e| AirpError::Internal(format!("插件工具 '{}' 测试调用失败: {}", name, e)))?;

    Ok(Json(PluginToolTestResponse {
        name: config.name,
        output: result.output,
        dry_run: result.dry_run,
    }))
}
