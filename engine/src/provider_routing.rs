//! Phase 5.1: Multi-Provider Routing.
//!
//! 从单一全局 provider 升级为可声明的 provider 数组 + 路由策略。
//! `RouteContext` 描述调用来源，`ProviderRouter::resolve` 查表命中 `ProviderEntry`。
//!
//! ## 持久化
//! - `data/providers.json` — provider 条目数组 + 路由策略表（**不含 api_key**）。
//! - `data/provider_keys.json` — `HashMap<provider_name, api_key>`，与 entries 分离存储，
//!   避免 `data/providers.json` 被误分享时泄露密钥。
//!
//! 旧版单 provider 配置（`data/settings.json` + `data/secrets.json`）保持不变；
//! 当 `data/providers.json` 缺省或 entries 为空时，daemon 仍走 legacy 单 provider 路径。

use crate::adapter::{BackendEngine, Provider};
use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 路由判定上下文。命中优先级：character_id > scene_role > task_kind。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteContext {
    pub character_id: Option<String>,
    pub scene_role: Option<String>,
    pub task_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderEntry {
    pub name: String,
    pub endpoint: String,
    #[serde(skip)]
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub engine: BackendEngine,
    #[serde(default)]
    pub is_default: bool,
}

impl ProviderEntry {
    pub fn to_provider_config(&self) -> crate::adapter::ProviderConfig {
        crate::adapter::ProviderConfig {
            provider: Provider::OpenAI,
            endpoint: self.endpoint.clone(),
            api_key: self.api_key.clone(),
        }
    }

    pub fn to_generation_params(&self) -> crate::adapter::GenerationParams {
        crate::adapter::GenerationParams {
            model: self.model.clone(),
            temperature: None,
            max_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRouting {
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub by_character: HashMap<String, String>,
    #[serde(default)]
    pub by_scene_role: HashMap<String, String>,
    #[serde(default)]
    pub by_task_kind: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ProviderRouter {
    entries: Vec<ProviderEntry>,
    by_name: HashMap<String, usize>,
    routing: ProviderRouting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProvider<'a> {
    pub entry: &'a ProviderEntry,
    pub matched_rule: MatchedRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedRule {
    Character,
    SceneRole,
    TaskKind,
    Default,
    FirstDefault,
}

impl ProviderRouter {
    pub fn new(entries: Vec<ProviderEntry>, routing: ProviderRouting) -> Self {
        let by_name = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), i))
            .collect();
        Self {
            entries,
            by_name,
            routing,
        }
    }

    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            routing: ProviderRouting::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[ProviderEntry] {
        &self.entries
    }

    pub fn routing(&self) -> &ProviderRouting {
        &self.routing
    }

    pub fn get(&self, name: &str) -> Option<&ProviderEntry> {
        self.by_name.get(name).map(|i| &self.entries[*i])
    }

    pub fn resolve(&self, ctx: &RouteContext) -> Option<ResolvedProvider<'_>> {
        if self.entries.is_empty() {
            return None;
        }
        if let Some(cid) = ctx.character_id.as_deref() {
            if let Some(name) = self.routing.by_character.get(cid) {
                if let Some(idx) = self.by_name.get(name) {
                    return Some(ResolvedProvider {
                        entry: &self.entries[*idx],
                        matched_rule: MatchedRule::Character,
                    });
                }
            }
        }
        if let Some(role) = ctx.scene_role.as_deref() {
            if let Some(name) = self.routing.by_scene_role.get(role) {
                if let Some(idx) = self.by_name.get(name) {
                    return Some(ResolvedProvider {
                        entry: &self.entries[*idx],
                        matched_rule: MatchedRule::SceneRole,
                    });
                }
            }
        }
        if let Some(task) = ctx.task_kind.as_deref() {
            if let Some(name) = self.routing.by_task_kind.get(task) {
                if let Some(idx) = self.by_name.get(name) {
                    return Some(ResolvedProvider {
                        entry: &self.entries[*idx],
                        matched_rule: MatchedRule::TaskKind,
                    });
                }
            }
        }
        if let Some(name) = self.routing.default_provider.as_deref() {
            if let Some(idx) = self.by_name.get(name) {
                return Some(ResolvedProvider {
                    entry: &self.entries[*idx],
                    matched_rule: MatchedRule::Default,
                });
            }
        }
        if let Some(idx) = self.entries.iter().position(|e| e.is_default) {
            return Some(ResolvedProvider {
                entry: &self.entries[idx],
                matched_rule: MatchedRule::FirstDefault,
            });
        }
        None
    }
}

impl Default for ProviderRouter {
    fn default() -> Self {
        Self::empty()
    }
}

pub fn validate_provider_config(
    entries: &[ProviderEntry],
    routing: &ProviderRouting,
) -> Result<(), String> {
    let mut seen_names = std::collections::HashSet::new();
    for entry in entries {
        if entry.name.is_empty() {
            return Err("ProviderEntry.name 不能为空字符串".to_string());
        }
        if !seen_names.insert(entry.name.as_str()) {
            return Err(format!("ProviderEntry.name 重复: {}", entry.name));
        }
        if entry.endpoint.is_empty() {
            return Err(format!(
                "ProviderEntry[name={}] endpoint 不能为空",
                entry.name
            ));
        }
        if entry.model.is_empty() {
            return Err(format!(
                "ProviderEntry[name={}] model 不能为空",
                entry.name
            ));
        }
    }
    if !entries.is_empty() {
        let has_default = entries.iter().any(|e| e.is_default);
        if !has_default {
            return Err(
                "providers 非空时至少必须有一个 entry 的 is_default = true".to_string(),
            );
        }
    }
    if let Some(name) = routing.default_provider.as_deref() {
        if !entries.iter().any(|e| e.name == name) {
            return Err(format!(
                "routing.default_provider 指向不存在的 provider: {}",
                name
            ));
        }
    }
    for (key, name) in &routing.by_character {
        if !entries.iter().any(|e| e.name == *name) {
            return Err(format!(
                "routing.by_character[{}] 指向不存在的 provider: {}",
                key, name
            ));
        }
    }
    for (key, name) in &routing.by_scene_role {
        if !entries.iter().any(|e| e.name == *name) {
            return Err(format!(
                "routing.by_scene_role[{}] 指向不存在的 provider: {}",
                key, name
            ));
        }
    }
    for (key, name) in &routing.by_task_kind {
        if !entries.iter().any(|e| e.name == *name) {
            return Err(format!(
                "routing.by_task_kind[{}] 指向不存在的 provider: {}",
                key, name
            ));
        }
    }
    Ok(())
}

// ── 持久化 ─────────────────────────────────────────────────────────────────

const PROVIDERS_FILE_NAME: &str = "providers.json";
const PROVIDER_KEYS_FILE_NAME: &str = "provider_keys.json";
const PROVIDERS_FILE_VERSION: u32 = 1;

/// `data/providers.json` 的盘上 schema。`api_key` 永远不写入此文件。
#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderRoutingFile {
    pub version: u32,
    pub entries: Vec<ProviderEntry>,
    #[serde(default)]
    pub routing: ProviderRouting,
}

/// `data/provider_keys.json` 的盘上 schema。
/// 单独存储以避免 `data/providers.json` 被分享时泄露密钥。
#[derive(Debug, Serialize, Deserialize)]
struct ProviderKeyFile {
    version: u32,
    /// provider name → api_key。空字符串视为未设置。
    keys: HashMap<String, String>,
}

const PROVIDER_KEY_FILE_VERSION: u32 = 1;

/// `data/providers.json` 路径。
pub fn providers_file_path(data_root: &Path) -> std::path::PathBuf {
    data_root.join(PROVIDERS_FILE_NAME)
}

/// `data/provider_keys.json` 路径（不对外暴露，仅 crate 内部使用）。
pub(crate) fn provider_keys_file_path(data_root: &Path) -> std::path::PathBuf {
    data_root.join(PROVIDER_KEYS_FILE_NAME)
}

/// 从 `data/providers.json` 加载 provider 数组与路由策略。
///
/// - 文件不存在 → 返回空向量 + 默认 routing（视为未启用多 provider）。
/// - 文件存在但解析失败 → 返回错误。
/// - 文件存在且 `entries` 为空 → 返回空向量 + 默认 routing。
///
/// `api_key` 字段不在 `providers.json` 中持久化，调用方需通过
/// [`load_provider_keys`] 单独加载并合并。
pub fn load_provider_routing(
    data_root: &Path,
) -> Result<(Vec<ProviderEntry>, ProviderRouting), AirpError> {
    let path = providers_file_path(data_root);
    if !path.exists() {
        return Ok((Vec::new(), ProviderRouting::default()));
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| AirpError::Internal(format!("读取 providers.json 失败: {e}")))?;
    let file: ProviderRoutingFile = serde_json::from_str(&raw)
        .map_err(|e| AirpError::Internal(format!("解析 providers.json 失败: {e}")))?;
    if file.version != PROVIDERS_FILE_VERSION {
        return Err(AirpError::Internal(format!(
            "providers.json 版本不匹配：期望 {}，实际 {}",
            PROVIDERS_FILE_VERSION, file.version
        )));
    }
    // 盘上文件可能包含旧版 entries，确保 api_key 为 None（serde skip 已保证，但兜底）。
    let entries = file
        .entries
        .into_iter()
        .map(|mut e| {
            e.api_key = None;
            e
        })
        .collect();
    Ok((entries, file.routing))
}

/// 将 provider 数组与路由策略写入 `data/providers.json`。
///
/// **不变量**：调用方必须保证 `entries` 中无 `api_key`（`ProviderEntry` 的 `api_key`
/// 字段已 `#[serde(skip)]`，序列化时自动剥离）。本函数额外做一次防御性清空。
pub fn save_provider_routing(
    data_root: &Path,
    entries: &[ProviderEntry],
    routing: &ProviderRouting,
) -> Result<(), AirpError> {
    let sanitized: Vec<ProviderEntry> = entries
        .iter()
        .map(|e| {
            let mut clone = e.clone();
            clone.api_key = None;
            clone
        })
        .collect();
    let file = ProviderRoutingFile {
        version: PROVIDERS_FILE_VERSION,
        entries: sanitized,
        routing: routing.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    let path = providers_file_path(data_root);
    crate::data_dir::replace_file(&path, &bytes)
}

/// 从 `data/provider_keys.json` 加载所有 provider 的 api_key。
///
/// 文件不存在或未启用持久化时返回空 HashMap。
pub fn load_provider_keys(
    data_root: &Path,
) -> Result<HashMap<String, String>, AirpError> {
    let path = provider_keys_file_path(data_root);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| AirpError::Internal(format!("读取 provider_keys.json 失败: {e}")))?;
    let file: ProviderKeyFile = serde_json::from_str(&raw)
        .map_err(|e| AirpError::Internal(format!("解析 provider_keys.json 失败: {e}")))?;
    if file.version != PROVIDER_KEY_FILE_VERSION {
        return Err(AirpError::Internal(format!(
            "provider_keys.json 版本不匹配：期望 {}，实际 {}",
            PROVIDER_KEY_FILE_VERSION, file.version
        )));
    }
    // 过滤空字符串，规范化"未设置"语义。
    Ok(file.keys.into_iter().filter(|(_, v)| !v.is_empty()).collect())
}

/// 将 provider name → api_key 映射写入 `data/provider_keys.json`。
///
/// 空字符串会被过滤掉（视为未设置）。
pub fn save_provider_keys(
    data_root: &Path,
    keys: &HashMap<String, String>,
) -> Result<(), AirpError> {
    let sanitized: HashMap<String, String> = keys
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let file = ProviderKeyFile {
        version: PROVIDER_KEY_FILE_VERSION,
        keys: sanitized,
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    let path = provider_keys_file_path(data_root);
    crate::data_dir::replace_file(&path, &bytes)
}

/// 一次性加载 entries + routing + api_keys，构造 ready-to-use `ProviderRouter`。
///
/// - `data/providers.json` 不存在或为空 → 返回 [`ProviderRouter::empty`]。
/// - 否则加载 entries、合并 keys、validate 后构造 router。
pub fn load_provider_router(data_root: &Path) -> Result<ProviderRouter, AirpError> {
    let (mut entries, routing) = load_provider_routing(data_root)?;
    if entries.is_empty() {
        return Ok(ProviderRouter::empty());
    }
    let keys = load_provider_keys(data_root)?;
    for entry in entries.iter_mut() {
        if let Some(key) = keys.get(&entry.name) {
            entry.api_key = Some(key.clone());
        }
    }
    validate_provider_config(&entries, &routing)
        .map_err(|e| AirpError::BadRequest(format!("providers.json 不合法: {e}")))?;
    Ok(ProviderRouter::new(entries, routing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::BackendEngine;

    fn entry(name: &str, endpoint: &str, model: &str, is_default: bool) -> ProviderEntry {
        ProviderEntry {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            api_key: None,
            model: model.to_string(),
            engine: BackendEngine::Direct,
            is_default,
        }
    }

    fn sample_entries() -> Vec<ProviderEntry> {
        vec![
            entry("openai", "https://api.openai.com/v1/chat/completions", "gpt-4o", true),
            entry("deepseek", "https://api.deepseek.com/v1/chat/completions", "deepseek-chat", false),
            entry("local", "http://127.0.0.1:11434/v1/chat/completions", "llama3", false),
        ]
    }

    #[test]
    fn empty_router_resolves_to_none() {
        let router = ProviderRouter::empty();
        assert!(router.is_empty());
        let ctx = RouteContext {
            character_id: Some("any".to_string()),
            ..Default::default()
        };
        assert!(router.resolve(&ctx).is_none());
    }

    #[test]
    fn resolve_falls_back_to_first_default_when_no_routing_rules() {
        let entries = sample_entries();
        let routing = ProviderRouting::default();
        let router = ProviderRouter::new(entries, routing);
        let ctx = RouteContext::default();
        let resolved = router.resolve(&ctx).expect("should fall back to is_default entry");
        assert_eq!(resolved.entry.name, "openai");
        assert_eq!(resolved.matched_rule, MatchedRule::FirstDefault);
    }

    #[test]
    fn resolve_falls_back_to_default_provider_field_when_present() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("deepseek".to_string()),
            ..Default::default()
        };
        let router = ProviderRouter::new(entries, routing);
        let ctx = RouteContext::default();
        let resolved = router.resolve(&ctx).expect("should hit default_provider");
        assert_eq!(resolved.entry.name, "deepseek");
        assert_eq!(resolved.matched_rule, MatchedRule::Default);
    }

    #[test]
    fn character_id_takes_priority_over_scene_role_and_task_kind() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("openai".to_string()),
            by_character: [("char-x".to_string(), "deepseek".to_string())].into(),
            by_scene_role: [("narrator".to_string(), "local".to_string())].into(),
            by_task_kind: [("summarize".to_string(), "local".to_string())].into(),
        };
        let router = ProviderRouter::new(entries, routing);
        let ctx = RouteContext {
            character_id: Some("char-x".to_string()),
            scene_role: Some("narrator".to_string()),
            task_kind: Some("summarize".to_string()),
        };
        let resolved = router.resolve(&ctx).expect("should resolve");
        assert_eq!(resolved.entry.name, "deepseek");
        assert_eq!(resolved.matched_rule, MatchedRule::Character);
    }

    #[test]
    fn scene_role_takes_priority_over_task_kind_and_default() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("openai".to_string()),
            by_scene_role: [("narrator".to_string(), "local".to_string())].into(),
            by_task_kind: [("summarize".to_string(), "deepseek".to_string())].into(),
            ..Default::default()
        };
        let router = ProviderRouter::new(entries, routing);
        let ctx = RouteContext {
            character_id: None,
            scene_role: Some("narrator".to_string()),
            task_kind: Some("summarize".to_string()),
        };
        let resolved = router.resolve(&ctx).expect("should resolve");
        assert_eq!(resolved.entry.name, "local");
        assert_eq!(resolved.matched_rule, MatchedRule::SceneRole);
    }

    #[test]
    fn task_kind_resolves_when_character_and_scene_absent() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("openai".to_string()),
            by_task_kind: [("summarize".to_string(), "deepseek".to_string())].into(),
            ..Default::default()
        };
        let router = ProviderRouter::new(entries, routing);
        let ctx = RouteContext {
            task_kind: Some("summarize".to_string()),
            ..Default::default()
        };
        let resolved = router.resolve(&ctx).expect("should resolve");
        assert_eq!(resolved.entry.name, "deepseek");
        assert_eq!(resolved.matched_rule, MatchedRule::TaskKind);
    }

    #[test]
    fn dangling_routing_reference_falls_through_to_default() {
        // routing.by_character 指向不存在的 provider name 时，
        // resolve 不应 panic，应继续向下走到 default_provider。
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("openai".to_string()),
            by_character: [("char-x".to_string(), "ghost-provider".to_string())].into(),
            ..Default::default()
        };
        let router = ProviderRouter::new(entries, routing);
        let ctx = RouteContext {
            character_id: Some("char-x".to_string()),
            ..Default::default()
        };
        let resolved = router.resolve(&ctx).expect("should fall through to default_provider");
        assert_eq!(resolved.entry.name, "openai");
        assert_eq!(resolved.matched_rule, MatchedRule::Default);
    }

    #[test]
    fn get_returns_entry_by_name() {
        let entries = sample_entries();
        let router = ProviderRouter::new(entries, ProviderRouting::default());
        let e = router.get("local").expect("local entry exists");
        assert_eq!(e.model, "llama3");
        assert!(router.get("nonexistent").is_none());
    }

    #[test]
    fn to_provider_config_and_generation_params_round_trip() {
        let e = entry(
            "deepseek",
            "https://api.deepseek.com/v1/chat/completions",
            "deepseek-chat",
            false,
        );
        let cfg = e.to_provider_config();
        assert_eq!(cfg.endpoint, e.endpoint);
        assert!(cfg.api_key.is_none());
        let params = e.to_generation_params();
        assert_eq!(params.model, e.model);
        assert!(params.temperature.is_none());
        assert!(params.max_tokens.is_none());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let entries = vec![entry("", "https://x", "m", true)];
        let routing = ProviderRouting::default();
        let err = validate_provider_config(&entries, &routing).unwrap_err();
        assert!(err.contains("name 不能为空"));
    }

    #[test]
    fn validate_rejects_duplicate_name() {
        let entries = vec![
            entry("dup", "https://a", "m1", true),
            entry("dup", "https://b", "m2", false),
        ];
        let err = validate_provider_config(&entries, &ProviderRouting::default()).unwrap_err();
        assert!(err.contains("重复"));
    }

    #[test]
    fn validate_rejects_empty_endpoint_or_model() {
        let entries_no_endpoint = vec![ProviderEntry {
            name: "x".to_string(),
            endpoint: String::new(),
            api_key: None,
            model: "m".to_string(),
            engine: BackendEngine::Direct,
            is_default: true,
        }];
        let err = validate_provider_config(&entries_no_endpoint, &ProviderRouting::default())
            .unwrap_err();
        assert!(err.contains("endpoint 不能为空"));

        let entries_no_model = vec![ProviderEntry {
            name: "x".to_string(),
            endpoint: "https://x".to_string(),
            api_key: None,
            model: String::new(),
            engine: BackendEngine::Direct,
            is_default: true,
        }];
        let err = validate_provider_config(&entries_no_model, &ProviderRouting::default())
            .unwrap_err();
        assert!(err.contains("model 不能为空"));
    }

    #[test]
    fn validate_requires_at_least_one_default_when_entries_nonempty() {
        let entries = vec![
            entry("a", "https://a", "m", false),
            entry("b", "https://b", "m", false),
        ];
        let err = validate_provider_config(&entries, &ProviderRouting::default()).unwrap_err();
        assert!(err.contains("is_default = true"));
    }

    #[test]
    fn validate_passes_on_empty_entries() {
        // 空 entries 不需要 default；routing 字段也应允许空
        assert!(validate_provider_config(&[], &ProviderRouting::default()).is_ok());
    }

    #[test]
    fn validate_rejects_default_provider_pointing_to_missing_entry() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("ghost".to_string()),
            ..Default::default()
        };
        let err = validate_provider_config(&entries, &routing).unwrap_err();
        assert!(err.contains("default_provider 指向不存在"));
    }

    #[test]
    fn validate_rejects_by_character_pointing_to_missing_entry() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            by_character: [("char-x".to_string(), "ghost".to_string())].into(),
            ..Default::default()
        };
        let err = validate_provider_config(&entries, &routing).unwrap_err();
        assert!(err.contains("by_character"));
    }

    #[test]
    fn validate_rejects_by_scene_role_pointing_to_missing_entry() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            by_scene_role: [("narrator".to_string(), "ghost".to_string())].into(),
            ..Default::default()
        };
        let err = validate_provider_config(&entries, &routing).unwrap_err();
        assert!(err.contains("by_scene_role"));
    }

    #[test]
    fn validate_rejects_by_task_kind_pointing_to_missing_entry() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            by_task_kind: [("summarize".to_string(), "ghost".to_string())].into(),
            ..Default::default()
        };
        let err = validate_provider_config(&entries, &routing).unwrap_err();
        assert!(err.contains("by_task_kind"));
    }

    #[test]
    fn validate_passes_when_all_routing_targets_resolve() {
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("openai".to_string()),
            by_character: [("char-x".to_string(), "deepseek".to_string())].into(),
            by_scene_role: [("narrator".to_string(), "local".to_string())].into(),
            by_task_kind: [("summarize".to_string(), "openai".to_string())].into(),
        };
        assert!(validate_provider_config(&entries, &routing).is_ok());
    }

    #[test]
    fn router_serializes_entries_via_serde_skip_api_key() {
        // api_key 使用 serde(skip)，序列化结果不应包含该字段
        let mut e = entry("openai", "https://x", "gpt-4o", true);
        e.api_key = Some("sk-secret".to_string());
        let v = serde_json::to_value(&e).unwrap();
        assert!(v.get("api_key").is_none(), "api_key must be skipped");
        assert_eq!(v["name"], "openai");
        // 反序列化回不携带 api_key → 字段为 None
        let raw = serde_json::to_string(&e).unwrap();
        let back: ProviderEntry = serde_json::from_str(&raw).unwrap();
        assert!(back.api_key.is_none());
    }

    #[test]
    fn routing_default_deserializes_empty_object() {
        // 空 routing 对象应反序列化为 default（所有 HashMap 空、default_provider None）
        let raw = r#"{}"#;
        let routing: ProviderRouting = serde_json::from_str(raw).unwrap();
        assert!(routing.default_provider.is_none());
        assert!(routing.by_character.is_empty());
        assert!(routing.by_scene_role.is_empty());
        assert!(routing.by_task_kind.is_empty());
    }

    // ── 持久化测试 ────────────────────────────────────────────────────────

    #[test]
    fn load_provider_routing_returns_empty_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let (entries, routing) = load_provider_routing(tmp.path()).unwrap();
        assert!(entries.is_empty());
        assert_eq!(routing, ProviderRouting::default());
    }

    #[test]
    fn save_then_load_provider_routing_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = sample_entries();
        let routing = ProviderRouting {
            default_provider: Some("openai".to_string()),
            by_character: [("char-x".to_string(), "deepseek".to_string())].into(),
            by_scene_role: [("narrator".to_string(), "local".to_string())].into(),
            by_task_kind: [("summarize".to_string(), "openai".to_string())].into(),
        };
        save_provider_routing(tmp.path(), &entries, &routing).unwrap();

        let (loaded_entries, loaded_routing) = load_provider_routing(tmp.path()).unwrap();
        assert_eq!(loaded_entries.len(), entries.len());
        assert_eq!(loaded_entries[0].name, "openai");
        assert_eq!(loaded_entries[0].api_key, None, "api_key must be stripped on disk");
        assert_eq!(loaded_routing, routing);
    }

    #[test]
    fn save_provider_routing_strips_api_key_even_if_set() {
        let tmp = tempfile::tempdir().unwrap();
        let mut entries = sample_entries();
        entries[0].api_key = Some("sk-leaked".to_string());
        save_provider_routing(tmp.path(), &entries, &ProviderRouting::default()).unwrap();

        // 直接读盘上文件，确认 api_key 字段不存在
        let path = providers_file_path(tmp.path());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("sk-leaked"),
            "api_key must never be persisted to providers.json"
        );
        assert!(!raw.contains("api_key"), "api_key field should be skipped entirely");
    }

    #[test]
    fn load_provider_routing_rejects_wrong_version() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = r#"{"version":99,"entries":[],"routing":{}}"#;
        std::fs::write(providers_file_path(tmp.path()), raw).unwrap();
        let err = load_provider_routing(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("版本不匹配"));
    }

    #[test]
    fn load_provider_routing_rejects_malformed_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(providers_file_path(tmp.path()), "{not json").unwrap();
        assert!(load_provider_routing(tmp.path()).is_err());
    }

    #[test]
    fn load_provider_keys_returns_empty_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = load_provider_keys(tmp.path()).unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn save_then_load_provider_keys_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut keys: HashMap<String, String> = HashMap::new();
        keys.insert("openai".to_string(), "sk-openai".to_string());
        keys.insert("deepseek".to_string(), "sk-deepseek".to_string());
        save_provider_keys(tmp.path(), &keys).unwrap();

        let loaded = load_provider_keys(tmp.path()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("openai").map(String::as_str), Some("sk-openai"));
        assert_eq!(loaded.get("deepseek").map(String::as_str), Some("sk-deepseek"));
    }

    #[test]
    fn save_provider_keys_drops_empty_values() {
        let tmp = tempfile::tempdir().unwrap();
        let mut keys: HashMap<String, String> = HashMap::new();
        keys.insert("openai".to_string(), "sk-real".to_string());
        keys.insert("empty".to_string(), String::new());
        save_provider_keys(tmp.path(), &keys).unwrap();

        let loaded = load_provider_keys(tmp.path()).unwrap();
        assert_eq!(loaded.len(), 1, "empty key values must be dropped");
        assert!(loaded.contains_key("openai"));
    }

    #[test]
    fn load_provider_keys_rejects_wrong_version() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = r#"{"version":99,"keys":{}}"#;
        std::fs::write(provider_keys_file_path(tmp.path()), raw).unwrap();
        assert!(load_provider_keys(tmp.path()).is_err());
    }

    #[test]
    fn load_provider_router_returns_empty_when_providers_json_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let router = load_provider_router(tmp.path()).unwrap();
        assert!(router.is_empty());
    }

    #[test]
    fn load_provider_router_merges_api_keys_into_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = sample_entries();
        save_provider_routing(tmp.path(), &entries, &ProviderRouting::default()).unwrap();

        let mut keys: HashMap<String, String> = HashMap::new();
        keys.insert("openai".to_string(), "sk-openai-secret".to_string());
        keys.insert("local".to_string(), "sk-local-secret".to_string());
        // 一个孤立 key（对应不存在的 entry name），应被忽略
        keys.insert("ghost".to_string(), "sk-ghost".to_string());
        save_provider_keys(tmp.path(), &keys).unwrap();

        let router = load_provider_router(tmp.path()).unwrap();
        let openai = router.get("openai").expect("openai entry exists");
        assert_eq!(openai.api_key.as_deref(), Some("sk-openai-secret"));
        let local = router.get("local").expect("local entry exists");
        assert_eq!(local.api_key.as_deref(), Some("sk-local-secret"));
        let deepseek = router.get("deepseek").expect("deepseek entry exists");
        assert!(
            deepseek.api_key.is_none(),
            "deepseek has no key in provider_keys.json"
        );
        assert!(router.get("ghost").is_none(), "ghost key must not create an entry");
    }

    #[test]
    fn load_provider_router_rejects_invalid_config() {
        let tmp = tempfile::tempdir().unwrap();
        // entries 非空但无 is_default=true → validate 应失败
        let entries = vec![
            entry("a", "https://a", "m1", false),
            entry("b", "https://b", "m2", false),
        ];
        save_provider_routing(tmp.path(), &entries, &ProviderRouting::default()).unwrap();
        let err = load_provider_router(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("is_default"));
    }
}
