//! Layered gateway configuration: `default -> config file (TOML) -> env`.
//!
//! Mirrors the merge philosophy of AIRP-Core's `config.rs` but is fully
//! domain-agnostic: the gateway knows about *bind address, auth, rate limits,
//! upstream MCP servers, and route mappings* — nothing about characters,
//! presets, or any roleplay concept.

use serde::Deserialize;
use std::path::Path;

use crate::error::{GatewayError, Result};

/// Top-level gateway configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    /// Socket address the frontend-facing server binds to, e.g. `127.0.0.1:8080`.
    pub bind: String,
    /// Optional bearer token required on `/` API routes. `None` = open.
    pub access_key: Option<String>,
    pub rate_limit: RateLimitConfig,
    pub cors: CorsConfig,
    /// MCP servers this gateway can forward to, keyed by `name`.
    pub upstreams: Vec<UpstreamConfig>,
    /// Declarative frontend-path -> MCP-operation mappings.
    pub routes: Vec<RouteRule>,
    /// Allowlist of permitted stdio `command`s (defense-in-depth against a
    /// config that spawns arbitrary programs). **Empty = allow any** (config is
    /// operator-trusted). Non-empty = a stdio upstream's `command` must match an
    /// entry by full string *or* file name, else [`GatewayConfig::validate`]
    /// fails. Has no effect on HTTP upstreams.
    pub allowed_commands: Vec<String>,
    /// Maximum size of an inbound frontend request body, in bytes. Default 1 MiB.
    /// Protects against unbounded memory use from large requests.
    pub max_request_bytes: usize,
    /// Maximum size of an upstream MCP response body, in bytes. Default 10 MiB.
    /// Protects against a malicious or buggy upstream sending an unbounded response.
    pub max_response_bytes: usize,
    /// Timeout in seconds for upstream MCP requests (connect + read). Default 30s.
    /// `McpClient::invoke` wraps the transport call in `tokio::time::timeout`.
    /// Set to 0 to disable (not recommended in production).
    pub upstream_timeout_secs: u64,
    /// When true, HTTP upstream URLs are checked against private/loopback/link-local
    /// address ranges at `validate()` time (SSRF defense). Default true (defense on).
    /// Set false only if you intentionally target an internal MCP server and
    /// understand the risk.
    pub block_private_upstream_urls: bool,
    /// When `allowed_commands` is non-empty, stdio upstreams with non-empty `args`
    /// are rejected unless this is set to true. Default false (defense on).
    /// Prevents `command = "sh", args = ["-c", "rm -rf /"]` style attacks.
    pub allow_arbitrary_args: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".to_string(),
            access_key: None,
            rate_limit: RateLimitConfig::default(),
            cors: CorsConfig::default(),
            upstreams: Vec::new(),
            routes: Vec::new(),
            allowed_commands: Vec::new(),
            max_request_bytes: 1024 * 1024,
            max_response_bytes: 10 * 1024 * 1024,
            upstream_timeout_secs: 30,
            block_private_upstream_urls: true,
            allow_arbitrary_args: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    pub enabled: bool,
    /// Sustained requests per second per client key (IP).
    pub per_second: u64,
    /// Burst allowance above the sustained rate.
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            per_second: 10,
            burst: 20,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CorsConfig {
    /// Allow any origin/method/header (permissive). Convenient for local dev.
    pub allow_any: bool,
    /// Explicit origins when `allow_any` is false.
    pub allow_origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allow_any: true,
            allow_origins: Vec::new(),
        }
    }
}

/// One upstream MCP server.
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamConfig {
    /// Unique name referenced by [`RouteRule::upstream`].
    pub name: String,
    #[serde(flatten)]
    pub transport: TransportConfig,
}

/// How to reach an upstream MCP server. Both stdio and HTTP are first-class.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum TransportConfig {
    /// Launch the MCP server as a child process; talk JSON-RPC over stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Connect to a running MCP server over streamable HTTP.
    Http {
        url: String,
        #[serde(default)]
        auth_token: Option<String>,
    },
}

/// A declarative mapping from a frontend request to an MCP operation.
#[derive(Clone, Debug, Deserialize)]
pub struct RouteRule {
    /// Frontend path, e.g. `/v1/chat/completions`.
    pub path: String,
    #[serde(default = "default_method")]
    pub method: String,
    /// Name of the [`UpstreamConfig`] to dispatch to.
    pub upstream: String,
    pub target: RouteTarget,
}

fn default_method() -> String {
    "POST".to_string()
}

/// True if `command` matches an allowlist entry by full string or file name
/// (so `airp-mcp` matches `/usr/bin/airp-mcp`).
fn command_allowed(command: &str, allow: &[String]) -> bool {
    let base = std::path::Path::new(command)
        .file_name()
        .and_then(|s| s.to_str());
    allow
        .iter()
        .any(|a| a == command || Some(a.as_str()) == base)
}

/// What MCP primitive a route invokes.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RouteTarget {
    /// Call an MCP tool. The request body is passed through as the tool args.
    Tool {
        name: String,
        /// Whether the response should be streamed back to the frontend as SSE.
        #[serde(default)]
        stream: bool,
    },
    /// Read an MCP resource by URI.
    Resource { uri: String },
}

impl GatewayConfig {
    /// Build config by layering: defaults, then optional TOML file, then env.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let mut cfg = match path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p)?;
                toml::from_str(&text).map_err(|e| GatewayError::Config(e.to_string()))?
            }
            _ => GatewayConfig::default(),
        };
        cfg.apply_env();
        Ok(cfg)
    }

    /// Validate security-relevant invariants. Call before building transports.
    ///
    /// Currently:
    /// - if [`allowed_commands`](Self::allowed_commands) is non-empty, every
    ///   stdio upstream's `command` must be allowed;
    /// - if `allow_arbitrary_args` is false and `allowed_commands` is non-empty,
    ///   stdio upstreams with non-empty `args` are rejected (defense against
    ///   `command = "sh", args = ["-c", "..."]` style attacks);
    /// - if [`block_private_upstream_urls`](Self::block_private_upstream_urls)
    ///   is true, every HTTP upstream URL must not resolve to a private /
    ///   loopback / link-local address (SSRF defense).
    pub fn validate(&self) -> Result<()> {
        // stdio command allowlist.
        if !self.allowed_commands.is_empty() {
            for up in &self.upstreams {
                if let TransportConfig::Stdio { command, args, .. } = &up.transport {
                    if !command_allowed(command, &self.allowed_commands) {
                        return Err(GatewayError::Config(format!(
                            "stdio command `{command}` (upstream `{}`) is not in allowed_commands",
                            up.name
                        )));
                    }
                    // Reject arbitrary args unless the operator explicitly opted in.
                    // Defense against `command = "sh", args = ["-c", "rm -rf /"]`.
                    if !args.is_empty() && !self.allow_arbitrary_args {
                        return Err(GatewayError::Config(format!(
                            "stdio upstream `{}` uses args but allow_arbitrary_args=false \
                             (allowed_commands is active). Set allow_arbitrary_args=true to permit.",
                            up.name
                        )));
                    }
                }
            }
        }

        // SSRF defense: reject HTTP upstreams targeting private/link-local/
        // reserved ranges. Loopback is allowed (the common local-MCP case).
        if self.block_private_upstream_urls {
            for up in &self.upstreams {
                if let TransportConfig::Http { url, .. } = &up.transport {
                    if let Some(reason) = url_blocked_target_reason(url) {
                        return Err(GatewayError::Config(format!(
                            "upstream `{}` URL `{url}` rejected (SSRF defense: {reason}). \
                             Set block_private_upstream_urls=false to allow.",
                            up.name
                        )));
                    }
                }
            }
        }

        // Route integrity: every route's upstream must exist in the upstreams list.
        let upstream_names: std::collections::HashSet<&str> =
            self.upstreams.iter().map(|u| u.name.as_str()).collect();
        for route in &self.routes {
            if !upstream_names.contains(route.upstream.as_str()) {
                return Err(GatewayError::Config(format!(
                    "route `{} {}` references unknown upstream `{}`",
                    route.method, route.path, route.upstream
                )));
            }
        }

        Ok(())
    }

    /// Apply `AIRP_GW_*` environment overrides (top-level scalars only).
    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("AIRP_GW_BIND") {
            if !v.is_empty() {
                self.bind = v;
            }
        }
        if let Ok(v) = std::env::var("AIRP_GW_ACCESS_KEY") {
            self.access_key = if v.is_empty() { None } else { Some(v) };
        }
        if let Ok(v) = std::env::var("AIRP_GW_MAX_REQUEST_BYTES") {
            if let Ok(n) = v.parse::<usize>() {
                if n > 0 {
                    self.max_request_bytes = n;
                }
            }
        }
        if let Ok(v) = std::env::var("AIRP_GW_MAX_RESPONSE_BYTES") {
            if let Ok(n) = v.parse::<usize>() {
                if n > 0 {
                    self.max_response_bytes = n;
                }
            }
        }
        if let Ok(v) = std::env::var("AIRP_GW_UPSTREAM_TIMEOUT_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.upstream_timeout_secs = n;
            }
        }
    }
}

/// Inspect an upstream URL for SSRF risk. Returns `Some(reason)` if the URL's
/// host is a private / link-local / unspecified / reserved address. Returns
/// `None` if the URL is unparseable or appears safe.
///
/// **Loopback (`127.0.0.0/8`, `::1`, `localhost`) is intentionally allowed.**
/// A local-first gateway's most common HTTP upstream is an MCP server on
/// localhost; blocking it by default is pure friction. The high-value SSRF
/// targets — internal LAN, cloud metadata at `169.254.169.254`, etc. — are
/// still blocked by default. Restrict loopback yourself via the OS/firewall if
/// your threat model requires it.
///
/// NOTE: checks the host literally; no DNS resolution (racy/ToCTOU). A hostname
/// that resolves to an internal address is not caught — set
/// `block_private_upstream_urls=false` only if you intend internal targets, and
/// rely on network policy for the rest.
fn url_blocked_target_reason(url: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    // Loopback hostnames are allowed (see doc above).
    if matches!(
        host,
        "localhost" | "localhost.localdomain" | "ip6-localhost"
    ) {
        return None;
    }
    // Only literal IPs are classified; loopback is allowed.
    let ip = host.parse::<std::net::IpAddr>().ok()?;
    if ip.is_loopback() {
        return None;
    }
    if ip.is_unspecified() {
        return Some("unspecified address");
    }
    match ip {
        std::net::IpAddr::V4(v4) if v4.is_link_local() => return Some("link-local address"),
        std::net::IpAddr::V6(v6) if v6.is_unicast_link_local() => {
            return Some("link-local address")
        }
        _ => {}
    }
    // Private / shared / benchmarking ranges per RFC 1918 + RFC 6890.
    if is_private_or_reserved(ip) {
        return Some("private/reserved address");
    }
    None
}

/// True for RFC 1918 private ranges + RFC 6890 reserved/shared/benchmarking,
/// which have no business being an upstream for a local gateway.
fn is_private_or_reserved(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_broadcast() || v4.is_documentation() || {
                // 0.0.0.0/8, 100.64.0.0/10 (CGNAT), 192.0.0.0/24, 192.0.2.0/24,
                // 198.18.0.0/15, 240.0.0.0/4 — cover via std lib where available.
                let o = v4.octets();
                o[0] == 0
                    || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64.0.0/10 CGNAT
                    || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
                    || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // 198.18.0.0/15 benchmarking
                    || o[0] >= 240 // 240.0.0.0/4 reserved
            }
        }
        std::net::IpAddr::V6(v6) => {
            // Unique local fc00::/7, site-local fec0::/10 (deprecated but reserved).
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfec0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_up(name: &str, command: &str) -> UpstreamConfig {
        UpstreamConfig {
            name: name.into(),
            transport: TransportConfig::Stdio {
                command: command.into(),
                args: vec![],
                cwd: None,
            },
        }
    }

    fn cfg_with(upstreams: Vec<UpstreamConfig>, allowed: Vec<&str>) -> GatewayConfig {
        GatewayConfig {
            upstreams,
            allowed_commands: allowed.into_iter().map(String::from).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_allowlist_permits_any_command() {
        let cfg = cfg_with(vec![stdio_up("a", "/bin/bash")], vec![]);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn allowlist_rejects_unlisted_command() {
        let cfg = cfg_with(vec![stdio_up("a", "/bin/bash")], vec!["airp-mcp"]);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn allowlist_matches_by_basename() {
        let cfg = cfg_with(
            vec![stdio_up("a", "/usr/local/bin/airp-mcp")],
            vec!["airp-mcp"],
        );
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn allowlist_ignores_http_upstreams() {
        // SSRF defense is on by default; this URL points at a public host so
        // it stays compatible with the allowlist-only intent of the test.
        let http = UpstreamConfig {
            name: "h".into(),
            transport: TransportConfig::Http {
                url: "https://example.com/mcp/v1".into(),
                auth_token: None,
            },
        };
        let cfg = cfg_with(vec![http], vec!["airp-mcp"]);
        assert!(cfg.validate().is_ok());
    }

    fn http_up(name: &str, url: &str) -> UpstreamConfig {
        UpstreamConfig {
            name: name.into(),
            transport: TransportConfig::Http {
                url: url.into(),
                auth_token: None,
            },
        }
    }

    fn cfg_with_http(upstreams: Vec<UpstreamConfig>, block_private: bool) -> GatewayConfig {
        GatewayConfig {
            upstreams,
            block_private_upstream_urls: block_private,
            ..Default::default()
        }
    }

    #[test]
    fn ssrf_allows_loopback_ip_by_default() {
        // Loopback is the common local-MCP upstream; allowed even with the
        // SSRF guard on.
        let cfg = cfg_with_http(vec![http_up("a", "http://127.0.0.1:3000/mcp/v1")], true);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn ssrf_allows_localhost_hostname_by_default() {
        let cfg = cfg_with_http(vec![http_up("a", "http://localhost:3000/mcp/v1")], true);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn ssrf_rejects_private_v4() {
        let cfg = cfg_with_http(vec![http_up("a", "http://10.0.0.5/mcp/v1")], true);
        assert!(cfg.validate().is_err());
        let cfg = cfg_with_http(vec![http_up("a", "http://192.168.1.1/mcp/v1")], true);
        assert!(cfg.validate().is_err());
        let cfg = cfg_with_http(vec![http_up("a", "http://172.16.0.1/mcp/v1")], true);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn ssrf_rejects_link_local() {
        let cfg = cfg_with_http(vec![http_up("a", "http://169.254.169.254/mcp/v1")], true);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn ssrf_rejects_unspecified() {
        let cfg = cfg_with_http(vec![http_up("a", "http://0.0.0.0:3000/mcp/v1")], true);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn ssrf_allows_public_host() {
        let cfg = cfg_with_http(vec![http_up("a", "https://example.com/mcp/v1")], true);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn ssrf_can_be_disabled_for_local_upstream() {
        let cfg = cfg_with_http(vec![http_up("a", "http://127.0.0.1:3000/mcp/v1")], false);
        assert!(cfg.validate().is_ok());
    }

    // --- args validation tests ---

    fn stdio_up_with_args(name: &str, command: &str, args: Vec<&str>) -> UpstreamConfig {
        UpstreamConfig {
            name: name.into(),
            transport: TransportConfig::Stdio {
                command: command.into(),
                args: args.into_iter().map(String::from).collect(),
                cwd: None,
            },
        }
    }

    #[test]
    fn args_rejected_when_allowlist_active_and_arbitrary_args_false() {
        let cfg = GatewayConfig {
            upstreams: vec![stdio_up_with_args(
                "a",
                "airp-mcp",
                vec!["mcp", "--data-dir", "/tmp"],
            )],
            allowed_commands: vec!["airp-mcp".into()],
            allow_arbitrary_args: false,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn args_allowed_when_arbitrary_args_opt_in() {
        let cfg = GatewayConfig {
            upstreams: vec![stdio_up_with_args(
                "a",
                "airp-mcp",
                vec!["mcp", "--data-dir", "/tmp"],
            )],
            allowed_commands: vec!["airp-mcp".into()],
            allow_arbitrary_args: true,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn args_not_checked_when_allowlist_empty() {
        // Empty allowed_commands means no command/args validation at all.
        let cfg = GatewayConfig {
            upstreams: vec![stdio_up_with_args("a", "/bin/bash", vec!["-c", "echo hi"])],
            allowed_commands: vec![],
            allow_arbitrary_args: false,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    // --- defaults tests ---

    #[test]
    fn default_timeout_and_limits() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.upstream_timeout_secs, 30);
        assert_eq!(cfg.max_request_bytes, 1024 * 1024);
        assert_eq!(cfg.max_response_bytes, 10 * 1024 * 1024);
        assert!(cfg.block_private_upstream_urls);
        assert!(!cfg.allow_arbitrary_args);
    }

    #[test]
    fn route_referencing_unknown_upstream_is_rejected() {
        let cfg = GatewayConfig {
            upstreams: vec![UpstreamConfig {
                name: "real".to_string(),
                transport: TransportConfig::Http {
                    url: "https://example.com/mcp".to_string(),
                    auth_token: None,
                },
            }],
            routes: vec![RouteRule {
                path: "/v1/test".to_string(),
                method: "POST".to_string(),
                upstream: "phantom".to_string(),
                target: RouteTarget::Tool {
                    name: "echo".to_string(),
                    stream: false,
                },
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(
            err.contains("phantom"),
            "error should mention the unknown upstream name: {err}"
        );
    }
}
