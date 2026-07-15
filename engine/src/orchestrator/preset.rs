use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

use super::card::{TavernPreset, TavernPrompt};

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
        return (TavernPreset {
            prompts: None,
            temperature: None,
            max_tokens: None,
            model: None,
        }, report);
    };

    // 探测格式版本：v2 canonical 有 prompt_order 或 ST 风格的扩展字段；
    // v1 legacy 仅有 prompts 数组；canonical AIRP 是 v1 的子集。
    let has_prompt_order = obj.contains_key("prompt_order");
    let has_st_extensions = obj
        .keys()
        .any(|k| !PRESET_TOP_LEVEL_CONSUMED.contains(&k.as_str()));
    report.format_version = if has_prompt_order || has_st_extensions {
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

    // 顶层别名归一化计数：使用了 openai_max_tokens / openai_model 任一别名。
    if obj.contains_key("openai_max_tokens") || obj.contains_key("openai_model") {
        report.aliases_normalized += 1;
    }

    // 顶层 advisory 保留计数：存在 PRESET_TOP_LEVEL_CONSUMED 之外的键。
    let top_advisory = obj
        .keys()
        .any(|k| !PRESET_TOP_LEVEL_CONSUMED.contains(&k.as_str()));
    if top_advisory {
        report.advisory_preserved += 1;
    }

    // 反序列化为 canonical TavernPreset。serde 已通过 #[serde(alias = ...)]
    // 消费 ST 别名（openai_max_tokens / openai_model）。
    let canonical: TavernPreset = match serde_json::from_value(source.clone()) {
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

    // 对每条 prompt 做诊断。
    let source_prompts: Vec<&Value> = obj
        .get("prompts")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().collect())
        .unwrap_or_default();
    report.total_input = source_prompts.len();

    // 同时检查 canonical prompts 与 source prompts 的对应关系。
    let canonical_prompts = canonical.prompts.clone().unwrap_or_default();
    let mut converted_count = 0usize;
    let mut aliases_count_for_report = if obj.contains_key("openai_max_tokens")
        || obj.contains_key("openai_model")
    {
        1
    } else {
        0
    };
    let mut advisory_count_for_report = if top_advisory { 1 } else { 0 };

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

        let identifier = p_obj
            .get("identifier")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let name = p_obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // identifier 缺失 → invalid（canonical TavernPrompt 要求 identifier:String）。
        if identifier.is_none() {
            report.invalid.push(PresetPromptDiagnostic {
                index: idx,
                identifier: None,
                name: name.clone(),
                reason: "prompt 缺少 identifier 字段".to_string(),
            });
            continue;
        }

        // name 缺失 → invalid。
        if name.is_none() {
            report.invalid.push(PresetPromptDiagnostic {
                index: idx,
                identifier: identifier.clone(),
                name: None,
                reason: "prompt 缺少 name 字段".to_string(),
            });
            continue;
        }

        // 该 prompt 已成功转换（进入 canonical）。
        converted_count += 1;

        // aliases_normalized：prompt 缺省 enabled（serde 默认 true）。
        if !p_obj.contains_key("enabled") {
            aliases_count_for_report += 1;
        }

        // advisory_preserved：prompt 含 PROMPT_CONSUMED 之外的键。
        let prompt_has_advisory = p_obj
            .keys()
            .any(|k| !PROMPT_CONSUMED.contains(&k.as_str()));
        if prompt_has_advisory {
            advisory_count_for_report += 1;
        }

        // needs_review：enabled=false 或 content 缺失/空。
        let enabled_val = p_obj.get("enabled").and_then(|v| v.as_bool());
        let content_val = p_obj
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if matches!(enabled_val, Some(false)) {
            report.needs_review.push(PresetPromptDiagnostic {
                index: idx,
                identifier: identifier.clone(),
                name: name.clone(),
                reason: "prompt enabled=false，运行时不会被装配".to_string(),
            });
        } else if content_val.trim().is_empty() {
            report.needs_review.push(PresetPromptDiagnostic {
                index: idx,
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
}
