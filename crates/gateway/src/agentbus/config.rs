//! Adapter configuration: the intent→route→scope mapping table.
//!
//! This is **adapter-local** state. It does NOT touch [`airp_gateway::config`]
//! (the core declarative config). The mapping lives here because it is purely
//! a State-Protocol concern: how an `intent.name` becomes a `RouteRule.path`
//! and which state scope its result lands in.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Default intent-name prefix to strip when forming a route path.
/// `chat.send` -> route path `/v1/chat.send`.
pub const DEFAULT_ROUTE_PREFIX: &str = "/v1/";

/// One row of the intent→scope default mapping.
///
/// Required only when an intent arrives without a `source` field (the UI did
/// not tell us which widget instance emitted it). When the intent *does* carry
/// `source`, that wins and this table is not consulted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentScopeFallback {
    /// Intent name, e.g. `chat.send`.
    pub intent: String,
    /// Default scope to patch, e.g. `w-chat`.
    pub scope: String,
}

/// Adapter-level config, built by the example/host and passed into
/// [`super::AgentBusAdapter::new`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdapterConfig {
    /// Default scope to patch when an intent has no `source` and no entry in
    /// `fallback_scopes`. If `None`, the envelope is rejected with an error.
    pub default_scope: Option<String>,
    /// Per-intent default scopes. keyed by intent name.
    #[serde(default)]
    pub fallback_scopes: Vec<IntentScopeFallback>,
    /// Prefix prepended to the intent name to form the route path.
    /// Default `/v1/` (so `chat.send` -> `/v1/chat.send`).
    #[serde(default = "default_route_prefix")]
    pub route_prefix: String,
    /// Initial blueprint to push on `hello`. Omit => empty blueprint.
    #[serde(default)]
    pub initial_blueprint: Option<super::envelope::Blueprint>,
    /// Initial widget manifests to push on `hello`.
    #[serde(default)]
    pub initial_manifests: Vec<super::envelope::Manifest>,
    /// Initial per-scope state to push on `hello`, as `{ scope: value }`.
    /// Each becomes a `state op: set` envelope.
    #[serde(default)]
    pub initial_state: HashMap<String, serde_json::Value>,
}

fn default_route_prefix() -> String {
    DEFAULT_ROUTE_PREFIX.to_string()
}

impl AdapterConfig {
    /// Resolve the scope for an intent, given whatever `source` the UI sent.
    /// `source` on the intent always wins; otherwise the fallback table; last
    /// the global default; last `None` (handler rejects).
    pub fn scope_for(&self, intent: &str, source: Option<&str>) -> Option<String> {
        if let Some(s) = source {
            return Some(s.to_string());
        }
        if let Some(row) = self.fallback_scopes.iter().find(|r| r.intent == intent) {
            return Some(row.scope.clone());
        }
        self.default_scope.clone()
    }

    /// Map an intent name to the route path the core bridge will match.
    pub fn route_path(&self, intent: &str) -> String {
        format!("{}{}", self.route_prefix, intent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AdapterConfig {
        AdapterConfig {
            default_scope: Some("w-default".into()),
            fallback_scopes: vec![IntentScopeFallback {
                intent: "chat.send".into(),
                scope: "w-chat".into(),
            }],
            route_prefix: default_route_prefix(),
            ..Default::default()
        }
    }

    #[test]
    fn source_wins_over_fallback() {
        let c = cfg();
        assert_eq!(
            c.scope_for("chat.send", Some("w-other")),
            Some("w-other".into())
        );
    }

    #[test]
    fn fallback_table_used_when_no_source() {
        let c = cfg();
        assert_eq!(c.scope_for("chat.send", None), Some("w-chat".into()));
    }

    #[test]
    fn global_default_used_when_no_source_and_no_fallback() {
        let c = cfg();
        assert_eq!(
            c.scope_for("unknown.intent", None),
            Some("w-default".into())
        );
    }

    #[test]
    fn none_when_no_source_no_fallback_no_default() {
        let c = AdapterConfig::default();
        assert_eq!(c.scope_for("x", None), None);
    }

    #[test]
    fn route_path_uses_prefix() {
        let c = cfg();
        assert_eq!(c.route_path("chat.send"), "/v1/chat.send");
    }
}
