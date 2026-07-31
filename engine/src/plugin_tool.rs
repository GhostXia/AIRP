//! Phase 5.3: Plugin / Custom Agent Tools.
//!
//! 允许用户通过 HTTP webhook 或本地脚本注册自定义 Agent 工具。注册后的工具
//! 会与内建工具一同进入 `ToolRegistry`，可被 `AgentLoop` 在规划阶段选中。
//!
//! ## 持久化
//! - `data/plugin_tools.json` — 工具配置数组（不含密钥）。
//! - `data/plugin_tool_headers.json` — `HashMap<tool_name, BTreeMap<header, value>>`，
//!   用于 webhook 自定义 header（如 `Authorization: Bearer ...`）。与配置分离存储，
//!   避免 `data/plugin_tools.json` 被分享时泄露鉴权信息。
//!
//! ## 安全沙箱（与 AGENTS.md 硬约束对齐）
//! - Webhook URL：只允许 `http://localhost|127.0.0.1|[::1]` 或任意 `https://`。
//!   拒绝 `file://`、`ftp://`、null byte、非 ASCII host。
//! - HTTPS SSRF（#329 N3 / #381 E-P0-3）：注册时与**每次请求前**均解析 DNS；
//!   解析失败 / 无记录 / 任一地址为内网 → **fail-closed 拒绝**；域名目标在 connect
//!   前把解析结果 pin 进专用 client（`resolve_to_addrs`），缩小 DNS rebinding 窗口。
//! - Script path：必须在 `data_root/plugins/` 下（用 `canonicalize` + 起始前缀校验），
//!   必须存在且为文件；拒绝 null byte 与 `..` 路径遍历。
//! - 执行限制：超时上限 30s、stdout/响应体上限 1 MiB、HTTP 请求体上限 1 MiB。
//! - HTTP webhook 默认走 daemon 共享 `reqwest::Client`（`redirect::Policy::none()`）；
//!   需 pin 的 HTTPS 域名走同策略的一次性 client。
//!
//! ## 设计纪律
//! - 工具入参/出参均为 `serde_json::Value`，与内建工具对齐（开放接入戒律）。
//! - `PluginTool` 实现 `Tool` trait，`call` 内统一捕获超时/输出截断/网络错误，
//!   返回 `AirpError`，绝不 panic。
//! - 破坏性工具的 `confirm` 语义：webhook/script 收到 `confirm` 字段后自行决定，
//!   AIRP 不替插件做 dry-run 模拟（与内建 destructive 工具不同）。

use crate::agent::tools::{Tool, ToolMeta, ToolResult, ToolSideEffect};
use crate::data_dir::replace_file;
use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// 插件工具调用方式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginInvocation {
    /// HTTP webhook：POST 到指定 URL，参数作为 JSON body。
    /// 响应体按 JSON 解析后作为 `ToolResult.output` 返回。
    Webhook {
        /// 完整 URL（必须 http(s) 且符合沙箱策略）。
        url: String,
        /// 自定义请求头名称→值的映射（密钥不入 plugin_tools.json，
        /// 由 `data/plugin_tool_headers.json` 单独持久化）。
        #[serde(default, skip_serializing)]
        headers: BTreeMap<String, String>,
        /// 调用超时秒数；缺省 30s，上限 30s。
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
    /// 本地脚本：执行 `data_root/plugins/` 下的可执行文件，
    /// 参数以 JSON 写入 stdin，stdout 解析为 JSON 作为 `ToolResult.output`。
    Script {
        /// 相对于 `data_root/plugins/` 的脚本路径（必须在该目录下）。
        relative_path: String,
        /// 额外 argv（在程序名之后传入）。已做长度上限校验。
        #[serde(default)]
        args: Vec<String>,
        /// 调用超时秒数；缺省 30s，上限 30s。
        #[serde(default)]
        timeout_secs: Option<u32>,
    },
}

impl PluginInvocation {
    /// 返回有效超时秒数（缺省 30s，上限 30s）。
    pub fn effective_timeout_secs(&self) -> u32 {
        const DEFAULT_SECS: u32 = 30;
        const MAX_SECS: u32 = 30;
        let raw = match self {
            PluginInvocation::Webhook { timeout_secs, .. }
            | PluginInvocation::Script { timeout_secs, .. } => timeout_secs.unwrap_or(DEFAULT_SECS),
        };
        raw.clamp(1, MAX_SECS)
    }
}

/// 单个插件工具的完整配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginToolConfig {
    /// 工具名（必须匹配 `^[a-z0-9_]{1,64}$`，且不与内建工具冲突）。
    pub name: String,
    /// 工具描述（建议英文 + 简短，模型选择工具时主要依据）。
    pub description: String,
    /// 副作用分类，与内建工具一致。
    #[serde(default)]
    pub side_effect: PluginSideEffect,
    /// 是否启用。禁用的工具不会被注册到 ToolRegistry。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 调用方式。
    pub invocation: PluginInvocation,
}

fn default_enabled() -> bool {
    true
}

/// 与 [`crate::agent::tools::ToolSideEffect`] 对齐，但需独立序列化以避免
/// lifetime 约束（`ToolSideEffect` 不实现 `Eq`，这里要 `PartialEq + Eq`）。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginSideEffect {
    #[default]
    Readonly,
    Mutate,
    Destructive,
    Append,
}

impl From<PluginSideEffect> for ToolSideEffect {
    fn from(value: PluginSideEffect) -> Self {
        match value {
            PluginSideEffect::Readonly => ToolSideEffect::Readonly,
            PluginSideEffect::Mutate => ToolSideEffect::Mutate,
            PluginSideEffect::Destructive => ToolSideEffect::Destructive,
            PluginSideEffect::Append => ToolSideEffect::Append,
        }
    }
}

impl PluginToolConfig {
    /// 校验单条插件工具配置。
    pub fn validate(&self, data_root: &Path) -> Result<(), String> {
        validate_tool_name(&self.name)?;
        if self.description.trim().is_empty() {
            return Err(format!("PluginTool[{}] description 不能为空", self.name));
        }
        if self.description.chars().count() > 512 {
            return Err(format!(
                "PluginTool[{}] description 过长（>512 chars）",
                self.name
            ));
        }
        match &self.invocation {
            PluginInvocation::Webhook { url, .. } => {
                validate_webhook_url(url)?;
            }
            PluginInvocation::Script {
                relative_path,
                args,
                ..
            } => {
                validate_script_path(data_root, relative_path)?;
                if args.len() > 16 {
                    return Err(format!("PluginTool[{}] script args 过多（>16）", self.name));
                }
                for arg in args {
                    if arg.len() > 4096 {
                        return Err(format!(
                            "PluginTool[{}] script arg 过长（>4096 chars）",
                            self.name
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// 工具名规则：`^[a-z0-9_]{1,64}$`，且不允许以数字开头。
pub fn validate_tool_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("PluginTool.name 不能为空".to_string());
    }
    if name.len() > 64 {
        return Err(format!("PluginTool.name 过长（>64）: {}", name));
    }
    if name.contains('\0') {
        return Err("PluginTool.name 含 null byte".to_string());
    }
    let mut chars = name.chars();
    let first = chars
        .next()
        .ok_or_else(|| "PluginTool.name 为空".to_string())?;
    if first.is_ascii_digit() {
        return Err(format!("PluginTool.name 不能以数字开头: {}", name));
    }
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() && first != '_' {
        return Err(format!(
            "PluginTool.name 首字符非法（仅 a-z, 0-9, _）: {}",
            name
        ));
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!(
            "PluginTool.name 含非法字符（仅 a-z, 0-9, _）: {}",
            name
        ));
    }
    // 保留前缀：与内建工具命名空间冲突的拒绝。
    const BUILTIN_PREFIXES: &[&str] = &[
        "echo",
        "session_",
        "character_",
        "lorebook_",
        "preset_",
        "volume_",
        "analysis_",
        "world_event_",
        "npc_",
        "plot_",
        "search_",
    ];
    for prefix in BUILTIN_PREFIXES {
        if name.starts_with(prefix) {
            return Err(format!(
                "PluginTool.name '{}' 与内建工具命名空间冲突（前缀 '{}）",
                name, prefix
            ));
        }
    }
    Ok(())
}

/// 校验 webhook URL：只允许 `http://localhost`、`127.0.0.1`、`[::1]` 或任意 `https://`。
///
/// **SSRF 控制（Major2, 2026-07-26；E-P0-3 / #329 N3, 2026-07-30）**：
/// - `https://` 拒绝字面量或 DNS 解析到的 loopback / private / link-local / 特殊用途地址；
/// - DNS 解析失败或无记录时 **fail-closed**（不再为可用性放行）；
/// - 本函数供注册路径使用；执行路径在每次 webhook 调用前复用同一套检查并 pin DNS。
pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    inspect_webhook_url(url).map(|_| ())
}

/// 请求连接计划：loopback HTTP / 公网字面量 HTTPS 复用共享 client；
/// HTTPS 域名在 connect 前 pin 到本次解析得到的公网地址。
#[derive(Debug, Clone)]
enum WebhookConnectPlan {
    /// `http://` loopback 或 `https://` 公网字面量 IP。
    SharedClient,
    /// `https://` 域名：host + 已换好目标 port 的公网 SocketAddr 列表。
    PinnedHttps {
        host: String,
        addrs: Vec<SocketAddr>,
    },
}

/// 注册时与请求时共用的 URL 检查；成功时返回 connect 计划。
fn inspect_webhook_url(url: &str) -> Result<WebhookConnectPlan, String> {
    inspect_webhook_url_with(url, system_lookup_host)
}

fn inspect_webhook_url_with<F>(url: &str, lookup: F) -> Result<WebhookConnectPlan, String>
where
    F: FnOnce(&str, u16) -> std::io::Result<Vec<SocketAddr>>,
{
    if url.is_empty() {
        return Err("webhook url 不能为空".to_string());
    }
    if url.contains('\0') {
        return Err("webhook url 含 null byte".to_string());
    }
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("webhook url 解析失败: {e}"))?;
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("webhook url 不允许携带 userinfo".to_string());
    }
    match parsed.scheme() {
        "https" => {
            let host = parsed
                .host_str()
                .ok_or_else(|| "webhook url 缺少 host".to_string())?;
            if host.is_empty() {
                return Err("webhook url host 为空".to_string());
            }
            let port = parsed
                .port_or_known_default()
                .ok_or_else(|| "webhook url 缺少可解析端口".to_string())?;
            if let Some(ip) = parse_literal_ip(host) {
                if is_internal_ip(&ip) {
                    return Err(format!(
                        "https webhook host 解析为内网 IP ({ip})，拒绝 SSRF 风险"
                    ));
                }
                return Ok(WebhookConnectPlan::SharedClient);
            }
            // 域名：解析全部 A/AAAA；失败/空/内网一律拒绝；成功则 pin。
            let addrs = resolve_public_host_addrs_with(host, port, lookup)?;
            Ok(WebhookConnectPlan::PinnedHttps {
                host: host.to_string(),
                addrs,
            })
        }
        "http" => {
            let host = parsed
                .host_str()
                .ok_or_else(|| "webhook url 缺少 host".to_string())?;
            const ALLOWED_LOOPBACK: &[&str] = &["localhost", "127.0.0.1", "[::1]", "::1"];
            if !ALLOWED_LOOPBACK.contains(&host) {
                return Err(format!(
                    "http webhook 仅允许 loopback（localhost/127.0.0.1/[::1]），实际 host: {host}"
                ));
            }
            Ok(WebhookConnectPlan::SharedClient)
        }
        other => Err(format!(
            "webhook url scheme 不允许（仅 http/https）: {other}"
        )),
    }
}

/// 解析字面量 IPv4 / IPv6（含 `[::1]` 形式）。失败返回 None。
fn parse_literal_ip(host: &str) -> Option<std::net::IpAddr> {
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(v4) = trimmed.parse::<std::net::Ipv4Addr>() {
        return Some(std::net::IpAddr::V4(v4));
    }
    if let Ok(v6) = trimmed.parse::<std::net::Ipv6Addr>() {
        return Some(std::net::IpAddr::V6(v6));
    }
    None
}

/// 判断 IP 是否为内网（loopback / private / link-local / unspecified / multicast）。
///
/// **CodeRabbit PR #384 CR1（2026-07-31）**：IPv4-mapped (`::ffff:a.b.c.d`) 与
/// IPv4-compatible (`::a.b.c.d`) 地址必须先拆解为内嵌 IPv4 再判定，否则
/// `https://[::ffff:127.0.0.1]/hook` 会绕过 SSRF 拒绝。
fn is_internal_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
        }
        std::net::IpAddr::V6(v6) => {
            // 先检查原生 IPv6 范围（::1 loopback、:: unspecified、multicast、ULA、link-local）。
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // IPv6 unique local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // IPv6 link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
            {
                return true;
            }
            // IPv4-mapped (::ffff:a.b.c.d) — 拆解后复用 IPv4 判定。
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal_ip(&std::net::IpAddr::V4(v4));
            }
            // IPv4-compatible (::a.b.c.d, deprecated) — 前 96 位为零。
            // :: 与 ::1 已被上面的原生检查捕获；其余 ::a.b.c.d 必须拆解，
            // 否则 ::127.0.0.1 等可绕过内网拒绝。
            let o = v6.octets();
            if o[..12].iter().all(|&b| b == 0) {
                let v4 = std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15]);
                return is_internal_ip(&std::net::IpAddr::V4(v4));
            }
            false
        }
    }
}

/// 系统 DNS 解析（可在测试中替换）。
fn system_lookup_host(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    use std::net::ToSocketAddrs;
    // ToSocketAddrs 对裸 IPv6 需要方括号；域名与 IPv4 直接拼 port。
    let addr_str = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    addr_str.to_socket_addrs().map(|iter| iter.collect())
}

/// 解析 host 的全部 A/AAAA，要求非空且全部为公网地址；返回带目标 port 的 SocketAddr。
///
/// **fail-closed**：解析错误、空记录、任一内网地址均返回 `Err`（#329 N3）。
fn resolve_public_host_addrs_with<F>(
    host: &str,
    port: u16,
    lookup: F,
) -> Result<Vec<SocketAddr>, String>
where
    F: FnOnce(&str, u16) -> std::io::Result<Vec<SocketAddr>>,
{
    let resolved = lookup(host, port)
        .map_err(|e| format!("https webhook host '{host}' DNS 解析失败（fail-closed）: {e}"))?;
    if resolved.is_empty() {
        return Err(format!(
            "https webhook host '{host}' DNS 未返回任何地址（fail-closed）"
        ));
    }
    for socket_addr in &resolved {
        if is_internal_ip(&socket_addr.ip()) {
            return Err(format!(
                "https webhook host '{host}' 解析到内网 IP ({})，拒绝 SSRF 风险",
                socket_addr.ip()
            ));
        }
    }
    Ok(resolved)
}

/// 按 connect 计划选择 client：共享 outbound 或 pin DNS 的一次性 client。
fn client_for_webhook_plan(
    shared: &reqwest::Client,
    plan: &WebhookConnectPlan,
) -> Result<reqwest::Client, AirpError> {
    match plan {
        WebhookConnectPlan::SharedClient => Ok(shared.clone()),
        WebhookConnectPlan::PinnedHttps { host, addrs } => reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15))
            .resolve_to_addrs(host, addrs)
            .build()
            .map_err(|e| AirpError::Internal(format!("插件 webhook pin-DNS client 构建失败: {e}"))),
    }
}

/// 校验脚本路径：必须在 `data_root/plugins/` 下，且必须存在且为文件。
pub fn validate_script_path(data_root: &Path, relative_path: &str) -> Result<(), String> {
    if relative_path.is_empty() {
        return Err("script relative_path 不能为空".to_string());
    }
    if relative_path.contains('\0') {
        return Err("script relative_path 含 null byte".to_string());
    }
    // 拒绝绝对路径与 Windows 盘符路径。
    let p = Path::new(relative_path);
    if p.is_absolute() {
        return Err(format!(
            "script relative_path 不能为绝对路径: {}",
            relative_path
        ));
    }
    // 拒绝 `..` 段。
    for component in p.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "script relative_path 含 '..'（拒绝路径遍历）: {}",
                    relative_path
                ));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!(
                    "script relative_path 含绝对路径段: {}",
                    relative_path
                ));
            }
        }
    }
    let plugins_dir = data_root.join("plugins");
    let full = plugins_dir.join(relative_path);
    let canonical = full
        .canonicalize()
        .map_err(|e| format!("script 路径不存在或无法访问 {}: {}", full.display(), e))?;
    let canonical_plugins = plugins_dir
        .canonicalize()
        .map_err(|e| format!("plugins 目录无法访问 {}: {}", plugins_dir.display(), e))?;
    // 必须在 plugins/ 下（canonical 比较避免大小写/符号链接绕过）。
    if !canonical.starts_with(&canonical_plugins) {
        return Err(format!(
            "script 路径越出 plugins/ 目录: {}",
            canonical.display()
        ));
    }
    let metadata =
        std::fs::metadata(&canonical).map_err(|e| format!("读取 script 元数据失败: {e}"))?;
    if !metadata.is_file() {
        return Err(format!("script 路径不是文件: {}", canonical.display()));
    }
    Ok(())
}

// ── 运行时工具包装 ───────────────────────────────────────────────────────────

/// 包装 [`PluginToolConfig`] 为可执行 [`Tool`]。
///
/// `http_client` 由 daemon 共享注入；script 执行通过 `tokio::process::Command`。
///
/// **Major3, 2026-07-26**：`name_static` / `description_static` 在 `new` 时
/// 一次性 leak，避免 `meta()` 每次调用都重复 leak 同一字符串。
pub struct PluginTool {
    config: PluginToolConfig,
    http_client: reqwest::Client,
    data_root: PathBuf,
    /// 一次性 leak 的工具名（`&'static str`）。
    name_static: &'static str,
    /// 一次性 leak 的工具描述（`&'static str`）。
    description_static: &'static str,
}

impl PluginTool {
    pub fn new(config: PluginToolConfig, http_client: reqwest::Client, data_root: PathBuf) -> Self {
        // Major3: 在构造时一次性 leak name/description，避免 meta() 每次调用都 leak。
        // 工具数量有限（用户级插件，<100 个），daemon 生命周期内不释放。
        let name_static: &'static str = Box::leak(config.name.clone().into_boxed_str());
        let description_static: &'static str =
            Box::leak(config.description.clone().into_boxed_str());
        Self {
            config,
            http_client,
            data_root,
            name_static,
            description_static,
        }
    }

    pub fn config(&self) -> &PluginToolConfig {
        &self.config
    }

    /// 执行 webhook 调用。返回响应 JSON。
    async fn call_webhook(
        &self,
        params: serde_json::Value,
        url: &str,
        headers: &BTreeMap<String, String>,
        timeout_secs: u32,
        confirm: bool,
    ) -> Result<serde_json::Value, AirpError> {
        // E-P0-3 / #329 N3：每次请求前重新解析并 fail-closed；域名 pin 到本次结果。
        // CR3/CR4（CodeRabbit PR #384）：inspect_webhook_url 含同步 DNS 解析，
        // 必须在 spawn_blocking 中执行以免阻塞 tokio 异步运行时；请求时 DNS/策略
        // 失败属环境问题而非客户端请求格式错误，归类为 Internal（与超时/发送失败一致）。
        let url_owned = url.to_string();
        let plan = tokio::task::spawn_blocking(move || inspect_webhook_url(&url_owned))
            .await
            .map_err(|e| {
                AirpError::Internal(format!(
                    "插件工具 {} webhook DNS 校验任务失败: {}",
                    self.config.name, e
                ))
            })?
            .map_err(|e| {
                AirpError::Internal(format!(
                    "插件工具 {} webhook 目标未通过请求时 DNS 校验: {}",
                    self.config.name, e
                ))
            })?;
        let http_client = client_for_webhook_plan(&self.http_client, &plan)?;
        let mut request = http_client.post(url);
        for (key, value) in headers {
            // 拒绝设置 Host / Content-Length / Transfer-Encoding 等被 reqwest 保护的头。
            if is_protected_header(key) {
                return Err(AirpError::BadRequest(format!(
                    "插件工具 header '{}' 被 AIRP 保护，不允许设置",
                    key
                )));
            }
            request = request.header(key, value);
        }
        let body = serde_json::json!({
            "params": params,
            "confirm": confirm,
        });
        request = request.json(&body);
        let response =
            tokio::time::timeout(Duration::from_secs(timeout_secs as u64), request.send())
                .await
                .map_err(|_| {
                    AirpError::Internal(format!(
                        "插件工具 {} webhook 超时 ({}s)",
                        self.config.name, timeout_secs
                    ))
                })?
                .map_err(|e| {
                    AirpError::Internal(format!(
                        "插件工具 {} webhook 调用失败: {}",
                        self.config.name, e
                    ))
                })?;
        let status = response.status();
        // 限制响应体大小（防止内存炸）。
        let bytes = response
            .bytes()
            .await
            .map_err(|e| AirpError::Internal(format!("读取 webhook 响应体失败: {e}")))?;
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err(AirpError::Internal(format!(
                "插件工具 {} webhook 响应体过大 ({} > {} bytes)",
                self.config.name,
                bytes.len(),
                MAX_OUTPUT_BYTES
            )));
        }
        if !status.is_success() {
            return Err(webhook_non_success_error(&self.config.name, status));
        }
        // 解析 JSON；若解析失败则把原文包成 {"raw": "..."}。
        let output = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_else(|_| {
            serde_json::json!({
                "raw": String::from_utf8_lossy(&bytes).to_string()
            })
        });
        Ok(output)
    }

    /// 执行脚本调用。返回 stdout 解析后的 JSON。
    ///
    /// **安全（Critical1, 2026-07-26）**：在 spawn 之前重新 canonicalize 并校验
    /// canonical 路径仍在 `data_root/plugins/` 下，防止配置校验后文件系统被替换
    /// （TOCTOU）。`validate_script_path` 在配置 upsert 时已校验一次，这里是执行
    /// 时第二次校验。
    ///
    /// **并发读写（Critical2, 2026-07-26）**：stdin 写入、stdout/stderr 读取
    /// 与 child wait 并发执行，避免子进程因 pipe buffer 满而阻塞导致超时假死。
    /// stdout/stderr 读取使用 `take(MAX_OUTPUT_BYTES + 1)` 限制内存，超限即报错。
    async fn call_script(
        &self,
        params: serde_json::Value,
        relative_path: &str,
        args: &[String],
        timeout_secs: u32,
        confirm: bool,
    ) -> Result<serde_json::Value, AirpError> {
        let plugins_dir = self.data_root.join("plugins");
        let full = plugins_dir.join(relative_path);
        let canonical = full.canonicalize().map_err(|e| {
            AirpError::Internal(format!(
                "插件工具 {} script 路径无效: {}",
                self.config.name, e
            ))
        })?;
        // Critical1: 执行前重新校验 canonical 路径仍在 plugins/ 下。
        let canonical_plugins = plugins_dir.canonicalize().map_err(|e| {
            AirpError::Internal(format!(
                "plugins 目录无法访问 {}: {}",
                plugins_dir.display(),
                e
            ))
        })?;
        if !canonical.starts_with(&canonical_plugins) {
            return Err(AirpError::Internal(format!(
                "插件工具 {} script 路径越出 plugins/ 目录: {}",
                self.config.name,
                canonical.display()
            )));
        }
        let metadata = std::fs::metadata(&canonical).map_err(|e| {
            AirpError::Internal(format!(
                "插件工具 {} script 元数据读取失败: {}",
                self.config.name, e
            ))
        })?;
        if !metadata.is_file() {
            return Err(AirpError::Internal(format!(
                "插件工具 {} script 路径不是文件: {}",
                self.config.name,
                canonical.display()
            )));
        }
        let mut command = tokio::process::Command::new(&canonical);
        command.args(args);
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        // 显式清理可能影响脚本的环境变量（安全沙箱）。
        command.env_clear();
        // 注入调用上下文。
        let env_params = serde_json::to_string(&params)
            .map_err(|e| AirpError::Internal(format!("序列化 params 失败: {e}")))?;
        command.env("AIRP_PLUGIN_PARAMS", &env_params);
        command.env("AIRP_PLUGIN_CONFIRM", if confirm { "1" } else { "0" });
        command.env("AIRP_PLUGIN_TOOL_NAME", &self.config.name);

        let mut child = command.spawn().map_err(|e| {
            AirpError::Internal(format!(
                "插件工具 {} script 启动失败: {}",
                self.config.name, e
            ))
        })?;
        // 先取出 stdin / stdout / stderr 管道，避免后续 child.wait() 与之冲突
        // （Critical2: 并发 stdin 写入 + stdout/stderr 读取 + wait，避免 pipe 阻塞）。
        let mut stdin_pipe = child.stdin.take();
        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();
        // 任一阶段超时即 kill 子进程并报错。
        let stdin_handle = {
            let payload = serde_json::to_vec(&serde_json::json!({
                "params": params,
                "confirm": confirm,
            }))
            .map_err(|e| AirpError::Internal(format!("序列化 stdin 失败: {e}")))?;
            if payload.len() > MAX_INPUT_BYTES {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(AirpError::Internal(format!(
                    "插件工具 {} stdin payload 过大 ({} > {} bytes)",
                    self.config.name,
                    payload.len(),
                    MAX_INPUT_BYTES
                )));
            }
            tokio::spawn(async move {
                if let Some(mut stdin) = stdin_pipe.take() {
                    // 写入失败时不致命：脚本可能不读 stdin。
                    let _ = stdin.write_all(&payload).await;
                    let _ = stdin.shutdown().await;
                }
            })
        };
        // 并发读取 stdout / stderr，使用 take 限制内存。
        let stdout_handle = tokio::spawn(async move {
            if let Some(stdout) = stdout_pipe.as_mut() {
                use tokio::io::AsyncReadExt;
                // 多读 1 byte 用于判断是否超限。
                let mut buf = Vec::new();
                let _ = stdout
                    .take((MAX_OUTPUT_BYTES as u64) + 1)
                    .read_to_end(&mut buf)
                    .await;
                buf
            } else {
                Vec::new()
            }
        });
        let stderr_handle = tokio::spawn(async move {
            if let Some(stderr) = stderr_pipe.as_mut() {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::new();
                let _ = stderr
                    .take((MAX_OUTPUT_BYTES as u64) + 1)
                    .read_to_end(&mut buf)
                    .await;
                buf
            } else {
                Vec::new()
            }
        });
        // 超时等待 child 退出。
        let wait_result =
            tokio::time::timeout(Duration::from_secs(timeout_secs as u64), child.wait()).await;
        let exit_status = match wait_result {
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                // 不必等待 stdin/stdout/stderr handle 完成：子进程已 kill，pipe 会 EOF。
                return Err(AirpError::Internal(format!(
                    "插件工具 {} script 超时 ({}s)",
                    self.config.name, timeout_secs
                )));
            }
            Ok(Err(e)) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(AirpError::Internal(format!(
                    "插件工具 {} script 执行失败: {}",
                    self.config.name, e
                )));
            }
            Ok(Ok(status)) => status,
        };
        // 等待 stdin 写入完成（写完即结束，不应阻塞）。
        let _ = stdin_handle.await;
        let stdout_buf = stdout_handle
            .await
            .map_err(|e| AirpError::Internal(format!("stdout join 失败: {e}")))?;
        let stderr_buf = stderr_handle
            .await
            .map_err(|e| AirpError::Internal(format!("stderr join 失败: {e}")))?;
        if stdout_buf.len() > MAX_OUTPUT_BYTES {
            return Err(AirpError::Internal(format!(
                "插件工具 {} script stdout 过大 ({} > {} bytes)",
                self.config.name,
                stdout_buf.len(),
                MAX_OUTPUT_BYTES
            )));
        }
        if !exit_status.success() {
            let stderr = String::from_utf8_lossy(&stderr_buf);
            return Err(AirpError::Internal(format!(
                "插件工具 {} script 退出码非 0: {} stderr={}",
                self.config.name,
                exit_status,
                stderr.trim()
            )));
        }
        // 解析 stdout 为 JSON；失败时包成 {"raw": "..."}。
        let value = serde_json::from_slice::<serde_json::Value>(&stdout_buf).unwrap_or_else(|_| {
            serde_json::json!({
                "raw": String::from_utf8_lossy(&stdout_buf).to_string()
            })
        });
        Ok(value)
    }
}

const MAX_OUTPUT_BYTES: usize = 1024 * 1024; // 1 MiB
const MAX_INPUT_BYTES: usize = 1024 * 1024; // 1 MiB

/// 受 reqwest 保护的头列表（不允许插件设置）。
fn webhook_non_success_error(tool_name: &str, status: reqwest::StatusCode) -> AirpError {
    AirpError::Internal(format!(
        "插件工具 {tool_name} webhook 返回非 success 状态: {status}"
    ))
}

fn is_protected_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection" | "content-encoding"
    )
}

impl Tool for PluginTool {
    fn meta(&self) -> ToolMeta {
        // Major3: 使用构造时 leak 的 name_static / description_static，
        // 避免 meta() 每次调用都重复 leak。
        ToolMeta {
            name: self.name_static,
            description: self.description_static,
            side_effect: self.config.side_effect.into(),
        }
    }

    fn call(
        &self,
        params: serde_json::Value,
        confirm: bool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, AirpError>> + Send + '_>,
    > {
        // 返回的 Future 借用 self（`+ '_``），无需 clone config / http_client / data_root。
        // CodeRabbit nitpick (2026-07-26): 直接调用 self.call_webhook / self.call_script，
        // 避免每次 call 重复构造 PluginTool 与重复 leak name_static / description_static。
        Box::pin(async move {
            let timeout = self.config.invocation.effective_timeout_secs();
            let output = match &self.config.invocation {
                PluginInvocation::Webhook { url, headers, .. } => {
                    self.call_webhook(params, url, headers, timeout, confirm)
                        .await?
                }
                PluginInvocation::Script {
                    relative_path,
                    args,
                    ..
                } => {
                    self.call_script(params, relative_path, args, timeout, confirm)
                        .await?
                }
            };
            Ok(ToolResult {
                output,
                dry_run: false,
            })
        })
    }
}

// ── 持久化 ─────────────────────────────────────────────────────────────────

const PLUGIN_TOOLS_FILE_NAME: &str = "plugin_tools.json";
const PLUGIN_TOOL_HEADERS_FILE_NAME: &str = "plugin_tool_headers.json";
const PLUGIN_TOOLS_FILE_VERSION: u32 = 1;

/// `data/plugin_tools.json` 的盘上 schema。
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginToolsFile {
    pub version: u32,
    pub tools: Vec<PluginToolConfig>,
}

/// `data/plugin_tool_headers.json` 的盘上 schema。
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginToolHeadersFile {
    version: u32,
    /// tool_name → headers。
    headers: HashMap<String, BTreeMap<String, String>>,
}

const PLUGIN_TOOL_HEADERS_FILE_VERSION: u32 = 1;

/// `data/plugin_tools.json` 路径。
pub fn plugin_tools_file_path(data_root: &Path) -> PathBuf {
    data_root.join(PLUGIN_TOOLS_FILE_NAME)
}

/// `data/plugin_tool_headers.json` 路径。
pub(crate) fn plugin_tool_headers_file_path(data_root: &Path) -> PathBuf {
    data_root.join(PLUGIN_TOOL_HEADERS_FILE_NAME)
}

/// 从磁盘加载插件工具配置 + headers，合并返回 ready-to-use 配置列表。
///
/// - 文件不存在 → 返回空 Vec（视为未启用插件工具）。
/// - 文件存在但解析失败 → 返回错误。
/// - 文件存在但 version 不匹配 → 返回错误。
pub fn load_plugin_tools(data_root: &Path) -> Result<Vec<PluginToolConfig>, AirpError> {
    let path = plugin_tools_file_path(data_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| AirpError::Internal(format!("读取 plugin_tools.json 失败: {e}")))?;
    let file: PluginToolsFile = serde_json::from_str(&raw)
        .map_err(|e| AirpError::Internal(format!("解析 plugin_tools.json 失败: {e}")))?;
    if file.version != PLUGIN_TOOLS_FILE_VERSION {
        return Err(AirpError::Internal(format!(
            "plugin_tools.json 版本不匹配：期望 {}，实际 {}",
            PLUGIN_TOOLS_FILE_VERSION, file.version
        )));
    }
    // 合并 headers。
    let headers_map = load_plugin_tool_headers(data_root)?;
    let mut tools = file.tools;
    for tool in tools.iter_mut() {
        if let PluginInvocation::Webhook { headers, .. } = &mut tool.invocation {
            if let Some(saved) = headers_map.get(&tool.name) {
                *headers = saved.clone();
            }
        }
    }
    // 校验所有工具。
    for tool in &tools {
        tool.validate(data_root)
            .map_err(|e| AirpError::BadRequest(format!("plugin_tools.json 不合法: {e}")))?;
    }
    Ok(tools)
}

/// 从 `data/plugin_tool_headers.json` 加载所有 webhook 自定义 header。
///
/// 文件不存在或为空 → 返回空 HashMap。
pub fn load_plugin_tool_headers(
    data_root: &Path,
) -> Result<HashMap<String, BTreeMap<String, String>>, AirpError> {
    let path = plugin_tool_headers_file_path(data_root);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| AirpError::Internal(format!("读取 plugin_tool_headers.json 失败: {e}")))?;
    let file: PluginToolHeadersFile = serde_json::from_str(&raw)
        .map_err(|e| AirpError::Internal(format!("解析 plugin_tool_headers.json 失败: {e}")))?;
    if file.version != PLUGIN_TOOL_HEADERS_FILE_VERSION {
        return Err(AirpError::Internal(format!(
            "plugin_tool_headers.json 版本不匹配：期望 {}，实际 {}",
            PLUGIN_TOOL_HEADERS_FILE_VERSION, file.version
        )));
    }
    Ok(file.headers)
}

/// 持久化插件工具配置 + headers。
///
/// headers 字段从 `PluginInvocation::Webhook` 中提取到独立的
/// `data/plugin_tool_headers.json`，避免 `data/plugin_tools.json` 被分享时泄露密钥。
///
/// **Major4, 2026-07-26**：`plugin_tool_headers.json` 含鉴权密钥，
/// 在 unix 平台上设置 0600 权限防止其他用户读取。
///
/// **Critical4, 2026-07-26**：preserve-on-edit 语义。当 webhook 工具的
/// `headers` 字段为空时，**保留**磁盘上已有的 headers（而不是清空）。
/// 这是因为前端编辑时只能拿到 `headers_set: bool`，无法回填真实 headers；
/// 用户不修改 headers 时发空 map，后端应保留原值。如需清空 headers，
/// 用户必须先删除工具再重新创建（或未来增加显式"清空"按钮）。
pub fn save_plugin_tools(data_root: &Path, tools: &[PluginToolConfig]) -> Result<(), AirpError> {
    // 校验全部工具。
    for tool in tools {
        tool.validate(data_root)
            .map_err(|e| AirpError::BadRequest(format!("plugin_tools.json 不合法: {e}")))?;
    }
    // Critical4: 加载已有 headers，对 webhook 工具的空 headers 保留原值。
    let existing_headers = load_plugin_tool_headers(data_root).unwrap_or_default();
    // 提取 headers 并清空配置中的 headers 字段（serde skip 已保证不写入）。
    let mut headers_map: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    let sanitized: Vec<PluginToolConfig> = tools
        .iter()
        .map(|tool| {
            let mut clone = tool.clone();
            if let PluginInvocation::Webhook { headers, .. } = &mut clone.invocation {
                if !headers.is_empty() {
                    // 显式更新：用新 headers 覆盖。
                    headers_map.insert(tool.name.clone(), headers.clone());
                } else if let Some(prev) = existing_headers.get(&tool.name) {
                    // preserve-on-edit：incoming 为空但磁盘有值 → 保留。
                    headers_map.insert(tool.name.clone(), prev.clone());
                }
                headers.clear();
            }
            clone
        })
        .collect();
    let file = PluginToolsFile {
        version: PLUGIN_TOOLS_FILE_VERSION,
        tools: sanitized,
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    let path = plugin_tools_file_path(data_root);
    replace_file(&path, &bytes)?;
    // 持久化 headers（即使为空也写入，保持文件存在性）。
    let headers_file = PluginToolHeadersFile {
        version: PLUGIN_TOOL_HEADERS_FILE_VERSION,
        headers: headers_map,
    };
    let headers_bytes = serde_json::to_vec_pretty(&headers_file)?;
    let headers_path = plugin_tool_headers_file_path(data_root);
    replace_file(&headers_path, &headers_bytes)?;
    // Major4: 凭据文件设置 0600 权限，防止其他用户读取。
    restrict_file_permissions(&headers_path);
    Ok(())
}

/// 在 unix 平台上将文件权限设置为 0600（仅 owner 读写）。
/// Windows 平台无此语义，no-op。失败时仅记录日志，不阻断流程
/// （权限加固是 defense-in-depth，不是安全边界）。
fn restrict_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

// ── 单元测试 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_data_root() -> TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    #[test]
    fn validate_tool_name_accepts_valid() {
        assert!(validate_tool_name("my_plugin").is_ok());
        assert!(validate_tool_name("weather").is_ok());
        assert!(validate_tool_name("_underscore_start").is_ok());
        assert!(validate_tool_name("a").is_ok());
    }

    #[test]
    fn validate_tool_name_rejects_invalid() {
        assert!(validate_tool_name("").is_err());
        assert!(validate_tool_name("1startsWithDigit").is_err());
        assert!(validate_tool_name("has-dash").is_err());
        assert!(validate_tool_name("has.dot").is_err());
        assert!(validate_tool_name("has space").is_err());
        assert!(validate_tool_name("hasUppercase").is_err());
        assert!(
            validate_tool_name("echo").is_err(),
            "echo conflicts with builtin"
        );
        assert!(
            validate_tool_name("session_x").is_err(),
            "session_ prefix reserved"
        );
        assert!(validate_tool_name("character_x").is_err());
        assert!(validate_tool_name("lorebook_x").is_err());
        assert!(validate_tool_name("preset_x").is_err());
        assert!(validate_tool_name("volume_x").is_err());
        assert!(validate_tool_name("analysis_x").is_err());
        assert!(validate_tool_name("world_event_x").is_err());
        assert!(validate_tool_name("npc_x").is_err());
        assert!(validate_tool_name("plot_x").is_err());
        assert!(validate_tool_name("search_x").is_err());
    }

    #[test]
    fn validate_tool_name_rejects_null_byte() {
        assert!(validate_tool_name("evil\0name").is_err());
    }

    #[test]
    fn validate_tool_name_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(validate_tool_name(&long).is_err());
        let ok = "a".repeat(64);
        assert!(validate_tool_name(&ok).is_ok());
    }

    #[test]
    fn validate_webhook_url_accepts_https_public_literal() {
        // 字面量公网 IP 不依赖 DNS；域名路径由 inject lookup 测试覆盖。
        assert!(validate_webhook_url("https://1.1.1.1/hook").is_ok());
        assert!(validate_webhook_url("https://8.8.8.8/v1/tool").is_ok());
    }

    #[test]
    fn validate_webhook_url_accepts_loopback_http() {
        assert!(validate_webhook_url("http://localhost:8080/hook").is_ok());
        assert!(validate_webhook_url("http://127.0.0.1:9000/hook").is_ok());
        assert!(validate_webhook_url("http://[::1]:8080/hook").is_ok());
    }

    #[test]
    fn validate_webhook_url_rejects_non_loopback_http() {
        assert!(validate_webhook_url("http://example.com/hook").is_err());
        assert!(validate_webhook_url("http://192.168.1.1/hook").is_err());
        assert!(validate_webhook_url("http://10.0.0.1/hook").is_err());
    }

    #[test]
    fn validate_webhook_url_rejects_other_schemes() {
        assert!(validate_webhook_url("file:///etc/passwd").is_err());
        assert!(validate_webhook_url("ftp://localhost/x").is_err());
        assert!(validate_webhook_url("gopher://localhost/x").is_err());
    }

    #[test]
    fn validate_webhook_url_rejects_userinfo() {
        assert!(validate_webhook_url("https://user:pass@example.com/hook").is_err());
        assert!(validate_webhook_url("http://user:pass@localhost/hook").is_err());
    }

    #[test]
    fn validate_webhook_url_rejects_null_byte() {
        assert!(validate_webhook_url("https://example.com/hook\0evil").is_err());
    }

    #[test]
    fn validate_webhook_url_rejects_literal_internal_https() {
        assert!(validate_webhook_url("https://127.0.0.1/hook").is_err());
        assert!(validate_webhook_url("https://10.0.0.5/hook").is_err());
        assert!(validate_webhook_url("https://192.168.1.10/hook").is_err());
        assert!(validate_webhook_url("https://[::1]/hook").is_err());
        // CR1: IPv4-mapped / IPv4-compatible IPv6 不得绕过内网拒绝。
        assert!(validate_webhook_url("https://[::ffff:127.0.0.1]/hook").is_err());
        assert!(validate_webhook_url("https://[::ffff:10.0.0.5]/hook").is_err());
        assert!(validate_webhook_url("https://[::ffff:192.168.1.10]/hook").is_err());
        assert!(validate_webhook_url("https://[::127.0.0.1]/hook").is_err());
    }

    #[test]
    fn resolve_public_host_addrs_fail_closed_on_dns_error_empty_and_internal() {
        // 不依赖真实 DNS / ISP 劫持：注入 lookup，覆盖 fail-closed 三分支。
        let err = resolve_public_host_addrs_with("evil.example", 443, |_, _| {
            Err(std::io::Error::other("simulated nxdomain"))
        })
        .expect_err("DNS io error must reject");
        assert!(err.contains("fail-closed"), "err={err}");

        let err = resolve_public_host_addrs_with("evil.example", 443, |_, _| Ok(Vec::new()))
            .expect_err("empty answer must reject");
        assert!(err.contains("未返回任何地址"), "err={err}");

        let err = resolve_public_host_addrs_with("evil.example", 443, |_, _| {
            Ok(vec![
                "8.8.8.8:443".parse().unwrap(),
                "10.0.0.1:443".parse().unwrap(),
            ])
        })
        .expect_err("any internal addr must reject");
        assert!(err.contains("内网"), "err={err}");

        let ok = resolve_public_host_addrs_with("evil.example", 443, |_, _| {
            Ok(vec![
                "8.8.8.8:443".parse().unwrap(),
                "1.1.1.1:443".parse().unwrap(),
            ])
        })
        .expect("all-public must accept");
        assert_eq!(ok.len(), 2);
    }

    #[test]
    fn inspect_webhook_url_pins_public_https_domain() {
        // 注入 lookup，避免 CI/本机 DNS 差异导致不稳定。
        let plan = inspect_webhook_url_with("https://example.com/hook", |host, port| {
            assert_eq!(host, "example.com");
            assert_eq!(port, 443);
            Ok(vec![
                "93.184.216.34:443".parse().unwrap(),
                "[2606:2800:220:1:248:1893:25c8:1946]:443".parse().unwrap(),
            ])
        })
        .expect("injected public answers must pin");
        match plan {
            WebhookConnectPlan::PinnedHttps { host, addrs } => {
                assert_eq!(host, "example.com");
                assert_eq!(addrs.len(), 2);
                assert!(addrs.iter().all(|a| a.port() == 443));
                assert!(addrs.iter().all(|a| !is_internal_ip(&a.ip())));
            }
            other => panic!("expected PinnedHttps, got {other:?}"),
        }
    }

    #[test]
    fn inspect_webhook_url_loopback_http_uses_shared_client() {
        let plan = inspect_webhook_url("http://127.0.0.1:8080/hook").unwrap();
        assert!(matches!(plan, WebhookConnectPlan::SharedClient));
    }

    #[tokio::test]
    async fn call_webhook_revalidates_and_rejects_internal_https() {
        let dir = make_data_root();
        let tool = PluginTool::new(
            PluginToolConfig {
                name: "ssrf_probe".to_string(),
                description: "request-time DNS revalidation".to_string(),
                side_effect: PluginSideEffect::Readonly,
                enabled: true,
                invocation: PluginInvocation::Webhook {
                    // 字面量内网 HTTPS：注册路径本应拒绝；即使配置被塞入也要在 call 时拒绝。
                    url: "https://127.0.0.1/hook".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(2),
                },
            },
            crate::outbound::outbound_client(),
            dir.path().to_path_buf(),
        );
        let err = tool
            .call(serde_json::json!({"x": 1}), false)
            .await
            .expect_err("internal https must fail at request time");
        let msg = err.to_string();
        assert!(
            msg.contains("内网") || msg.contains("SSRF") || msg.contains("127.0.0.1"),
            "unexpected err: {msg}"
        );
    }

    #[test]
    fn validate_script_path_rejects_traversal() {
        let dir = make_data_root();
        assert!(validate_script_path(dir.path(), "../etc/passwd").is_err());
        assert!(validate_script_path(dir.path(), "subdir/../../etc/passwd").is_err());
        assert!(validate_script_path(dir.path(), "./../etc/passwd").is_err());
    }

    #[test]
    fn validate_script_path_rejects_absolute() {
        let dir = make_data_root();
        assert!(validate_script_path(dir.path(), "/etc/passwd").is_err());
        #[cfg(windows)]
        assert!(validate_script_path(dir.path(), "C:/Windows/System32/cmd.exe").is_err());
    }

    #[test]
    fn validate_script_path_rejects_null_byte() {
        let dir = make_data_root();
        assert!(validate_script_path(dir.path(), "evil\0name.sh").is_err());
    }

    #[test]
    fn validate_script_path_accepts_existing_file_in_plugins_dir() {
        let dir = make_data_root();
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let script = plugins_dir.join("hello.sh");
        std::fs::write(&script, "#!/bin/sh\necho '{}'\n").unwrap();
        assert!(validate_script_path(dir.path(), "hello.sh").is_ok());
        assert!(validate_script_path(dir.path(), "./hello.sh").is_ok());
    }

    #[test]
    fn validate_script_path_rejects_missing_file() {
        let dir = make_data_root();
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        assert!(validate_script_path(dir.path(), "missing.sh").is_err());
    }

    #[test]
    fn validate_script_path_rejects_directory() {
        let dir = make_data_root();
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir_all(plugins_dir.join("subdir")).unwrap();
        assert!(validate_script_path(dir.path(), "subdir").is_err());
    }

    #[test]
    fn plugin_side_effect_serializes_as_snake_case() {
        for (variant, expected) in &[
            (PluginSideEffect::Readonly, "\"readonly\""),
            (PluginSideEffect::Mutate, "\"mutate\""),
            (PluginSideEffect::Destructive, "\"destructive\""),
            (PluginSideEffect::Append, "\"append\""),
        ] {
            let serialized = serde_json::to_string(variant).unwrap();
            assert_eq!(&serialized, expected);
            let deserialized: PluginSideEffect = serde_json::from_str(&serialized).unwrap();
            assert_eq!(&deserialized, variant);
        }
    }

    #[test]
    fn plugin_invocation_effective_timeout_clamps_to_30s() {
        let webhook = PluginInvocation::Webhook {
            url: "https://1.1.1.1/hook".to_string(),
            headers: BTreeMap::new(),
            timeout_secs: Some(120),
        };
        assert_eq!(webhook.effective_timeout_secs(), 30);
        let webhook_default = PluginInvocation::Webhook {
            url: "https://example.com/hook".to_string(),
            headers: BTreeMap::new(),
            timeout_secs: None,
        };
        assert_eq!(webhook_default.effective_timeout_secs(), 30);
        let webhook_zero = PluginInvocation::Webhook {
            url: "https://example.com/hook".to_string(),
            headers: BTreeMap::new(),
            timeout_secs: Some(0),
        };
        assert_eq!(webhook_zero.effective_timeout_secs(), 1);
    }

    #[test]
    fn plugin_tool_config_validate_rejects_long_description() {
        let dir = make_data_root();
        let long_desc = "x".repeat(513);
        let config = PluginToolConfig {
            name: "ok_name".to_string(),
            description: long_desc,
            side_effect: PluginSideEffect::Readonly,
            enabled: true,
            invocation: PluginInvocation::Webhook {
                url: "https://example.com/hook".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: None,
            },
        };
        assert!(config.validate(dir.path()).is_err());
    }

    #[test]
    fn plugin_tool_config_validate_rejects_too_many_args() {
        let dir = make_data_root();
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let script = plugins_dir.join("hello.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        let args: Vec<String> = (0..17).map(|i| format!("arg{i}")).collect();
        let config = PluginToolConfig {
            name: "ok_name".to_string(),
            description: "test".to_string(),
            side_effect: PluginSideEffect::Readonly,
            enabled: true,
            invocation: PluginInvocation::Script {
                relative_path: "hello.sh".to_string(),
                args,
                timeout_secs: None,
            },
        };
        let err = config.validate(dir.path()).unwrap_err();
        assert!(err.contains("args 过多"), "got: {err}");
    }

    #[test]
    fn save_and_load_plugin_tools_roundtrip() {
        let dir = make_data_root();
        let tools = vec![
            PluginToolConfig {
                name: "weather_lookup".to_string(),
                description: "Look up weather by city".to_string(),
                side_effect: PluginSideEffect::Readonly,
                enabled: true,
                invocation: PluginInvocation::Webhook {
                    url: "https://example.com/weather".to_string(),
                    headers: {
                        let mut m = BTreeMap::new();
                        m.insert("X-Custom".to_string(), "value".to_string());
                        m.insert("Authorization".to_string(), "Bearer secret".to_string());
                        m
                    },
                    timeout_secs: Some(15),
                },
            },
            PluginToolConfig {
                name: "local_calc".to_string(),
                description: "Local calculator".to_string(),
                side_effect: PluginSideEffect::Readonly,
                enabled: false,
                invocation: PluginInvocation::Webhook {
                    url: "http://localhost:8080/calc".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: None,
                },
            },
        ];
        save_plugin_tools(dir.path(), &tools).unwrap();
        let loaded = load_plugin_tools(dir.path()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "weather_lookup");
        // headers 在加载时合并回来。
        if let PluginInvocation::Webhook { headers, .. } = &loaded[0].invocation {
            assert_eq!(headers.len(), 2);
            assert_eq!(
                headers.get("Authorization"),
                Some(&"Bearer secret".to_string())
            );
        } else {
            panic!("expected webhook invocation");
        }
        assert_eq!(loaded[1].name, "local_calc");
    }

    #[test]
    fn load_plugin_tools_returns_empty_when_file_missing() {
        let dir = make_data_root();
        let loaded = load_plugin_tools(dir.path()).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn save_plugin_tools_rejects_invalid_config() {
        let dir = make_data_root();
        let bad = PluginToolConfig {
            name: "1invalid".to_string(), // 以数字开头
            description: "test".to_string(),
            side_effect: PluginSideEffect::Readonly,
            enabled: true,
            invocation: PluginInvocation::Webhook {
                url: "https://example.com/hook".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: None,
            },
        };
        assert!(save_plugin_tools(dir.path(), &[bad]).is_err());
    }

    #[test]
    fn protected_headers_are_rejected() {
        assert!(is_protected_header("Host"));
        assert!(is_protected_header("host"));
        assert!(is_protected_header("HOST"));
        assert!(is_protected_header("Content-Length"));
        assert!(is_protected_header("Transfer-Encoding"));
        assert!(is_protected_header("Connection"));
        assert!(is_protected_header("Content-Encoding"));
        assert!(!is_protected_header("Authorization"));
        assert!(!is_protected_header("X-Custom"));
    }

    #[tokio::test]
    async fn webhook_non_success_error_omits_response_body() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const SECRET_BODY: &str = "unique-secret-webhook-body-8f3d2a";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(502).set_body_string(SECRET_BODY))
            .mount(&server)
            .await;

        let dir = make_data_root();
        let tool = PluginTool::new(
            PluginToolConfig {
                name: "private_tool".to_string(),
                description: "test webhook error redaction".to_string(),
                side_effect: PluginSideEffect::Readonly,
                enabled: true,
                invocation: PluginInvocation::Webhook {
                    url: format!("{}/hook", server.uri()),
                    headers: BTreeMap::new(),
                    timeout_secs: None,
                },
            },
            reqwest::Client::new(),
            dir.path().to_path_buf(),
        );

        let error = tool
            .call(serde_json::json!({"input": "test"}), false)
            .await
            .unwrap_err();
        let internal_message = error.to_string();
        let public_message = error.public_message();
        assert!(internal_message.contains("private_tool"));
        assert!(internal_message.contains("502"));
        assert!(!internal_message.contains(SECRET_BODY));
        assert!(!public_message.contains(SECRET_BODY));
    }

    #[tokio::test]
    async fn plugin_tool_call_webhook_rejects_protected_header() {
        let dir = make_data_root();
        let config = PluginToolConfig {
            name: "evil".to_string(),
            description: "test".to_string(),
            side_effect: PluginSideEffect::Readonly,
            enabled: true,
            invocation: PluginInvocation::Webhook {
                url: "https://1.1.1.1/hook".to_string(),
                headers: {
                    let mut m = BTreeMap::new();
                    m.insert("Host".to_string(), "evil.com".to_string());
                    m
                },
                timeout_secs: None,
            },
        };
        let tool = PluginTool::new(config, reqwest::Client::new(), dir.path().to_path_buf());
        let result = tool.call(serde_json::json!({"x": 1}), false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("保护"), "got: {err}");
    }
}
