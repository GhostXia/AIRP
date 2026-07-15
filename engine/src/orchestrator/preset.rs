use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::card::{TavernPreset, TavernPrompt};
use crate::error::AirpError;

static SETVAR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\{setvar::([^:]+)::([^}]*)\}\}").expect("SETVAR_RE"));
static GETVAR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\{getvar::([^}]+)\}\}").expect("GETVAR_RE"));

/// `PresetImportReport` 的转换器版本。任何会改变 canonical 输出或诊断语义的
/// 改动都必须 bump 此字符串，便于 audit trail 比对。
pub const PRESET_CONVERTER_VERSION: &str = "airp-v1";

/// Preset 顶层字段中，AIRP canonical 已消费的字段集合。其余顶层字段（如
/// `prompt_order`、`top_p`、`frequency_penalty` 等 SillyTavern 扩展）保留在
/// raw sidecar 中，不进入 canonical preset 也不进入运行时执行。
pub(crate) const PRESET_TOP_LEVEL_CONSUMED: &[&str] = &[
    "prompts",
    "temperature",
    "max_tokens",
    "openai_max_tokens",
    "model",
    "openai_model",
];

/// Single TavernPrompt 已消费的字段集合。其余 prompt 字段（如 `injection_position`、
/// `injection_depth`、`probability`、`role` 之外的 ST 控制字段）保留在 raw sidecar。
pub(crate) const PROMPT_CONSUMED: &[&str] = &[
    "identifier",
    "name",
    "enabled",
    "role",
    "content",
    "system_prompt",
];

// ── Import diagnostics ──────────────────────────────────────────────────────

/// Preset 导入诊断报告（#115 P1）。
///
/// 与 [`crate::orchestrator::WorldbookImportReport`] 同构语义：
/// - `converted` 表示已写入 canonical 的 prompt 数；
/// - `aliases_normalized` 表示使用了 ST 别名字段（如 `openai_max_tokens`、
///   `openai_model`、或 prompt 缺省 `enabled` 走默认 true）的条目数；
/// - `advisory_preserved` 表示在 raw sidecar 中保留了 ST-only 字段（如
///   `prompt_order`、`injection_position`、`probability` 等）的 prompt 数；
/// - `invalid` 是无法解析、被跳过的 prompt；
/// - `needs_review` 是解析成功但运行时行为可能不符合用户预期的 prompt
///   （例如 `enabled=false`、`content` 为空等）。
///
/// 不变式守护：
/// - 报告只描述 RP 角色平面的 preset 数据来源，不写入 agent 协调器脚手架；
/// - canonical 输出与 raw sidecar 分离，runtime 只读 canonical；
/// - ST-only 字段不进入运行时执行。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PresetImportReport {
    /// Source-level shape error. Entry-level errors remain in `invalid`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_error: Option<String>,
    /// 检测到的源格式版本：`v2_canonical` / `v1_legacy` / `unknown`。
    pub format_version: String,
    /// 源 JSON 的 SHA256 前 12 个 hex 字符，便于 audit trail 比对。
    pub source_hash: String,
    /// 转换器版本（[`PRESET_CONVERTER_VERSION`]）。
    pub converter_version: String,
    /// 导入完成时的 RFC3339 时间戳。
    pub imported_at: String,
    /// 源 JSON 中的 prompt 总数。
    pub total_input: usize,
    /// 成功转换为 canonical TavernPrompt 的条目数。
    pub converted: usize,
    /// 使用了 SillyTavern 别名字段或被默认值兜底的条目数（`openai_max_tokens` /
    /// `openai_model` 顶层别名，或 prompt 缺省 `enabled` 走默认 true）。
    pub aliases_normalized: usize,
    /// 在 raw sidecar 中保留了 ST-only 字段的 prompt 数（即 prompt 对象含
    /// `PROMPT_CONSUMED` 之外的键，或顶层含 `PRESET_TOP_LEVEL_CONSUMED` 之外
    /// 的键）。
    pub advisory_preserved: usize,
    /// 检测到的顶层模型参数名（`temperature` / `max_tokens` / `model`）。
    pub top_level_params: Vec<String>,
    /// 无法解析、被跳过的 prompt（含原因）。
    pub invalid: Vec<PresetPromptDiagnostic>,
    /// 需人工复核的 prompt（含原因）。不阻塞写入。
    pub needs_review: Vec<PresetPromptDiagnostic>,
}

impl PresetImportReport {
    /// 是否有 source / entry 级错误或需要人工复核的条目。
    pub fn has_issues(&self) -> bool {
        self.source_error.is_some() || !self.invalid.is_empty() || !self.needs_review.is_empty()
    }

    /// 成功导入的 prompt 数（= converted）。
    pub fn imported_count(&self) -> usize {
        self.converted
    }

    /// 被跳过的 prompt 数（= invalid 条数）。
    pub fn skipped_count(&self) -> usize {
        self.invalid.len()
    }

    /// 返回阻止此次 import 写入的原因。空 `prompts` 数组是合法的（视为显式清空），
    /// 仅当 source 不可解析或顶层形状错误时拒绝写入。
    pub fn replacement_error(&self) -> Option<String> {
        if let Some(reason) = &self.source_error {
            return Some(reason.clone());
        }
        None
    }
}

/// 单条 prompt 的诊断信息（#115 P1）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PresetPromptDiagnostic {
    /// 在源 JSON `prompts` 数组中的位置索引。
    pub index: usize,
    /// 条目的 `identifier` 字段（如有），便于人工定位。
    pub identifier: Option<String>,
    /// 条目的 `name` 字段（如有），便于人工定位。
    pub name: Option<String>,
    /// 诊断原因。
    pub reason: String,
}

fn extract_identifier_name(
    prompt: &serde_json::Map<String, Value>,
) -> (Option<String>, Option<String>) {
    let nonblank_string = |field| {
        prompt
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    (nonblank_string("identifier"), nonblank_string("name"))
}

// ── Normalization ───────────────────────────────────────────────────────────

/// 把 SillyTavern preset JSON 归一化为 AIRP canonical [`TavernPreset`]，
/// 并返回诊断报告（#115 P1）。
///
/// 接受以下输入形式（自动探测）：
/// - SillyTavern v2 preset：`{ "prompts": [...], "prompt_order": [...], "temperature": ..., ... }`
/// - 老 v1 / 平铺 preset：`{ "prompts": [...] }`（无 `prompt_order`）
/// - canonical AIRP preset（幂等：输出等价 preset）
///
/// 输出说明：
/// - canonical [`TavernPreset`] 只含 `prompts` / `temperature` / `max_tokens` /
///   `model` 四类字段，是 runtime 唯一消费的形态；
/// - raw sidecar（由调用方落盘）保留原始 JSON 无损；
/// - 报告统计 ST-only 字段的保留情况，runtime 不消费这些字段。
///
/// invalid 条目被跳过（不计入 canonical `prompts`），其余继续处理。
/// `needs_review` 不阻塞写入。
pub fn normalize_preset(source: &Value) -> (TavernPreset, PresetImportReport) {
    let mut report = PresetImportReport {
        converter_version: PRESET_CONVERTER_VERSION.to_string(),
        imported_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    };

    // 源 hash：用于 audit trail，不参与 runtime 决策。
    let source_bytes = serde_json::to_vec(source).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&source_bytes);
    let digest = hasher.finalize();
    report.source_hash = format!("{:x}", digest)[..12].to_string();

    let Some(obj) = source.as_object() else {
        report.source_error = Some("preset JSON 顶层必须是对象".to_string());
        report.format_version = "unknown".to_string();
        return (
            TavernPreset {
                prompts: None,
                temperature: None,
                max_tokens: None,
                model: None,
            },
            report,
        );
    };

    // 探测格式版本：v2 canonical 有 prompt_order 或 ST 风格的扩展字段；
    // v1 legacy 仅有 prompts 数组；canonical AIRP 是 v1 的子集。
    let has_st_extensions = obj
        .keys()
        .any(|k| !PRESET_TOP_LEVEL_CONSUMED.contains(&k.as_str()));
    report.format_version = if has_st_extensions {
        "v2_canonical".to_string()
    } else {
        "v1_legacy".to_string()
    };

    // 顶层参数检测（含 ST 别名归一化）。
    let mut top_level_params: Vec<String> = Vec::new();
    if obj.contains_key("temperature") {
        top_level_params.push("temperature".to_string());
    }
    if obj.contains_key("max_tokens") || obj.contains_key("openai_max_tokens") {
        top_level_params.push("max_tokens".to_string());
    }
    if obj.contains_key("model") || obj.contains_key("openai_model") {
        top_level_params.push("model".to_string());
    }
    report.top_level_params = top_level_params;

    // 反序列化为 canonical TavernPreset 前先过滤无效条目。
    //
    // 原实现直接对 source 做 serde 反序列化：任一 prompt 缺 identifier/name
    // 会让 serde 拒绝整个 preset，导致下方 per-prompt 的 invalid 诊断分支永远
    // 不可达，`PresetImportReport.invalid` 恒为空，diagnostics 形同虚设。
    //
    // 修复（#115 diagnostics 要求）：先用宽松 Value 操作识别无效条目并记录
    // 到 `invalid`，构建只含有效条目的 source 副本交给 serde。这样 serde 不会
    // 因无效条目失败，invalid 分支真正可达；canonical 输出只含有效 prompt。
    let source_prompts: Vec<&Value> = match obj.get("prompts") {
        None => Vec::new(),
        Some(Value::Array(prompts)) => prompts.iter().collect(),
        Some(_) => {
            report.source_error = Some("preset JSON 顶层 `prompts` 字段必须是数组".to_string());
            return (
                TavernPreset {
                    prompts: None,
                    temperature: None,
                    max_tokens: None,
                    model: None,
                },
                report,
            );
        }
    };
    report.total_input = source_prompts.len();

    let mut valid_prompts: Vec<(usize, &Value)> = Vec::new();
    for (idx, src_prompt) in source_prompts.iter().enumerate() {
        let Some(p_obj) = src_prompt.as_object() else {
            report.invalid.push(PresetPromptDiagnostic {
                index: idx,
                identifier: None,
                name: None,
                reason: "prompt 必须是对象".to_string(),
            });
            continue;
        };
        let (identifier, name) = extract_identifier_name(p_obj);

        // identifier 缺失或仅空白 → invalid（canonical 运行时需要稳定的非空 ID）。
        if identifier.is_none() {
            report.invalid.push(PresetPromptDiagnostic {
                index: idx,
                identifier: None,
                name: name.clone(),
                reason: "prompt 缺少有效的 identifier 字段".to_string(),
            });
            continue;
        }

        // name 缺失或仅空白 → invalid。
        if name.is_none() {
            report.invalid.push(PresetPromptDiagnostic {
                index: idx,
                identifier: identifier.clone(),
                name: None,
                reason: "prompt 缺少有效的 name 字段".to_string(),
            });
            continue;
        }

        // 其它 TavernPrompt 字段也必须逐条校验。若留到整个 preset 的 serde
        // 阶段才失败，一个坏条目会再次拒绝全部合法兄弟条目，使 invalid 诊断失效。
        if let Err(error) = serde_json::from_value::<TavernPrompt>((*src_prompt).clone()) {
            report.invalid.push(PresetPromptDiagnostic {
                index: idx,
                identifier,
                name,
                reason: format!("prompt 不符合 TavernPrompt schema: {error}"),
            });
            continue;
        }

        valid_prompts.push((idx, src_prompt));
    }

    // 构建过滤后的 source 供 serde 反序列化：保留顶层字段（temperature/model 等），
    // 只替换 prompts 数组为有效条目。serde 已通过 #[serde(alias = ...)] 消费 ST
    // 别名（openai_max_tokens / openai_model）。
    let mut filtered = source.clone();
    if let Some(filtered_obj) = filtered.as_object_mut() {
        let valid_values: Vec<Value> = valid_prompts.iter().map(|(_, v)| (*v).clone()).collect();
        filtered_obj.insert("prompts".to_string(), Value::Array(valid_values));
    }
    let canonical: TavernPreset = match serde_json::from_value(filtered) {
        Ok(p) => p,
        Err(e) => {
            report.source_error = Some(format!("preset JSON 不符合 TavernPreset schema: {e}"));
            return (
                TavernPreset {
                    prompts: None,
                    temperature: None,
                    max_tokens: None,
                    model: None,
                },
                report,
            );
        }
    };

    // 对每条有效 prompt 做 needs_review / aliases / advisory 诊断。
    let canonical_prompts = canonical.prompts.clone().unwrap_or_default();
    let mut converted_count = 0usize;
    let mut aliases_count_for_report =
        if obj.contains_key("openai_max_tokens") || obj.contains_key("openai_model") {
            1
        } else {
            0
        };
    let mut advisory_count_for_report = if has_st_extensions { 1 } else { 0 };

    for (idx, src_prompt) in valid_prompts.iter() {
        let p_obj = src_prompt
            .as_object()
            .expect("valid_prompts only holds object entries; non-objects go to invalid");
        let (identifier, name) = extract_identifier_name(p_obj);

        // 该 prompt 已成功转换（进入 canonical）。
        converted_count += 1;

        // aliases_normalized：prompt 缺省 enabled（serde 默认 true）。
        if !p_obj.contains_key("enabled") {
            aliases_count_for_report += 1;
        }

        // advisory_preserved：prompt 含 PROMPT_CONSUMED 之外的键。
        let prompt_has_advisory = p_obj.keys().any(|k| !PROMPT_CONSUMED.contains(&k.as_str()));
        if prompt_has_advisory {
            advisory_count_for_report += 1;
        }

        // needs_review：enabled=false 或 content 缺失/空。
        let enabled_val = p_obj.get("enabled").and_then(|v| v.as_bool());
        let content_val = p_obj.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(enabled_val, Some(false)) {
            report.needs_review.push(PresetPromptDiagnostic {
                index: *idx,
                identifier: identifier.clone(),
                name: name.clone(),
                reason: "prompt enabled=false，运行时不会被装配".to_string(),
            });
        } else if content_val.trim().is_empty() {
            report.needs_review.push(PresetPromptDiagnostic {
                index: *idx,
                identifier: identifier.clone(),
                name: name.clone(),
                reason: "prompt content 为空或仅空白，运行时不会注入文本".to_string(),
            });
        }
    }

    report.converted = converted_count;
    report.aliases_normalized = aliases_count_for_report;
    report.advisory_preserved = advisory_count_for_report;

    // sanity: canonical_prompts.len() 应该 == converted_count。
    // 不一致说明 serde 在反序列化阶段已经丢弃了 invalid prompts；这种情况下
    // canonical 仍然合法，但 report.invalid 已经记录了原因。
    debug_assert_eq!(
        canonical_prompts.len(),
        converted_count,
        "canonical prompts count must match converted count"
    );

    (canonical, report)
}

/// 解析并拼接预设 JSON 生成的 Prompts。
pub fn assemble_preset_prompts(
    preset_json: &str,
    enabled_override_ids: Option<&Vec<String>>,
    char_name: &str,
    user_name: &str,
    last_message: &str,
) -> String {
    let preset: TavernPreset = match serde_json::from_str(preset_json) {
        Ok(p) => p,
        Err(e) => {
            // M0 F-46 / 6.0l：解析失败不再静默吞错
            tracing::warn!(err = %e, "TavernPreset JSON 解析失败，回退到空预设");
            return String::new();
        }
    };

    let Some(prompts) = preset.prompts else {
        return String::new();
    };

    let mut full_prompt = String::new();
    for p in prompts {
        let is_enabled = if let Some(overrides) = enabled_override_ids {
            overrides.contains(&p.identifier)
        } else {
            p.enabled
        };
        if is_enabled {
            if let Some(ref content) = p.content {
                full_prompt.push_str(content);
                full_prompt.push('\n');
            }
        }
    }

    render_macros(&full_prompt, char_name, user_name, last_message)
}

/// 执行预设的宏变量解析（setvar/getvar、char/user/lastUserMessage）。
pub fn render_macros(text: &str, char_name: &str, user_name: &str, last_message: &str) -> String {
    let mut rendered = text.to_string();

    rendered = rendered.replace("{{char}}", char_name);
    rendered = rendered.replace("{{user}}", user_name);
    rendered = rendered.replace("{{lastUserMessage}}", last_message);

    // 提取 setvar 并存入临时 Map，清除原文中的 setvar
    let mut variables: HashMap<String, String> = HashMap::new();
    for cap in SETVAR_RE.captures_iter(&rendered.clone()) {
        variables.insert(cap[1].to_string(), cap[2].to_string());
    }
    rendered = SETVAR_RE.replace_all(&rendered, "").to_string();

    // 替换 getvar
    rendered = GETVAR_RE
        .replace_all(&rendered, |caps: &regex::Captures| {
            variables.get(&caps[1]).cloned().unwrap_or_default()
        })
        .to_string();

    rendered
}

// ── PresetService（#115 P1 第二阶段：agent tool 共享数据访问层）──────────────
//
// 与 `LorebookService` 对齐：封装 normalize + canonical/raw 落盘 + 读取，
// 供 daemon handler 和 agent tool 共享调用。本 PR 只在 agent tool 侧接入；
// handler 侧接入留 #174 合并后的去重 PR。

static PRESET_WRITE_LOCK: Lazy<std::sync::Mutex<()>> = Lazy::new(|| std::sync::Mutex::new(()));

/// Preset 数据访问层。封装归一化、canonical/raw 落盘与读取。
pub struct PresetService {
    data_root: std::path::PathBuf,
}

impl PresetService {
    pub fn new(data_root: impl AsRef<std::path::Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    /// 读 canonical preset.json（优先 normalized 路径，回退 legacy）。
    pub fn read(&self, preset_id: &crate::types::PresetId) -> Result<TavernPreset, AirpError> {
        let normalized = crate::data_dir::preset_json_path(&self.data_root, preset_id.as_str());
        let legacy = crate::data_dir::legacy_preset_json_path(&self.data_root, preset_id.as_str());
        let path = if normalized.exists() {
            normalized
        } else {
            legacy
        };
        if !path.exists() {
            return Err(AirpError::NotFound(format!(
                "preset {} not found",
                preset_id
            )));
        }
        let json_str = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&json_str)?)
    }

    /// 写 canonical preset.json + raw.json sidecar。
    /// 允许覆盖（destructive update 语义，与 import_preset 的拒绝覆盖不同）。
    pub fn write(
        &self,
        preset_id: &crate::types::PresetId,
        source_json: &str,
    ) -> Result<(TavernPreset, PresetImportReport), AirpError> {
        let cleaned = crate::data_dir::strip_utf8_bom(source_json);
        let source: Value = serde_json::from_str(cleaned)
            .map_err(|e| AirpError::BadRequest(format!("preset JSON 无效: {}", e)))?;
        let (canonical, report) = normalize_preset(&source);
        if let Some(reason) = report.replacement_error() {
            return Err(AirpError::BadRequest(format!("preset 无法导入: {reason}")));
        }
        let canonical_bytes = serde_json::to_vec_pretty(&canonical)?;
        let raw_bytes = cleaned.as_bytes();

        let _guard = PRESET_WRITE_LOCK
            .lock()
            .expect("preset write lock poisoned");
        let dir = self.data_root.join("presets").join(preset_id.as_str());
        let generation = format!(
            "{}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            report.source_hash
        );
        let version_dir = dir.join("versions").join(&generation);
        std::fs::create_dir_all(&version_dir)?;
        crate::data_dir::replace_file(&version_dir.join("preset.json"), &canonical_bytes)?;
        crate::data_dir::replace_file(&version_dir.join("raw.json"), raw_bytes)?;
        // Both immutable files exist before the single atomic pointer switch. Old versions are
        // retained so readers that resolved the previous pointer can finish safely.
        crate::data_dir::replace_file(&dir.join("current"), generation.as_bytes())?;
        Ok((canonical, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_rendering() {
        let preset_json = r#"{
            "prompts": [
                {
                    "identifier": "init-vars",
                    "name": "Init Variables",
                    "enabled": true,
                    "role": "system",
                    "content": "{{setvar::style::日轻}}{{setvar::writer::{{char}}}}"
                },
                {
                    "identifier": "main-prompt",
                    "name": "Main Prompt",
                    "enabled": true,
                    "role": "system",
                    "content": "作者是{{getvar::writer}}。写作风格是{{getvar::style}}。最后一句话是: {{lastUserMessage}}"
                },
                {
                    "identifier": "disabled-prompt",
                    "name": "Disabled",
                    "enabled": false,
                    "role": "system",
                    "content": "这里是禁用的条目"
                }
            ]
        }"#;

        let result = assemble_preset_prompts(preset_json, None, "Konata", "User", "你好啊");

        assert!(result.contains("作者是Konata"));
        assert!(result.contains("写作风格是日轻"));
        assert!(result.contains("最后一句话是: 你好啊"));
        assert!(!result.contains("这里是禁用的条目"));
    }

    // ── normalize_preset diagnostics（#115 P1） ──────────────────────────────

    fn valid_v1_preset_source() -> serde_json::Value {
        serde_json::json!({
            "prompts": [
                {
                    "identifier": "main",
                    "name": "Main",
                    "role": "system",
                    "content": "hello"
                }
            ]
        })
    }

    #[test]
    fn normalize_preset_detects_v1_legacy_format() {
        let source = valid_v1_preset_source();
        let (canonical, report) = normalize_preset(&source);

        assert_eq!(report.format_version, "v1_legacy");
        assert!(report.source_error.is_none());
        assert_eq!(report.total_input, 1);
        assert_eq!(report.converted, 1);
        assert!(report.invalid.is_empty());
        assert!(report.needs_review.is_empty());
        assert_eq!(canonical.prompts.as_ref().unwrap().len(), 1);
        // source_hash 是 12 位 hex 前缀
        assert_eq!(report.source_hash.len(), 12);
        assert!(report.source_hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(report.converter_version, PRESET_CONVERTER_VERSION);
    }

    #[test]
    fn normalize_preset_detects_v2_canonical_format_with_prompt_order() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "p1", "name": "P1", "role": "system", "content": "a"}
            ],
            "prompt_order": [{"character_id": "main", "order": ["p1"]}],
            "temperature": 0.7
        });
        let (canonical, report) = normalize_preset(&source);

        assert_eq!(report.format_version, "v2_canonical");
        assert_eq!(report.top_level_params, vec!["temperature".to_string()]);
        assert!(report.source_error.is_none());
        assert_eq!(report.converted, 1);
        // prompt_order 是 ST-only 顶层字段 → advisory_preserved +1（顶层）
        assert_eq!(report.advisory_preserved, 1);
        assert_eq!(canonical.temperature, Some(0.7));
    }

    #[test]
    fn normalize_preset_rejects_non_object_top_level() {
        let source = serde_json::json!(["not", "an", "object"]);
        let (canonical, report) = normalize_preset(&source);

        assert_eq!(report.format_version, "unknown");
        assert!(report.source_error.is_some());
        assert!(report
            .source_error
            .as_ref()
            .unwrap()
            .contains("顶层必须是对象"));
        assert!(canonical.prompts.is_none());
    }

    #[test]
    fn normalize_preset_rejects_non_array_prompts_as_source_error() {
        let source = serde_json::json!({"prompts": null});
        let (canonical, report) = normalize_preset(&source);

        assert!(report
            .source_error
            .as_deref()
            .is_some_and(|reason| reason.contains("必须是数组")));
        assert!(report.replacement_error().is_some());
        assert!(canonical.prompts.is_none());
    }

    #[test]
    fn normalize_preset_records_missing_identifier_as_invalid() {
        let source = serde_json::json!({
            "prompts": [
                {"name": "NoId", "role": "system", "content": "x"},
                {"identifier": "ok", "name": "Ok", "role": "system", "content": "y"}
            ]
        });
        let (canonical, report) = normalize_preset(&source);

        assert_eq!(report.total_input, 2);
        assert_eq!(report.converted, 1);
        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].index, 0);
        assert_eq!(report.invalid[0].name.as_deref(), Some("NoId"));
        assert!(report.invalid[0].reason.contains("identifier"));
        // invalid 条目不进入 canonical
        let canonical_prompts = canonical.prompts.as_ref().unwrap();
        assert_eq!(canonical_prompts.len(), 1);
        assert_eq!(canonical_prompts[0].identifier, "ok");
    }

    #[test]
    fn normalize_preset_records_missing_name_as_invalid() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "no-name", "role": "system", "content": "x"}
            ]
        });
        let (_canonical, report) = normalize_preset(&source);

        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].index, 0);
        assert_eq!(report.invalid[0].identifier.as_deref(), Some("no-name"));
        assert!(report.invalid[0].reason.contains("name"));
    }

    #[test]
    fn normalize_preset_records_blank_identifier_and_name_as_invalid() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "   ", "name": "BlankId", "content": "x"},
                {"identifier": "blank-name", "name": "\t", "content": "y"}
            ]
        });
        let (canonical, report) = normalize_preset(&source);

        assert_eq!(report.total_input, 2);
        assert_eq!(report.converted, 0);
        assert_eq!(report.invalid.len(), 2);
        assert!(report.invalid[0].reason.contains("identifier"));
        assert!(report.invalid[1].reason.contains("name"));
        assert!(canonical.prompts.as_ref().unwrap().is_empty());
    }

    #[test]
    fn normalize_preset_skips_prompt_with_invalid_field_type() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "bad", "name": "Bad", "enabled": "yes", "content": "x"},
                {"identifier": "ok", "name": "Ok", "enabled": true, "content": "y"}
            ]
        });
        let (canonical, report) = normalize_preset(&source);

        assert!(report.source_error.is_none());
        assert_eq!(report.total_input, 2);
        assert_eq!(report.converted, 1);
        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].identifier.as_deref(), Some("bad"));
        assert!(report.invalid[0].reason.contains("schema"));
        let prompts = canonical.prompts.as_ref().unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].identifier, "ok");
    }

    #[test]
    fn normalize_preset_records_non_object_prompt_as_invalid() {
        let source = serde_json::json!({
            "prompts": [
                "not-an-object",
                {"identifier": "ok", "name": "Ok", "role": "system", "content": "y"}
            ]
        });
        let (_canonical, report) = normalize_preset(&source);

        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].index, 0);
        assert!(report.invalid[0].reason.contains("对象"));
        assert_eq!(report.converted, 1);
    }

    #[test]
    fn normalize_preset_flags_disabled_prompt_as_needs_review() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "disabled", "name": "Disabled", "enabled": false, "role": "system", "content": "x"}
            ]
        });
        let (_canonical, report) = normalize_preset(&source);

        assert_eq!(report.converted, 1);
        assert_eq!(report.needs_review.len(), 1);
        assert_eq!(
            report.needs_review[0].identifier.as_deref(),
            Some("disabled")
        );
        assert!(report.needs_review[0].reason.contains("enabled=false"));
    }

    #[test]
    fn normalize_preset_flags_empty_content_as_needs_review() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "empty", "name": "Empty", "enabled": true, "role": "system", "content": "   "}
            ]
        });
        let (_canonical, report) = normalize_preset(&source);

        assert_eq!(report.needs_review.len(), 1);
        assert!(report.needs_review[0].reason.contains("content"));
    }

    #[test]
    fn normalize_preset_counts_missing_enabled_as_alias_normalized() {
        let source = serde_json::json!({
            "prompts": [
                {"identifier": "no-enabled", "name": "NoEnabled", "role": "system", "content": "x"},
                {"identifier": "has-enabled", "name": "HasEnabled", "enabled": true, "role": "system", "content": "y"}
            ]
        });
        let (_canonical, report) = normalize_preset(&source);

        // 只有第一条缺 enabled → aliases_normalized += 1
        assert_eq!(report.aliases_normalized, 1);
    }

    #[test]
    fn normalize_preset_counts_top_level_alias_normalization() {
        let source = serde_json::json!({
            "prompts": [
                // prompt 显式带 enabled，隔离顶层 alias 计数
                {"identifier": "p1", "name": "P1", "enabled": true, "role": "system", "content": "x"}
            ],
            "openai_max_tokens": 4096,
            "openai_model": "gpt-4o"
        });
        let (canonical, report) = normalize_preset(&source);

        // 顶层使用了任一 ST 别名 → aliases_normalized +1（顶层；prompt 带了 enabled 不再加）
        assert_eq!(report.aliases_normalized, 1);
        assert_eq!(report.top_level_params, vec!["max_tokens", "model"]);
        // serde alias 把 openai_max_tokens/openai_model 归一化到 canonical 字段
        assert_eq!(canonical.max_tokens, Some(4096));
        assert_eq!(canonical.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn normalize_preset_counts_advisory_preserved_for_st_only_prompt_fields() {
        let source = serde_json::json!({
            "prompts": [
                {
                    "identifier": "p1",
                    "name": "P1",
                    "role": "system",
                    "content": "x",
                    "injection_position": 0,
                    "probability": 100
                }
            ]
        });
        let (_canonical, report) = normalize_preset(&source);

        // prompt 含 PROMPT_CONSUMED 之外的字段 → advisory_preserved += 1
        assert_eq!(report.advisory_preserved, 1);
    }

    #[test]
    fn normalize_preset_source_hash_is_stable_for_identical_input() {
        let source = valid_v1_preset_source();
        let (_, r1) = normalize_preset(&source);
        let (_, r2) = normalize_preset(&source);

        assert_eq!(r1.source_hash, r2.source_hash);
    }

    #[test]
    fn normalize_preset_replacement_error_only_on_source_error() {
        // 合法空 prompts 数组：视为显式清空，不拒绝写入
        let source = serde_json::json!({"prompts": []});
        let (canonical, report) = normalize_preset(&source);

        assert!(report.replacement_error().is_none());
        assert_eq!(report.converted, 0);
        assert!(canonical.prompts.as_ref().unwrap().is_empty());

        // source_error 存在时才拒绝
        let (_, bad_report) = normalize_preset(&serde_json::json!(42));
        assert!(bad_report.replacement_error().is_some());
    }

    #[test]
    fn normalize_preset_has_issues_reflects_invalid_and_needs_review() {
        // 干净 preset：无 issues
        let clean = valid_v1_preset_source();
        let (_, clean_report) = normalize_preset(&clean);
        assert!(!clean_report.has_issues());

        // 有 invalid：has_issues = true
        let with_invalid = serde_json::json!({
            "prompts": [{"name": "NoId", "role": "system", "content": "x"}]
        });
        let (_, invalid_report) = normalize_preset(&with_invalid);
        assert!(invalid_report.has_issues());

        // 有 needs_review：has_issues = true
        let with_review = serde_json::json!({
            "prompts": [{"identifier": "x", "name": "X", "enabled": false, "role": "system", "content": "y"}]
        });
        let (_, review_report) = normalize_preset(&with_review);
        assert!(review_report.has_issues());
    }
}
