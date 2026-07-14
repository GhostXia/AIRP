//! Shared SillyTavern → AIRP worldbook normalizer.
//!
//! 单一归一化入口：把 SillyTavern character_book JSON（或裸 ST lorebook
//! entries）转换为 AIRP canonical [`Lorebook`]，同时产出
//! [`WorldbookImportReport`] 诊断信息。
//!
//! 三个入口点共用此 normalizer：
//! 1. PNG character_book 导入（`handlers::convert_character_book_to_lorebook`）
//! 2. PUT `/v1/characters/:id/lorebook` API
//! 3. Agent `update_lorebook` tool
//!
//! 设计原则：
//! - **幂等**：传入 canonical AIRP Lorebook JSON，输出等价 Lorebook，
//!   `extensions` 不产生冗余字段。
//! - **保留**：ST-only 字段（`selective`/`position`/`probability`/…）不丢弃，
//!   原样进入 `extensions`，供未来检索 Tool 或人工审阅使用。
//! - **诊断**：每条 entry 的归一化结果有明确状态（converted / advisory_preserved
//!   / aliases_normalized / invalid / needs_review）。
//! - **不阻塞**：invalid 条目被跳过，其余继续处理；`needs_review` 不阻塞写入。
//!
//! 不变式守护：
//! - 不变式①：normalizer 只做数据归一化，不注入 agent 脚手架。
//! - trigger() 不消费 advisory metadata（`secondary_keys`/`case_sensitive`/
//!   `extensions`）；这些字段当前是"建议元数据 + 未来检索 Tool 的输入"。

use crate::orchestrator::lorebook::{Lorebook, LorebookEntry, DEFAULT_PRIORITY};
use serde_json::Value;
use std::collections::BTreeMap;

/// 已消费字段集合：这些字段已被 normalizer 提取到 canonical 或 advisory 字段，
/// 不会重复进入 `extensions`。包含 AIRP canonical 字段名和 SillyTavern 别名。
const CONSUMED_FIELDS: &[&str] = &[
    // AIRP canonical
    "keys",
    "content",
    "enabled",
    "priority",
    "constant",
    "comment",
    "secondary_keys",
    "case_sensitive",
    "extensions",
    // SillyTavern aliases（已被 normalizer 消费）
    "key",
    "disable",
    "order",
    "insertion_order",
    "keysecondary",
    "caseSensitive",
];

// ── Import diagnostics ──────────────────────────────────────────────────────

/// 归一化诊断报告。
///
/// 每个计数都是"条目数"而非"字段数"——同一条目可能同时命中多个计数
/// （例如既用了别名又保留了 advisory metadata）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct WorldbookImportReport {
    /// Source-level shape error. Entry-level errors remain in `invalid`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_error: Option<String>,
    /// 源 JSON 中的 entry 总数。
    pub total_input: usize,
    /// 成功转换为 canonical LorebookEntry 的条目数。
    pub converted: usize,
    /// 使用了 SillyTavern 别名字段的条目数（`key`/`disable`/`order`/
    /// `insertion_order`/`keysecondary`/`caseSensitive`）。
    pub aliases_normalized: usize,
    /// 有 advisory metadata 被保留的条目数（`secondary_keys` 非空 /
    /// `case_sensitive` 有值 / `extensions` 非空）。
    pub advisory_preserved: usize,
    /// 无法解析、被跳过的条目（含原因）。
    pub invalid: Vec<EntryDiagnostic>,
    /// 需人工复核的条目（含原因）。不阻塞写入。
    pub needs_review: Vec<EntryDiagnostic>,
}

impl WorldbookImportReport {
    /// 是否有 source / entry 级错误或需要人工复核的条目。
    pub fn has_issues(&self) -> bool {
        self.source_error.is_some() || !self.invalid.is_empty() || !self.needs_review.is_empty()
    }

    /// 成功导入的条目数（= converted）。
    pub fn imported_count(&self) -> usize {
        self.converted
    }

    /// 被跳过的条目数（= invalid 条数）。
    pub fn skipped_count(&self) -> usize {
        self.invalid.len()
    }

    /// Return the reason why this result must not replace a persisted lorebook.
    /// Explicit empty containers (`{"entries": []}` / `{"entries": {}}`) are valid clears.
    pub fn replacement_error(&self) -> Option<String> {
        if let Some(reason) = &self.source_error {
            return Some(reason.clone());
        }
        if self.total_input > 0 && self.converted == 0 {
            return Some(format!(
                "all {} worldbook entries are invalid; refusing to replace existing data",
                self.total_input
            ));
        }
        None
    }
}

/// 单条 entry 的诊断信息。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EntryDiagnostic {
    /// 在源 JSON entries 中的位置索引。
    pub index: usize,
    /// 条目的 comment 字段（如有），便于人工定位。
    pub comment: Option<String>,
    /// 诊断原因。
    pub reason: String,
}

// ── Normalization ───────────────────────────────────────────────────────────

/// 把 SillyTavern character_book / lorebook JSON 归一化为 AIRP canonical
/// [`Lorebook`]，并返回诊断报告。
///
/// 接受以下输入形式（自动探测）：
/// - ST character_book：`{ "entries": { "0": {...}, "1": {...} }, ... }`
/// - ST lorebook 数组：`{ "entries": [{...}, {...}] }`
/// - 裸 entry 数组：`[{...}, {...}]`
/// - 单个 entry 对象：`{...}`
/// - AIRP canonical Lorebook JSON（幂等：输出等价 Lorebook）
///
/// invalid 条目被跳过，其余继续处理。`needs_review` 不阻塞。
pub fn normalize_worldbook(source: &Value) -> (Lorebook, WorldbookImportReport) {
    let raw_entries = match extract_raw_entries(source) {
        Ok(entries) => entries,
        Err(reason) => {
            return (
                Lorebook {
                    entries: Vec::new(),
                },
                WorldbookImportReport {
                    source_error: Some(reason),
                    ..Default::default()
                },
            );
        }
    };
    let mut report = WorldbookImportReport {
        total_input: raw_entries.len(),
        ..Default::default()
    };

    let mut entries: Vec<LorebookEntry> = Vec::with_capacity(raw_entries.len());

    for (idx, v) in raw_entries.iter().enumerate() {
        match normalize_entry(v, idx) {
            Ok((entry, diag)) => {
                if diag.aliases_used {
                    report.aliases_normalized += 1;
                }
                if diag.advisory_preserved {
                    report.advisory_preserved += 1;
                }
                if let Some(reason) = diag.needs_review_reason {
                    report.needs_review.push(EntryDiagnostic {
                        index: idx,
                        comment: entry.comment.clone(),
                        reason,
                    });
                }
                entries.push(entry);
                report.converted += 1;
            }
            Err(reason) => {
                let comment = v
                    .get("comment")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_owned());
                report.invalid.push(EntryDiagnostic {
                    index: idx,
                    comment,
                    reason,
                });
            }
        }
    }

    // 统一按 priority 降序排列，与 trigger() 的运行时排序方向一致，
    // 避免存储顺序与运行时输出顺序漂移。trigger() 会重新排序，此处主要为可读性。
    entries.sort_by_key(|e| std::cmp::Reverse(e.priority.unwrap_or(DEFAULT_PRIORITY)));

    (Lorebook { entries }, report)
}

/// 从源 JSON 中提取 raw entry 列表，处理 ST 的多种包装形式。
fn extract_raw_entries(source: &Value) -> Result<Vec<&Value>, String> {
    // Wrapped AIRP/ST form. Presence of `entries` is authoritative; a scalar
    // must not fall through and be mistaken for a single entry.
    if let Some(entries_val) = source.get("entries") {
        if let Some(map) = entries_val.as_object() {
            if looks_like_entry(map) {
                return Ok(vec![entries_val]);
            }
            return Ok(map.values().collect());
        }
        if let Some(arr) = entries_val.as_array() {
            return Ok(arr.iter().collect());
        }
        return Err("'entries' must be an array or object map".to_string());
    }

    if let Some(arr) = source.as_array() {
        return Ok(arr.iter().collect());
    }

    if let Some(map) = source.as_object() {
        if looks_like_entry(map) {
            return Ok(vec![source]);
        }
        // Also accept a bare non-empty uid-keyed entry map, but reject `{}` and
        // metadata-only objects so malformed replace requests cannot clear data.
        if !map.is_empty()
            && map
                .values()
                .all(|value| value.as_object().is_some_and(looks_like_entry))
        {
            return Ok(map.values().collect());
        }
    }

    Err(
        "unsupported worldbook shape; expected an entries container, entry array, or entry object"
            .to_string(),
    )
}

/// 判断一个 JSON object 是否看起来像一个 lorebook entry（有 content 或 keys 字段）。
fn looks_like_entry(obj: &serde_json::Map<String, Value>) -> bool {
    obj.contains_key("content") || obj.contains_key("keys") || obj.contains_key("key")
}

// ── Per-entry normalization ─────────────────────────────────────────────────

struct EntryDiag {
    aliases_used: bool,
    advisory_preserved: bool,
    needs_review_reason: Option<String>,
}

/// 归一化单条 entry。返回 `Err(reason)` 表示 invalid（跳过）。
fn normalize_entry(v: &Value, _index: usize) -> Result<(LorebookEntry, EntryDiag), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "entry is not a JSON object".to_string())?;

    validate_entry_field_types(obj)?;

    // 1. keys：优先 `keys`（array），回退 `key`（string，逗号分隔）
    let (keys, used_key_alias) = extract_keys(obj);

    // 2. content：必须有且为 string
    let content = obj
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "missing or non-string 'content' field".to_string())?
        .to_owned();

    // 3. enabled：优先 `enabled`，回退 `disable`（反转）
    let (enabled, used_disable_alias) = extract_enabled(obj);

    // 4. priority：优先 `priority`，回退 `order`，再回退 `insertion_order`
    let (priority, used_order_alias) = extract_priority(obj);

    // 5. constant
    let constant = obj.get("constant").and_then(|c| c.as_bool());

    // 6. comment
    let comment = obj
        .get("comment")
        .and_then(|c| c.as_str())
        .map(|s| s.to_owned());

    // 7. secondary_keys：优先 `secondary_keys`（AIRP canonical），回退 `keysecondary`（ST）
    let (secondary_keys, used_keysecondary_alias) = extract_secondary_keys(obj);

    // 8. case_sensitive：优先 `case_sensitive`（AIRP canonical），回退 `caseSensitive`（ST）
    let (case_sensitive, used_casesensitive_alias) = extract_case_sensitive(obj);

    // 9. extensions：保留已有 AIRP extensions + 收集所有未消费字段
    let extensions = collect_extensions(obj);

    // 诊断
    let aliases_used = used_key_alias
        || used_disable_alias
        || used_order_alias
        || used_keysecondary_alias
        || used_casesensitive_alias;

    let advisory_preserved =
        !secondary_keys.is_empty() || case_sensitive.is_some() || extensions.is_some();

    // needs_review：keys 为空且非 constant —— 此 entry 永远不会触发
    let needs_review_reason = if keys.is_empty() && !constant.unwrap_or(false) {
        Some("entry has no keys and is not constant — it will never trigger".to_string())
    } else {
        None
    };

    let entry = LorebookEntry {
        keys,
        content,
        enabled,
        priority,
        constant,
        comment,
        secondary_keys,
        case_sensitive,
        extensions,
    };

    Ok((
        entry,
        EntryDiag {
            aliases_used,
            advisory_preserved,
            needs_review_reason,
        },
    ))
}

/// 提取 keys。优先 `keys`（array of strings），回退 `key`（string，逗号分隔）。
/// 返回 (keys, used_alias)。
fn extract_keys(obj: &serde_json::Map<String, Value>) -> (Vec<String>, bool) {
    if let Some(arr) = obj.get("keys").and_then(|k| k.as_array()) {
        let keys: Vec<String> = arr
            .iter()
            .filter_map(|s| s.as_str().map(|s| s.to_owned()))
            .filter(|s| !s.is_empty())
            .collect();
        return (keys, false);
    }

    // ST `key` is seen as either a comma-separated string or an array.
    if let Some(key_str) = obj.get("key").and_then(|k| k.as_str()) {
        let keys: Vec<String> = key_str
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        return (keys, true);
    }
    if let Some(arr) = obj.get("key").and_then(|k| k.as_array()) {
        return (
            arr.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .filter(|s| !s.is_empty())
                .collect(),
            true,
        );
    }

    (Vec::new(), false)
}

fn validate_entry_field_types(obj: &serde_json::Map<String, Value>) -> Result<(), String> {
    for name in ["keys", "secondary_keys", "keysecondary"] {
        if let Some(value) = obj.get(name) {
            let Some(values) = value.as_array() else {
                return Err(format!("'{name}' must be an array of strings"));
            };
            if values.iter().any(|value| !value.is_string()) {
                return Err(format!("'{name}' must contain only strings"));
            }
        }
    }

    if let Some(value) = obj.get("key") {
        let valid = value.is_string()
            || value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string));
        if !valid {
            return Err("'key' must be a string or array of strings".to_string());
        }
    }

    for name in [
        "enabled",
        "disable",
        "constant",
        "case_sensitive",
        "caseSensitive",
    ] {
        if obj
            .get(name)
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
        {
            return Err(format!("'{name}' must be a boolean"));
        }
    }

    for name in ["priority", "order", "insertion_order"] {
        if let Some(value) = obj.get(name) {
            if value.is_null() {
                continue;
            }
            let Some(number) = value.as_i64() else {
                return Err(format!("'{name}' must be a 32-bit integer"));
            };
            if i32::try_from(number).is_err() {
                return Err(format!("'{name}' is outside the 32-bit integer range"));
            }
        }
    }

    if obj
        .get("comment")
        .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err("'comment' must be a string or null".to_string());
    }
    if obj
        .get("extensions")
        .is_some_and(|value| !value.is_null() && !value.is_object())
    {
        return Err("'extensions' must be an object or null".to_string());
    }

    Ok(())
}

/// 提取 enabled。优先 `enabled`（bool），回退 `disable`（bool，反转）。
/// 返回 (enabled, used_alias)。
fn extract_enabled(obj: &serde_json::Map<String, Value>) -> (Option<bool>, bool) {
    if let Some(enabled) = obj.get("enabled").and_then(|e| e.as_bool()) {
        return (Some(enabled), false);
    }

    if let Some(disable) = obj.get("disable").and_then(|d| d.as_bool()) {
        return (Some(!disable), true);
    }

    (None, false)
}

/// 提取 priority。优先 `priority`，回退 `order`，再回退 `insertion_order`。
/// 返回 (priority, used_alias)。
fn extract_priority(obj: &serde_json::Map<String, Value>) -> (Option<i32>, bool) {
    if let Some(p) = obj.get("priority").and_then(|p| p.as_i64()) {
        return (Some(p as i32), false);
    }

    if let Some(p) = obj.get("order").and_then(|p| p.as_i64()) {
        return (Some(p as i32), true);
    }

    if let Some(p) = obj.get("insertion_order").and_then(|p| p.as_i64()) {
        return (Some(p as i32), true);
    }

    (None, false)
}

/// 提取 secondary_keys。优先 `secondary_keys`（AIRP canonical），回退 `keysecondary`（ST）。
/// 返回 (secondary_keys, used_alias)。
fn extract_secondary_keys(obj: &serde_json::Map<String, Value>) -> (Vec<String>, bool) {
    if let Some(arr) = obj.get("secondary_keys").and_then(|k| k.as_array()) {
        let keys: Vec<String> = arr
            .iter()
            .filter_map(|s| s.as_str().map(|s| s.to_owned()))
            .filter(|s| !s.is_empty())
            .collect();
        return (keys, false);
    }

    if let Some(arr) = obj.get("keysecondary").and_then(|k| k.as_array()) {
        let keys: Vec<String> = arr
            .iter()
            .filter_map(|s| s.as_str().map(|s| s.to_owned()))
            .filter(|s| !s.is_empty())
            .collect();
        return (keys, true);
    }

    (Vec::new(), false)
}

/// 提取 case_sensitive。优先 `case_sensitive`（AIRP canonical），回退 `caseSensitive`（ST）。
/// 返回 (case_sensitive, used_alias)。
fn extract_case_sensitive(obj: &serde_json::Map<String, Value>) -> (Option<bool>, bool) {
    if let Some(cs) = obj.get("case_sensitive").and_then(|c| c.as_bool()) {
        return (Some(cs), false);
    }

    if let Some(cs) = obj.get("caseSensitive").and_then(|c| c.as_bool()) {
        return (Some(cs), true);
    }

    (None, false)
}

/// 收集所有未消费字段到 extensions BTreeMap。
///
/// 如果输入已有 AIRP canonical `extensions`（object），先保留其内容，
/// 再把未消费的 ST-only / 未知字段合并进去。BTreeMap 保证序列化顺序稳定。
/// 返回 None 如果没有任何 extension 字段（保持 canonical 输出干净）。
fn collect_extensions(obj: &serde_json::Map<String, Value>) -> Option<BTreeMap<String, Value>> {
    let mut ext: BTreeMap<String, Value> = BTreeMap::new();

    // 保留已有 AIRP extensions
    if let Some(existing) = obj.get("extensions").and_then(|e| e.as_object()) {
        for (k, v) in existing {
            ext.insert(k.clone(), v.clone());
        }
    }

    // 收集未消费字段
    for (k, v) in obj {
        if !CONSUMED_FIELDS.contains(&k.as_str()) {
            ext.insert(k.clone(), v.clone());
        }
    }

    if ext.is_empty() {
        None
    } else {
        Some(ext)
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Alias normalization ──────────────────────────────────────────────

    #[test]
    fn test_key_singular_alias_split_by_comma() {
        let src = serde_json::json!({
            "key": "moon gate, night, observatory",
            "content": "test content"
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(
            lb.entries[0].keys,
            vec!["moon gate", "night", "observatory"]
        );
        assert_eq!(report.aliases_normalized, 1);
    }

    #[test]
    fn test_disable_alias_inverted_to_enabled() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "disable": true
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].enabled, Some(false));
        assert_eq!(report.aliases_normalized, 1);

        let src2 = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "disable": false
        });
        let (lb2, _) = normalize_worldbook(&src2);
        assert_eq!(lb2.entries[0].enabled, Some(true));
    }

    #[test]
    fn test_order_alias_maps_to_priority() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "order": 42
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].priority, Some(42));
        assert_eq!(report.aliases_normalized, 1);
    }

    #[test]
    fn test_insertion_order_alias_fallback() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "insertion_order": 15
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].priority, Some(15));
        assert_eq!(report.aliases_normalized, 1);
    }

    #[test]
    fn test_keysecondary_alias_maps_to_secondary_keys() {
        let src = serde_json::json!({
            "keys": ["primary"],
            "keysecondary": ["secondary1", "secondary2"],
            "content": "test"
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(
            lb.entries[0].secondary_keys,
            vec!["secondary1", "secondary2"]
        );
        assert_eq!(report.aliases_normalized, 1);
        assert_eq!(report.advisory_preserved, 1);
    }

    #[test]
    fn test_casesensitive_alias_maps_to_case_sensitive() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "caseSensitive": true
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].case_sensitive, Some(true));
        assert_eq!(report.aliases_normalized, 1);
        assert_eq!(report.advisory_preserved, 1);
    }

    #[test]
    fn test_enabled_takes_precedence_over_disable() {
        // When both `enabled` and `disable` are present, `enabled` wins.
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "enabled": true,
            "disable": true
        });
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].enabled, Some(true));
    }

    #[test]
    fn test_priority_takes_precedence_over_order() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "priority": 99,
            "order": 1
        });
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].priority, Some(99));
    }

    // ── Entry format handling ────────────────────────────────────────────

    #[test]
    fn test_st_character_book_object_map_entries() {
        let src = serde_json::json!({
            "entries": {
                "0": { "keys": ["a"], "content": "entry A" },
                "1": { "keys": ["b"], "content": "entry B" }
            }
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 2);
        assert_eq!(report.total_input, 2);
        assert_eq!(report.converted, 2);
    }

    #[test]
    fn test_st_lorebook_array_entries() {
        let src = serde_json::json!({
            "entries": [
                { "keys": ["a"], "content": "entry A" },
                { "keys": ["b"], "content": "entry B" }
            ]
        });
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 2);
    }

    #[test]
    fn test_bare_array_entries() {
        let src = serde_json::json!([
            { "keys": ["a"], "content": "entry A" },
            { "keys": ["b"], "content": "entry B" }
        ]);
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 2);
    }

    #[test]
    fn test_single_entry_object() {
        let src = serde_json::json!({
            "keys": ["solo"],
            "content": "single entry"
        });
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(lb.entries[0].keys, vec!["solo"]);
    }

    // ── Invalid entries ─────────────────────────────────────────────────

    #[test]
    fn test_missing_content_is_invalid() {
        let src = serde_json::json!({
            "entries": [
                { "keys": ["a"], "content": "valid" },
                { "keys": ["b"] }
            ]
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].index, 1);
        assert!(report.invalid[0].reason.contains("content"));
    }

    #[test]
    fn test_wrong_field_types_are_invalid_instead_of_defaulted() {
        let src = serde_json::json!({
            "entries": [
                { "keys": "dragon", "content": "wrong keys type" },
                { "keys": ["dragon"], "content": "wrong enabled type", "enabled": "false" },
                { "keys": ["dragon"], "content": "priority overflow", "priority": 2147483648_i64 }
            ]
        });
        let (lb, report) = normalize_worldbook(&src);
        assert!(lb.entries.is_empty());
        assert_eq!(report.invalid.len(), 3);
        assert!(report.replacement_error().is_some());
    }

    #[test]
    fn test_unsupported_shape_is_fatal_but_explicit_empty_entries_is_valid() {
        let (_, malformed) = normalize_worldbook(&serde_json::json!({"name": "not entries"}));
        assert!(malformed.source_error.is_some());
        assert!(malformed.replacement_error().is_some());

        let (empty, report) = normalize_worldbook(&serde_json::json!({"entries": []}));
        assert!(empty.entries.is_empty());
        assert!(report.source_error.is_none());
        assert!(report.replacement_error().is_none());
    }

    #[test]
    fn test_wrapped_single_entry_object_is_supported() {
        let src = serde_json::json!({
            "entries": {"keys": ["dragon"], "content": "single wrapped entry"}
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(report.converted, 1);
        assert!(report.replacement_error().is_none());
    }

    #[test]
    fn test_non_object_entry_is_invalid() {
        let src = serde_json::json!({
            "entries": [
                "not an object",
                42,
                { "keys": ["a"], "content": "valid" }
            ]
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(report.invalid.len(), 2);
    }

    // ── Needs review ────────────────────────────────────────────────────

    #[test]
    fn test_empty_keys_non_constant_needs_review() {
        let src = serde_json::json!({
            "keys": [],
            "content": "orphan entry"
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(report.needs_review.len(), 1);
        assert!(report.needs_review[0].reason.contains("never trigger"));
    }

    #[test]
    fn test_empty_keys_constant_does_not_need_review() {
        let src = serde_json::json!({
            "keys": [],
            "content": "constant entry",
            "constant": true
        });
        let (_, report) = normalize_worldbook(&src);
        assert_eq!(report.needs_review.len(), 0);
    }

    // ── Extensions preservation ─────────────────────────────────────────

    #[test]
    fn test_st_only_fields_go_to_extensions() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "selective": true,
            "position": "before_char",
            "depth": 4,
            "probability": 80,
            "sticky": 5,
            "cooldown": 10,
            "delay": 2,
            "group": "lore_group_1",
            "use_regex": false,
            "match_whole_words": true,
            "recursion": false
        });
        let (lb, report) = normalize_worldbook(&src);
        assert_eq!(lb.entries.len(), 1);
        let ext = lb.entries[0]
            .extensions
            .as_ref()
            .expect("extensions should be populated");
        assert_eq!(ext.get("selective"), Some(&serde_json::json!(true)));
        assert_eq!(ext.get("position"), Some(&serde_json::json!("before_char")));
        assert_eq!(ext.get("depth"), Some(&serde_json::json!(4)));
        assert_eq!(ext.get("probability"), Some(&serde_json::json!(80)));
        assert_eq!(ext.get("sticky"), Some(&serde_json::json!(5)));
        assert_eq!(ext.get("cooldown"), Some(&serde_json::json!(10)));
        assert_eq!(ext.get("delay"), Some(&serde_json::json!(2)));
        assert_eq!(ext.get("group"), Some(&serde_json::json!("lore_group_1")));
        assert_eq!(ext.get("use_regex"), Some(&serde_json::json!(false)));
        assert_eq!(ext.get("match_whole_words"), Some(&serde_json::json!(true)));
        assert_eq!(ext.get("recursion"), Some(&serde_json::json!(false)));
        assert_eq!(report.advisory_preserved, 1);
    }

    #[test]
    fn test_unknown_fields_go_to_extensions() {
        let src = serde_json::json!({
            "keys": ["test"],
            "content": "test",
            "custom_field": "custom_value",
            "another_unknown": 123
        });
        let (lb, _) = normalize_worldbook(&src);
        let ext = lb.entries[0].extensions.as_ref().unwrap();
        assert_eq!(
            ext.get("custom_field"),
            Some(&serde_json::json!("custom_value"))
        );
        assert_eq!(ext.get("another_unknown"), Some(&serde_json::json!(123)));
    }

    #[test]
    fn test_consumed_alias_fields_not_in_extensions() {
        let src = serde_json::json!({
            "key": "test",
            "content": "test",
            "disable": true,
            "order": 10,
            "keysecondary": ["sec"],
            "caseSensitive": true
        });
        let (lb, _) = normalize_worldbook(&src);
        // extensions should be None — all fields were consumed
        assert!(lb.entries[0].extensions.is_none());
    }

    // ── Idempotency ──────────────────────────────────────────────────────

    #[test]
    fn test_idempotent_on_canonical_lorebook() {
        let canonical = serde_json::json!({
            "entries": [
                {
                    "keys": ["moon gate"],
                    "content": "The moon gate opens at night.",
                    "enabled": true,
                    "priority": 10,
                    "constant": false,
                    "comment": "test entry",
                    "secondary_keys": [],
                    "case_sensitive": null
                }
            ]
        });
        let (lb, report) = normalize_worldbook(&canonical);
        assert_eq!(lb.entries.len(), 1);
        assert_eq!(lb.entries[0].keys, vec!["moon gate"]);
        assert_eq!(lb.entries[0].content, "The moon gate opens at night.");
        assert_eq!(lb.entries[0].enabled, Some(true));
        assert_eq!(lb.entries[0].priority, Some(10));
        assert_eq!(lb.entries[0].constant, Some(false));
        assert_eq!(lb.entries[0].comment.as_deref(), Some("test entry"));
        assert!(lb.entries[0].secondary_keys.is_empty());
        assert_eq!(lb.entries[0].case_sensitive, None);
        assert!(lb.entries[0].extensions.is_none());
        // No aliases used, no advisory preserved (all empty/None)
        assert_eq!(report.aliases_normalized, 0);
        assert_eq!(report.advisory_preserved, 0);
        assert_eq!(report.invalid.len(), 0);
        assert_eq!(report.needs_review.len(), 0);
    }

    #[test]
    fn test_idempotent_round_trip() {
        // Normalize ST form → serialize → re-normalize → should be stable
        let st_form = serde_json::json!({
            "entries": [
                {
                    "keys": ["dragon"],
                    "keysecondary": ["wyrm"],
                    "content": "Dragons are ancient.",
                    "disable": false,
                    "order": 20,
                    "constant": true,
                    "caseSensitive": true,
                    "selective": true,
                    "position": "after_char"
                }
            ]
        });
        let (lb1, _) = normalize_worldbook(&st_form);
        let serialized = serde_json::to_value(&lb1).unwrap();
        let (lb2, report2) = normalize_worldbook(&serialized);

        assert_eq!(lb2.entries.len(), 1);
        assert_eq!(lb2.entries[0].keys, lb1.entries[0].keys);
        assert_eq!(lb2.entries[0].content, lb1.entries[0].content);
        assert_eq!(lb2.entries[0].enabled, lb1.entries[0].enabled);
        assert_eq!(lb2.entries[0].priority, lb1.entries[0].priority);
        assert_eq!(lb2.entries[0].constant, lb1.entries[0].constant);
        assert_eq!(lb2.entries[0].secondary_keys, lb1.entries[0].secondary_keys);
        assert_eq!(lb2.entries[0].case_sensitive, lb1.entries[0].case_sensitive);
        assert_eq!(lb2.entries[0].extensions, lb1.entries[0].extensions);
        // Second pass: no aliases (all already canonical), advisory still preserved
        assert_eq!(report2.aliases_normalized, 0);
        assert_eq!(report2.advisory_preserved, 1);
    }

    // ── Edge cases ──────────────────────────────────────────────────────

    #[test]
    fn test_empty_entries_produces_empty_lorebook() {
        let src = serde_json::json!({ "entries": [] });
        let (lb, report) = normalize_worldbook(&src);
        assert!(lb.entries.is_empty());
        assert_eq!(report.total_input, 0);
    }

    #[test]
    fn test_empty_source_object() {
        let src = serde_json::json!({});
        let (lb, report) = normalize_worldbook(&src);
        assert!(lb.entries.is_empty());
        assert_eq!(report.total_input, 0);
    }

    #[test]
    fn test_keys_filter_empty_strings() {
        let src = serde_json::json!({
            "keys": ["valid", "", "also_valid"],
            "content": "test"
        });
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].keys, vec!["valid", "also_valid"]);
    }

    #[test]
    fn test_priority_sort_descending() {
        let src = serde_json::json!({
            "entries": [
                { "keys": ["low"], "content": "low pri", "priority": 5 },
                { "keys": ["high"], "content": "high pri", "priority": 50 },
                { "keys": ["mid"], "content": "mid pri", "priority": 20 }
            ]
        });
        let (lb, _) = normalize_worldbook(&src);
        assert_eq!(lb.entries[0].keys, vec!["high"]);
        assert_eq!(lb.entries[1].keys, vec!["mid"]);
        assert_eq!(lb.entries[2].keys, vec!["low"]);
    }

    #[test]
    fn test_full_st_character_book_fixture() {
        // Mirrors the sillytavern-character-book-source.json fixture shape
        let src = serde_json::json!({
            "entries": [
                {
                    "keys": ["moon gate"],
                    "secondary_keys": ["night"],
                    "content": "The moon gate opens only at night.",
                    "enabled": true,
                    "insertion_order": 10,
                    "constant": false,
                    "selective": true,
                    "position": "before_char"
                },
                {
                    "keys": [],
                    "content": "The kingdom levies a salt tax.",
                    "disable": false,
                    "order": 25,
                    "constant": true
                },
                {
                    "keys": ["abandoned shrine"],
                    "content": "The abandoned shrine is overgrown.",
                    "disable": true,
                    "order": 15,
                    "constant": true
                }
            ]
        });
        let (lb, report) = normalize_worldbook(&src);

        assert_eq!(lb.entries.len(), 3);
        assert_eq!(report.total_input, 3);
        assert_eq!(report.converted, 3);
        assert_eq!(report.invalid.len(), 0);

        // After priority sort (descending): salt tax (25) > shrine (15) > moon gate (10)
        assert_eq!(lb.entries[0].priority, Some(25));
        assert_eq!(lb.entries[1].priority, Some(15));
        assert_eq!(lb.entries[2].priority, Some(10));

        // entries[0] = salt tax: ST aliases (disable, order), no advisory, constant=true
        assert_eq!(lb.entries[0].keys, Vec::<String>::new());
        assert_eq!(lb.entries[0].enabled, Some(true)); // disable=false → enabled=true
        assert_eq!(lb.entries[0].constant, Some(true));
        assert!(lb.entries[0].extensions.is_none());

        // entries[1] = shrine: disabled constant, ST aliases (disable, order)
        assert_eq!(lb.entries[1].keys, vec!["abandoned shrine"]);
        assert_eq!(lb.entries[1].enabled, Some(false)); // disable=true → enabled=false
        assert_eq!(lb.entries[1].constant, Some(true));

        // entries[2] = moon gate: ST aliases (secondary_keys, insertion_order), advisory preserved
        assert_eq!(lb.entries[2].keys, vec!["moon gate"]);
        assert_eq!(lb.entries[2].secondary_keys, vec!["night"]);
        assert_eq!(lb.entries[2].enabled, Some(true));
        assert_eq!(lb.entries[2].constant, Some(false));
        let ext = lb.entries[2].extensions.as_ref().unwrap();
        assert_eq!(ext.get("selective"), Some(&serde_json::json!(true)));
        assert_eq!(ext.get("position"), Some(&serde_json::json!("before_char")));
    }
}
