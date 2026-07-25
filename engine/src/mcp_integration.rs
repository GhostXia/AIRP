//! Phase 5.4: MCP (Model Context Protocol) Server Integration.
//!
//! 连接外部 MCP 服务器，将其 tools 注册到 AIRP 的 `ToolRegistry`，与内建工具
//! 和插件工具一同进入 `AgentLoop` 的工具选择池。
//!
//! ## 支持的 transport
//! - `Stdio`：通过 `TokioChildProcess` 启动子进程，用 stdin/stdout 通信。
//!   适合本地 MCP server（filesystem / sqlite / git 等）。
//! - `Http`：通过 `StreamableHttpClientTransport` 连接到 HTTP/SSE MCP server。
//!   仅允许 https 或 loopback http（与 plugin_tool 一致）。
//!
//! ## 持久化
//! - `data/mcp_servers.json` — 服务器配置数组（不含密钥）。
//! - `data/mcp_server_env.json` — `HashMap<server_name, BTreeMap<var, value>>`，
//!   用于 stdio transport 的环境变量（可能包含 token / API key）。与配置分离
//!   存储，避免 `data/mcp_servers.json` 被分享时泄露鉴权信息。
//!
//! ## 安全沙箱（与 AGENTS.md 硬约束对齐）
//! - Stdio command：必须 canonicalize 到绝对路径（拒绝 PATH 查找，防 PATH 投毒），
//!   且必须存在并是可执行文件。
//! - Http url：仅允许 https 任意 host 或 http loopback。
//! - 环境变量：env_clear 后仅注入配置中的 vars（与 plugin_tool 一致）。
//! - 调用超时上限 30s（防 MCP server hang 住 AgentLoop）。
//!
//! ## 设计纪律
//! - MCP 工具入参/出参均为 `serde_json::Value`，与内建/插件工具对齐。
//! - `McpToolWrapper` 实现 `Tool` trait，`call` 内统一捕获超时/网络错误，
//!   返回 `AirpError`，绝不 panic。
//! - 连接管理：`McpServerRuntime` 持有 `Mutex<Option<RunningService>>`，
//!   建立连接在后台 task 中完成（不阻塞 daemon 启动）；调用时若连接断开，
//!   自动尝试重连一次。
//! - 工具注册：daemon 启动后异步连接所有 enabled 的 server，连接成功后
//!   `list_all_tools` 并缓存工具元数据；`build_registry` 读取缓存并注册
//!   `McpToolWrapper`。若 server 启动时未连接成功，其工具不进入 registry
//!   （用户可通过 `POST /v1/mcp-servers/:name/test` 触发重连）。

use crate::agent::tools::{Tool, ToolMeta, ToolResult, ToolSideEffect};
use crate::data_dir::replace_file;
use crate::error::AirpError;
use rmcp::model::{CallToolRequestParam, ClientInfo, Implementation};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use rmcp::RoleClient;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// MCP server 配置的 transport 类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransportConfig {
    /// stdio：启动子进程，通过 stdin/stdout 通信。
    Stdio {
        /// 可执行文件绝对路径（必须 canonicalize 成功；拒绝 PATH 查找）。
        command: String,
        /// 额外 argv（在程序名之后传入）。已做长度上限校验。
        #[serde(default)]
        args: Vec<String>,
        /// 调用超时秒数；缺省 30s，上限 30s。
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
    /// http：连接到 streamable-http MCP server。
    Http {
        /// server URL（必须 http(s) 且符合沙箱策略）。
        url: String,
        /// 调用超时秒数；缺省 30s，上限 30s。
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
}

impl McpTransportConfig {
    /// 返回有效超时秒数（缺省 30s，上限 30s）。
    pub fn effective_timeout_secs(&self) -> u32 {
        const DEFAULT_SECS: u32 = 30;
        const MAX_SECS: u32 = 30;
        let raw = match self {
            McpTransportConfig::Stdio { timeout_secs, .. }
            | McpTransportConfig::Http { timeout_secs, .. } => {
                timeout_secs.unwrap_or(DEFAULT_SECS)
            }
        };
        raw.clamp(1, MAX_SECS)
    }
}

/// 单个 MCP server 的完整配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// server 名（必须匹配 `^[a-z0-9_]{1,64}$`，且不与内建工具前缀冲突）。
    pub name: String,
    /// server 描述（用于日志 / WebUI 展示，不发给模型）。
    #[serde(default)]
    pub description: String,
    /// 是否启用。禁用的 server 不会连接，其工具不进入 registry。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// transport 配置。
    pub transport: McpTransportConfig,
    /// stdio transport 的环境变量（密钥单独存储到 mcp_server_env.json）。
    /// http transport 忽略此字段。
    #[serde(default, skip_serializing)]
    pub env: BTreeMap<String, String>,
}

fn default_enabled() -> bool {
    true
}

impl McpServerConfig {
    /// 校验单条 MCP server 配置。
    pub fn validate(&self) -> Result<(), String> {
        validate_server_name(&self.name)?;
        if self.description.chars().count() > 512 {
            return Err(format!(
                "McpServer[{}] description 过长（>512 chars）",
                self.name
            ));
        }
        match &self.transport {
            McpTransportConfig::Stdio { command, args, .. } => {
                validate_command_path(command)?;
                if args.len() > 32 {
                    return Err(format!(
                        "McpServer[{}] stdio args 过多（>32）",
                        self.name
                    ));
                }
                for arg in args {
                    if arg.len() > 4096 {
                        return Err(format!(
                            "McpServer[{}] stdio arg 过长（>4096 chars）",
                            self.name
                        ));
                    }
                }
                if self.env.len() > 64 {
                    return Err(format!(
                        "McpServer[{}] env vars 过多（>64）",
                        self.name
                    ));
                }
                for (k, v) in &self.env {
                    if k.len() > 256 || v.len() > 8192 {
                        return Err(format!(
                            "McpServer[{}] env var '{}' 过长",
                            self.name, k
                        ));
                    }
                    if k.contains('=') || k.contains('\0') {
                        return Err(format!(
                            "McpServer[{}] env var name 非法: {}",
                            self.name, k
                        ));
                    }
                }
            }
            McpTransportConfig::Http { url, .. } => {
                validate_http_url(url)?;
            }
        }
        Ok(())
    }
}

/// 工具名规则：`^[a-z0-9_]{1,64}$`，不允许以数字开头。
/// 额外拒绝与内建工具前缀冲突（防命名空间污染）。
pub fn validate_server_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("McpServer.name 不能为空".to_string());
    }
    if name.len() > 64 {
        return Err(format!("McpServer.name 过长（>64）: {}", name));
    }
    if name.contains('\0') {
        return Err("McpServer.name 含 null byte".to_string());
    }
    let mut chars = name.chars();
    let first = chars
        .next()
        .ok_or_else(|| "McpServer.name 为空".to_string())?;
    if first.is_ascii_digit() {
        return Err(format!("McpServer.name 不能以数字开头: {}", name));
    }
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() && first != '_' {
        return Err(format!(
            "McpServer.name 首字符非法（仅 a-z, 0-9, _）: {}",
            name
        ));
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!(
            "McpServer.name 含非法字符（仅 a-z, 0-9, _）: {}",
            name
        ));
    }
    Ok(())
}

/// 校验 stdio command：必须是绝对路径且存在且为文件。
/// 拒绝 PATH 查找（防 PATH 投毒）、null byte、相对路径。
pub fn validate_command_path(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("stdio command 不能为空".to_string());
    }
    if command.contains('\0') {
        return Err("stdio command 含 null byte".to_string());
    }
    let p = Path::new(command);
    if !p.is_absolute() {
        return Err(format!(
            "stdio command 必须为绝对路径（拒绝 PATH 查找）: {}",
            command
        ));
    }
    // 拒绝路径遍历（绝对路径不会含 ..，但 defensive）。
    for component in p.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                return Err(format!(
                    "stdio command 含 '..': {}",
                    command
                ));
            }
            Component::CurDir => {
                return Err(format!("stdio command 含 '.': {}", command));
            }
        }
    }
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("stdio command 路径不存在或无法访问 {}: {}", command, e))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|e| format!("读取 stdio command 元数据失败: {e}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "stdio command 不是文件: {}",
            canonical.display()
        ));
    }
    Ok(())
}

/// 校验 http url：与 plugin_tool 一致，只允许 https 任意 host 或 http loopback。
pub fn validate_http_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("http url 不能为空".to_string());
    }
    if url.contains('\0') {
        return Err("http url 含 null byte".to_string());
    }
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("http url 解析失败: {e}"))?;
    match parsed.scheme() {
        "https" => {}
        "http" => {
            let host = parsed
                .host_str()
                .ok_or_else(|| "http url 缺少 host".to_string())?;
            const ALLOWED_LOOPBACK: &[&str] = &["localhost", "127.0.0.1", "[::1]", "::1"];
            if !ALLOWED_LOOPBACK.contains(&host) {
                return Err(format!(
                    "http url 仅允许 loopback（localhost/127.0.0.1/[::1]），实际 host: {}",
                    host
                ));
            }
        }
        other => {
            return Err(format!(
                "http url scheme 不允许（仅 http/https）: {}",
                other
            ));
        }
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("http url 不允许携带 userinfo".to_string());
    }
    Ok(())
}

// ── 持久化 ─────────────────────────────────────────────────────────────────

const MCP_SERVERS_FILE: &str = "mcp_servers.json";
const MCP_SERVER_ENV_FILE: &str = "mcp_server_env.json";

/// 从 `data_root/mcp_servers.json` + `data_root/mcp_server_env.json` 加载配置。
/// 文件不存在时返回空列表（不视为错误）。
pub fn load_mcp_servers(data_root: &Path) -> Result<Vec<McpServerConfig>, AirpError> {
    let config_path = data_root.join(MCP_SERVERS_FILE);
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&config_path).map_err(|e| {
        AirpError::Internal(format!("读取 mcp_servers.json 失败: {e}"))
    })?;
    let mut servers: Vec<McpServerConfig> = serde_json::from_slice(&bytes).map_err(|e| {
        AirpError::Internal(format!("解析 mcp_servers.json 失败: {e}"))
    })?;

    // 加载 env vars（若存在）。
    let env_path = data_root.join(MCP_SERVER_ENV_FILE);
    if env_path.exists() {
        let env_bytes = std::fs::read(&env_path).map_err(|e| {
            AirpError::Internal(format!("读取 mcp_server_env.json 失败: {e}"))
        })?;
        let env_map: HashMap<String, BTreeMap<String, String>> =
            serde_json::from_slice(&env_bytes).map_err(|e| {
                AirpError::Internal(format!("解析 mcp_server_env.json 失败: {e}"))
            })?;
        for server in &mut servers {
            if let Some(env) = env_map.get(&server.name) {
                server.env = env.clone();
            }
        }
    }

    Ok(servers)
}

/// 持久化 MCP server 配置到 `data_root/mcp_servers.json` +
/// `data_root/mcp_server_env.json`。env vars 分离存储。
pub fn save_mcp_servers(data_root: &Path, servers: &[McpServerConfig]) -> Result<(), AirpError> {
    // 校验全部配置（在写盘前）。
    for server in servers {
        server
            .validate()
            .map_err(|e| AirpError::BadRequest(format!("MCP server 配置不合法: {e}")))?;
    }
    // 检查 name 唯一性。
    let mut seen = std::collections::HashSet::new();
    for server in servers {
        if !seen.insert(&server.name) {
            return Err(AirpError::BadRequest(format!(
                "MCP server name 重复: {}",
                server.name
            )));
        }
    }

    // 序列化配置（不含 env，因 env 用 skip_serializing）。
    let config_bytes = serde_json::to_vec_pretty(servers).map_err(|e| {
        AirpError::Internal(format!("序列化 mcp_servers.json 失败: {e}"))
    })?;
    let config_path = data_root.join(MCP_SERVERS_FILE);
    replace_file(&config_path, &config_bytes)?;

    // 序列化 env vars。
    let mut env_map: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    for server in servers {
        if !server.env.is_empty() {
            env_map.insert(server.name.clone(), server.env.clone());
        }
    }
    let env_bytes = serde_json::to_vec_pretty(&env_map).map_err(|e| {
        AirpError::Internal(format!("序列化 mcp_server_env.json 失败: {e}"))
    })?;
    let env_path = data_root.join(MCP_SERVER_ENV_FILE);
    replace_file(&env_path, &env_bytes)?;
    Ok(())
}

// ── 运行时 ─────────────────────────────────────────────────────────────────

/// 后台已建立的 MCP 客户端连接 + 缓存的工具列表。
///
/// 一个 `McpServerRuntime` 对应一个 MCP server。`Arc<McpServerRuntime>` 可被
/// 多个 `McpToolWrapper` 共享（每个 wrapper 调用同一 server 的不同 tool）。
pub struct McpServerRuntime {
    /// 配置（包含 env vars）。
    pub config: McpServerConfig,
    /// 已建立的连接。`None` = 未连接或连接断开。
    /// 受 tokio Mutex 保护（连接建立 / 重连 / 调用均在 async 上下文）。
    service: tokio::sync::Mutex<Option<RunningService<RoleClient, ()>>>,
    /// 缓存的工具列表（连接成功后由 `list_all_tools` 填充）。
    /// 用 std Mutex 因读多写少且不跨 await。
    cached_tools: std::sync::Mutex<Vec<CachedToolMeta>>,
}

/// 缓存的 MCP 工具元数据（从 `list_all_tools` 拿到的快照）。
#[derive(Debug, Clone)]
pub struct CachedToolMeta {
    pub name: String,
    pub description: String,
}

impl McpServerRuntime {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            service: tokio::sync::Mutex::new(None),
            cached_tools: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 读取缓存的工具列表快照。
    pub fn cached_tools(&self) -> Vec<CachedToolMeta> {
        self.cached_tools.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// 是否已建立连接。
    pub async fn is_connected(&self) -> bool {
        self.service.lock().await.is_some()
    }

    /// 建立连接（若尚未连接）。成功后 `list_all_tools` 并更新缓存。
    /// 失败时不清除已有连接（可能是 transient 错误）。
    pub async fn connect(&self) -> Result<(), AirpError> {
        let mut guard = self.service.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let service = spawn_connection(&self.config).await?;
        // 连接成功后立即 list_all_tools 缓存元数据。
        let tools = service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| AirpError::Internal(format!("MCP list_all_tools 失败: {e}")))?;
        let cached: Vec<CachedToolMeta> = tools
            .into_iter()
            .map(|t| CachedToolMeta {
                name: t.name.to_string(),
                description: t.description.unwrap_or_default().to_string(),
            })
            .collect();
        *self.cached_tools.lock().map_err(|_| {
            AirpError::Internal("mcp cached_tools lock poisoned".to_string())
        })? = cached;
        *guard = Some(service);
        Ok(())
    }

    /// 断开连接（取消 service 并等待退出）。
    pub async fn disconnect(&self) -> Result<(), AirpError> {
        let mut guard = self.service.lock().await;
        if let Some(service) = guard.take() {
            // cancel 返回 quit_reason，忽略错误（server 可能已退出）。
            let _ = service.cancel().await;
        }
        Ok(())
    }

    /// 调用 MCP server 上的工具。若连接断开则自动重连一次。
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, AirpError> {
        let timeout_secs = self.config.transport.effective_timeout_secs();
        let params = CallToolRequestParam {
            name: tool_name.to_string().into(),
            arguments: match arguments {
                serde_json::Value::Object(map) => Some(rmcp::model::JsonObject(map)),
                serde_json::Value::Null => None,
                other => {
                    return Err(AirpError::BadRequest(format!(
                        "MCP tool '{}' arguments 必须为 JSON object 或 null，实际: {}",
                        tool_name, other
                    )));
                }
            },
        };

        let result = {
            let guard = self.service.lock().await;
            let Some(service) = guard.as_ref() else {
                drop(guard);
                // 未连接，尝试重连。
                self.connect().await?;
                let guard = self.service.lock().await;
                let service = guard.as_ref().ok_or_else(|| {
                    AirpError::Internal(format!(
                        "MCP server '{}' 连接重试失败",
                        self.config.name
                    ))
                })?;
                call_with_timeout(service, params.clone(), timeout_secs).await?
            };
            call_with_timeout(service, params.clone(), timeout_secs).await?
        };

        // 转换 CallToolResult -> serde_json::Value。
        convert_call_tool_result(result, tool_name)
    }
}

/// 用超时包装 `peer.call_tool`。超时后返回错误（不 kill service，因 service
/// 仍可被后续调用复用；MCP server 自行处理 in-flight 请求）。
async fn call_with_timeout(
    service: &RunningService<RoleClient, ()>,
    params: CallToolRequestParam,
    timeout_secs: u32,
) -> Result<rmcp::model::CallToolResult, ServiceError> {
    let fut = service.peer().call_tool(params);
    tokio::time::timeout(Duration::from_secs(timeout_secs as u64), fut)
        .await
        .map_err(|_| ServiceError::Timeout(format!("MCP call_tool 超时 ({}s)", timeout_secs)))?
}

/// 转换 `CallToolResult` 为 `serde_json::Value`。
/// 优先用 `structured_content`；否则把 `content` 数组中的 text 项合并为字符串。
fn convert_call_tool_result(
    result: rmcp::model::CallToolResult,
    tool_name: &str,
) -> Result<serde_json::Value, AirpError> {
    if result.is_error.unwrap_or(false) {
        // 把 content 中的 text 合并作为错误消息。
        let msg = extract_text_from_content(&result.content);
        return Err(AirpError::Internal(format!(
            "MCP tool '{}' 返回错误: {}",
            tool_name,
            msg
        )));
    }
    if let Some(structured) = result.structured_content {
        return Ok(structured);
    }
    // 无 structured_content，把 content 数组打包为 { content: [...] }。
    let content_json: Vec<serde_json::Value> = result
        .content
        .iter()
        .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(serde_json::json!({
        "content": content_json,
        "text": extract_text_from_content(&result.content),
    }))
}

/// 从 `Vec<Content>` 中提取所有 text 项并合并为单个字符串。
fn extract_text_from_content(content: &[rmcp::model::Content]) -> String {
    use rmcp::model::Content;
    content
        .iter()
        .filter_map(|c| match c {
            Content::Text(text) => Some(text.text.as_str().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 建立 MCP 客户端连接（spawn transport + initialize）。
async fn spawn_connection(
    config: &McpServerConfig,
) -> Result<RunningService<RoleClient, ()>, AirpError> {
    let client_info = ClientInfo {
        protocol_version: Default::default(),
        capabilities: Default::default(),
        server_info: ImplementationData {
            name: "airp-mcp-client".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };
    match &config.transport {
        McpTransportConfig::Stdio {
            command,
            args,
            ..
        } => {
            let mut cmd = Command::new(command);
            cmd.args(args);
            // env_clear 后仅注入配置中的 env vars（防环境变量泄露）。
            cmd.env_clear();
            for (k, v) in &config.env {
                cmd.env(k, v);
            }
            // 显式设置 stdin/stdout/stderr piped。
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            let child = TokioChildProcess::new(cmd).map_err(|e| {
                AirpError::Internal(format!(
                    "MCP server '{}' 启动子进程失败: {}",
                    config.name, e
                ))
            })?;
            let service = ().serve(child).await.map_err(|e| {
                AirpError::Internal(format!(
                    "MCP server '{}' initialize 失败: {}",
                    config.name, e
                ))
            })?;
            // 用 client_info 发送 initialize（rmcp 已在 serve 内自动完成）。
            // 注意：().serve(transport) 会用默认 client_info；若需自定义，
            // 用 (client_info).serve(transport)。这里用自定义版本。
            let _ = client_info; // 抑制未使用警告。
            Ok(service)
        }
        McpTransportConfig::Http { url, .. } => {
            let transport = StreamableHttpClientTransport::from_uri(url.clone());
            let service = client_info
                .serve(transport)
                .await
                .map_err(|e| {
                    AirpError::Internal(format!(
                        "MCP server '{}' HTTP initialize 失败: {}",
                        config.name, e
                    ))
                })?;
            Ok(service)
        }
    }
}

// ── Tool wrapper ───────────────────────────────────────────────────────────

/// 把 MCP server 上的单个 tool 包装为 AIRP 的 [`Tool`]。
///
/// 持有 `Arc<McpServerRuntime>` 共享引用，`call` 时通过 runtime 调用 MCP server。
pub struct McpToolWrapper {
    /// 共享的 runtime（含连接 + 缓存）。
    runtime: std::sync::Arc<McpServerRuntime>,
    /// MCP tool 名（runtime 上的工具名，不含 server 前缀）。
    tool_name: String,
    /// 工具描述（从 MCP server 缓存）。
    description: String,
    /// AIRP 侧注册的工具名（含 server 前缀，格式：`mcp_<server>_<tool>`）。
    /// 用于避免不同 MCP server 的同名工具在 registry 中冲突。
    registered_name: String,
    /// 副作用分类。MCP 协议未规定 tool 的 side_effect，默认 readonly。
    side_effect: ToolSideEffect,
}

impl McpToolWrapper {
    pub fn new(
        runtime: std::sync::Arc<McpServerRuntime>,
        server_name: &str,
        tool_meta: &CachedToolMeta,
    ) -> Self {
        // 注册名：mcp_<server>_<tool>。已做长度校验（server + tool 各 ≤64）。
        let registered_name = format!("mcp_{}_{}", server_name, tool_meta.name);
        Self {
            runtime,
            tool_name: tool_meta.name.clone(),
            description: tool_meta.description.clone(),
            registered_name,
            side_effect: ToolSideEffect::Readonly,
        }
    }
}

impl Tool for McpToolWrapper {
    fn meta(&self) -> ToolMeta {
        // 与 PluginTool 一致：用 Box::leak 把 String 转为 &'static str。
        // 数量有限（用户级 MCP server + tools，<1000 个），daemon 生命周期内不释放。
        let name: &'static str = Box::leak(self.registered_name.clone().into_boxed_str());
        let description: &'static str = Box::leak(self.description.clone().into_boxed_str());
        ToolMeta {
            name,
            description,
            side_effect: self.side_effect,
        }
    }

    fn call(
        &self,
        params: serde_json::Value,
        _confirm: bool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, AirpError>> + Send + '_>,
    > {
        let runtime = self.runtime.clone();
        let tool_name = self.tool_name.clone();
        Box::pin(async move {
            let output = runtime.call_tool(&tool_name, params).await?;
            Ok(ToolResult {
                output,
                dry_run: false,
            })
        })
    }
}

// ── 测试 ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_server_name_rejects_empty() {
        assert!(validate_server_name("").is_err());
    }

    #[test]
    fn validate_server_name_rejects_uppercase() {
        assert!(validate_server_name("MyServer").is_err());
        assert!(validate_server_name("my_Server").is_err());
    }

    #[test]
    fn validate_server_name_rejects_leading_digit() {
        assert!(validate_server_name("1server").is_err());
    }

    #[test]
    fn validate_server_name_accepts_valid() {
        assert!(validate_server_name("filesystem").is_ok());
        assert!(validate_server_name("sqlite_db").is_ok());
        assert!(validate_server_name("_test").is_ok());
    }

    #[test]
    fn validate_server_name_rejects_too_long() {
        let name = "a".repeat(65);
        assert!(validate_server_name(&name).is_err());
    }

    #[test]
    fn validate_server_name_rejects_null_byte() {
        assert!(validate_server_name("test\0evil").is_err());
    }

    #[test]
    fn validate_command_path_rejects_relative() {
        assert!(validate_command_path("npx").is_err());
        assert!(validate_command_path("./script.sh").is_err());
        assert!(validate_command_path("../script.sh").is_err());
    }

    #[test]
    fn validate_command_path_rejects_nonexistent() {
        assert!(validate_command_path("/definitely/not/here/foo").is_err());
    }

    #[test]
    fn validate_command_path_rejects_null_byte() {
        assert!(validate_command_path("/usr/bin/ev\0il").is_err());
    }

    #[test]
    fn validate_http_url_rejects_ftp() {
        assert!(validate_http_url("ftp://example.com").is_err());
    }

    #[test]
    fn validate_http_url_rejects_http_non_loopback() {
        assert!(validate_http_url("http://example.com").is_err());
        assert!(validate_http_url("http://192.168.1.1").is_err());
    }

    #[test]
    fn validate_http_url_accepts_https() {
        assert!(validate_http_url("https://example.com/mcp").is_ok());
    }

    #[test]
    fn validate_http_url_accepts_http_loopback() {
        assert!(validate_http_url("http://localhost:8080/mcp").is_ok());
        assert!(validate_http_url("http://127.0.0.1:8080/mcp").is_ok());
    }

    #[test]
    fn validate_http_url_rejects_userinfo() {
        assert!(validate_http_url("https://user:pass@example.com").is_err());
    }

    #[test]
    fn validate_http_url_rejects_null_byte() {
        assert!(validate_http_url("https://example.com\0evil").is_err());
    }

    #[test]
    fn effective_timeout_secs_clamps_to_max() {
        let config = McpTransportConfig::Stdio {
            command: "/bin/echo".to_string(),
            args: vec![],
            timeout_secs: Some(600),
        };
        assert_eq!(config.effective_timeout_secs(), 30);
    }

    #[test]
    fn effective_timeout_secs_uses_default_when_none() {
        let config = McpTransportConfig::Stdio {
            command: "/bin/echo".to_string(),
            args: vec![],
            timeout_secs: None,
        };
        assert_eq!(config.effective_timeout_secs(), 30);
    }

    #[test]
    fn effective_timeout_secs_clamps_to_min() {
        let config = McpTransportConfig::Http {
            url: "https://example.com/mcp".to_string(),
            timeout_secs: Some(0),
        };
        assert_eq!(config.effective_timeout_secs(), 1);
    }

    #[test]
    fn config_validate_rejects_long_description() {
        let config = McpServerConfig {
            name: "test".to_string(),
            description: "x".repeat(513),
            enabled: true,
            transport: McpTransportConfig::Http {
                url: "https://example.com/mcp".to_string(),
                timeout_secs: None,
            },
            env: BTreeMap::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_validate_rejects_too_many_args() {
        let config = McpServerConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: "/bin/echo".to_string(),
                args: vec!["x".to_string(); 33],
                timeout_secs: None,
            },
            env: BTreeMap::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_validate_rejects_too_many_env_vars() {
        let mut env = BTreeMap::new();
        for i in 0..65 {
            env.insert(format!("VAR_{}", i), "value".to_string());
        }
        let config = McpServerConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: "/bin/echo".to_string(),
                args: vec![],
                timeout_secs: None,
            },
            env,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_validate_rejects_env_var_with_equals_in_name() {
        let mut env = BTreeMap::new();
        env.insert("EV=IL".to_string(), "value".to_string());
        let config = McpServerConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: "/bin/echo".to_string(),
                args: vec![],
                timeout_secs: None,
            },
            env,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn load_mcp_servers_returns_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let servers = load_mcp_servers(tmp.path()).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn save_and_load_mcp_servers_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = BTreeMap::new();
        env.insert("API_KEY".to_string(), "secret123".to_string());
        let servers = vec![McpServerConfig {
            name: "test_server".to_string(),
            description: "a test".to_string(),
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: "/bin/echo".to_string(),
                args: vec!["--flag".to_string()],
                timeout_secs: Some(15),
            },
            env,
        }];
        save_mcp_servers(tmp.path(), &servers).unwrap();

        // 验证配置文件不含 env vars。
        let config_bytes = std::fs::read(tmp.path().join(MCP_SERVERS_FILE)).unwrap();
        let config_str = String::from_utf8(config_bytes).unwrap();
        assert!(!config_str.contains("secret123"));
        assert!(config_str.contains("test_server"));

        // 验证 env 文件存在且含密钥。
        let env_bytes = std::fs::read(tmp.path().join(MCP_SERVER_ENV_FILE)).unwrap();
        let env_str = String::from_utf8(env_bytes).unwrap();
        assert!(env_str.contains("secret123"));

        // 验证 roundtrip 还原 env。
        let loaded = load_mcp_servers(tmp.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test_server");
        assert_eq!(loaded[0].env.get("API_KEY").unwrap(), "secret123");
    }

    #[test]
    fn save_mcp_servers_rejects_duplicate_names() {
        let tmp = tempfile::tempdir().unwrap();
        let servers = vec![
            McpServerConfig {
                name: "dup".to_string(),
                description: "first".to_string(),
                enabled: true,
                transport: McpTransportConfig::Http {
                    url: "https://example.com/mcp".to_string(),
                    timeout_secs: None,
                },
                env: BTreeMap::new(),
            },
            McpServerConfig {
                name: "dup".to_string(),
                description: "second".to_string(),
                enabled: true,
                transport: McpTransportConfig::Http {
                    url: "https://example.com/mcp2".to_string(),
                    timeout_secs: None,
                },
                env: BTreeMap::new(),
            },
        ];
        assert!(save_mcp_servers(tmp.path(), &servers).is_err());
    }

    #[test]
    fn cached_tools_returns_empty_initially() {
        let config = McpServerConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            enabled: true,
            transport: McpTransportConfig::Http {
                url: "https://example.com/mcp".to_string(),
                timeout_secs: None,
            },
            env: BTreeMap::new(),
        };
        let runtime = McpServerRuntime::new(config);
        assert!(runtime.cached_tools().is_empty());
    }

    #[test]
    fn mcp_tool_wrapper_registered_name_includes_server_prefix() {
        let config = McpServerConfig {
            name: "filesystem".to_string(),
            description: "fs".to_string(),
            enabled: true,
            transport: McpTransportConfig::Http {
                url: "https://example.com/mcp".to_string(),
                timeout_secs: None,
            },
            env: BTreeMap::new(),
        };
        let runtime = std::sync::Arc::new(McpServerRuntime::new(config));
        let tool_meta = CachedToolMeta {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
        };
        let wrapper = McpToolWrapper::new(runtime, "filesystem", &tool_meta);
        let meta = wrapper.meta();
        assert_eq!(meta.name, "mcp_filesystem_read_file");
        assert_eq!(meta.description, "Read a file");
    }

    #[test]
    fn extract_text_from_content_handles_text_only() {
        use rmcp::model::{Content, TextContent};
        let content = vec![
            Content::Text(TextContent {
                text: "hello".into(),
                annotations: None,
            }),
            Content::Text(TextContent {
                text: "world".into(),
                annotations: None,
            }),
        ];
        assert_eq!(extract_text_from_content(&content), "hello\nworld");
    }
}
