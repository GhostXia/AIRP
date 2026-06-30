//! M_PR PR-4: SillyTavern 正则脚本解析与应用。
//!
//! 解析 `presets/{id}/regex/*.json` 中的 SillyTavern 正则脚本格式，
//! 筛选出 placement 含 AI Output (2) 且未禁用的脚本，转换为 `RegexFilter`
//! 注入 FSM 链。仅处理 `replaceString == ""`（即「隐藏」用途）的脚本；
//! 非空替换交给 LLM Agent (PR-6+) 处理。

use crate::error::AirpError;
use crate::fsm::RegexFilter;
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// SillyTavern 正则脚本单条规则。字段名匹配酒馆社区导出格式（驼峰）。
#[derive(Debug, Clone, Deserialize)]
pub struct SillyTavernRegexScript {
    /// 脚本显示名（PR-6 用于 Filter Agent prompt 注释）。
    #[serde(rename = "scriptName", default)]
    #[allow(dead_code)] // PR-6 Filter Agent prompt 注入时使用
    pub script_name: String,
    #[serde(rename = "findRegex", default)]
    pub find_regex: String,
    #[serde(rename = "replaceString", default)]
    pub replace_string: String,
    #[serde(default)]
    pub placement: Vec<i32>,
    #[serde(default)]
    pub disabled: bool,
}

/// SillyTavern placement 枚举值：AI 输出。
const PLACEMENT_AI_OUTPUT: i32 = 2;

/// PR-4: 加载某预设关联的所有正则脚本。
///
/// 扫 `presets/{preset_id}/regex/*.json`；每个 JSON 文件可以是：
///   1. 单个脚本对象（SillyTavern 单文件导出）
///   2. 脚本数组（用户合并多脚本）
///
/// 单文件解析失败仅 tracing::warn，不阻塞其它脚本加载。
pub fn load_preset_regex_scripts(
    root: &Path,
    preset_id: &str,
) -> Result<Vec<SillyTavernRegexScript>, AirpError> {
    let regex_dir = root.join("presets").join(preset_id).join("regex");
    if !regex_dir.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&regex_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(path = ?path, err = %e, "PR-4: 读取正则脚本失败");
                continue;
            }
        };
        let cleaned = crate::data_dir::strip_utf8_bom(&raw);

        // 优先尝试数组；失败则尝试单对象
        if let Ok(arr) = serde_json::from_str::<Vec<SillyTavernRegexScript>>(cleaned) {
            out.extend(arr);
        } else if let Ok(single) = serde_json::from_str::<SillyTavernRegexScript>(cleaned) {
            out.push(single);
        } else {
            tracing::warn!(path = ?path, "PR-4: 正则脚本 JSON 解析失败，跳过");
        }
    }
    Ok(out)
}

/// PR-4: 将 SillyTavern 脚本筛选转换为 FSM `RegexFilter`。
///
/// 注入条件：
///   1. `disabled == false`
///   2. `placement` 含 AI Output (2)
///   3. `replace_string` 为空（纯隐藏用途；非空替换需走 LLM Agent）
pub fn scripts_to_filters(scripts: &[SillyTavernRegexScript]) -> Vec<RegexFilter> {
    scripts
        .iter()
        .filter(|s| !s.disabled)
        .filter(|s| s.placement.contains(&PLACEMENT_AI_OUTPUT))
        .filter(|s| s.replace_string.is_empty())
        .map(|s| {
            let pattern = strip_regex_delimiters(&s.find_regex);
            RegexFilter::from_regex(&pattern)
        })
        .collect()
}

/// 剥除 SillyTavern `findRegex` 的 `/pattern/flags` 包裹。
///
/// 例如：`/<thought>[\\s\\S]*?<\\/thought>/gi` → `<thought>[\\s\\S]*?<\\/thought>`。
/// 无包裹时（裸 pattern）原样返回。
fn strip_regex_delimiters(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(stripped) = trimmed.strip_prefix('/') {
        if let Some(last_slash) = stripped.rfind('/') {
            // last_slash 是 stripped 中最后 / 的位置（即 / flags 分隔点）
            return stripped[..last_slash].to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const SCRIPT_HIDE_THOUGHT: &str = r#"{
        "scriptName": "Hide Thoughts",
        "findRegex": "/<thought>[\\s\\S]*?<\\/thought>/gi",
        "replaceString": "",
        "placement": [2],
        "disabled": false
    }"#;

    const SCRIPT_DISABLED: &str = r#"{
        "scriptName": "Disabled Hide",
        "findRegex": "/<status>[\\s\\S]*?<\\/status>/g",
        "replaceString": "",
        "placement": [2],
        "disabled": true
    }"#;

    const SCRIPT_USER_INPUT: &str = r#"{
        "scriptName": "User Only",
        "findRegex": "/foo/g",
        "replaceString": "",
        "placement": [1],
        "disabled": false
    }"#;

    const SCRIPT_NON_EMPTY_REPLACE: &str = r#"{
        "scriptName": "Replace Word",
        "findRegex": "/old/g",
        "replaceString": "new",
        "placement": [2],
        "disabled": false
    }"#;

    #[test]
    fn test_pr4_strip_delimiters_with_flags() {
        assert_eq!(
            strip_regex_delimiters("/<thought>[\\s\\S]*?<\\/thought>/gi"),
            "<thought>[\\s\\S]*?<\\/thought>"
        );
    }

    #[test]
    fn test_pr4_strip_delimiters_no_flags() {
        assert_eq!(strip_regex_delimiters("/foo/"), "foo");
    }

    #[test]
    fn test_pr4_strip_delimiters_bare_pattern() {
        // 无包裹的纯 pattern 原样返回
        assert_eq!(strip_regex_delimiters("foo"), "foo");
    }

    #[test]
    fn test_pr4_load_single_object() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let regex_dir = root.join("presets").join("test_preset").join("regex");
        fs::create_dir_all(&regex_dir).unwrap();
        fs::write(regex_dir.join("a.json"), SCRIPT_HIDE_THOUGHT).unwrap();

        let scripts = load_preset_regex_scripts(root, "test_preset").unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].script_name, "Hide Thoughts");
        assert!(!scripts[0].disabled);
    }

    #[test]
    fn test_pr4_load_array_format() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let regex_dir = root.join("presets").join("test_preset").join("regex");
        fs::create_dir_all(&regex_dir).unwrap();
        let arr = format!("[{},{}]", SCRIPT_HIDE_THOUGHT, SCRIPT_DISABLED);
        fs::write(regex_dir.join("bundle.json"), arr).unwrap();

        let scripts = load_preset_regex_scripts(root, "test_preset").unwrap();
        assert_eq!(scripts.len(), 2);
    }

    #[test]
    fn test_pr4_load_empty_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let scripts = load_preset_regex_scripts(root, "noexist").unwrap();
        assert!(scripts.is_empty());
    }

    #[test]
    fn test_pr4_load_invalid_json_skipped() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let regex_dir = root.join("presets").join("test_preset").join("regex");
        fs::create_dir_all(&regex_dir).unwrap();
        fs::write(regex_dir.join("good.json"), SCRIPT_HIDE_THOUGHT).unwrap();
        fs::write(regex_dir.join("bad.json"), "{ malformed").unwrap();

        let scripts = load_preset_regex_scripts(root, "test_preset").unwrap();
        assert_eq!(scripts.len(), 1, "坏 JSON 应被跳过，好脚本仍加载");
    }

    #[test]
    fn test_pr4_filter_keeps_only_ai_output_enabled_empty_replace() {
        let scripts: Vec<SillyTavernRegexScript> = vec![
            serde_json::from_str(SCRIPT_HIDE_THOUGHT).unwrap(),
            serde_json::from_str(SCRIPT_DISABLED).unwrap(),
            serde_json::from_str(SCRIPT_USER_INPUT).unwrap(),
            serde_json::from_str(SCRIPT_NON_EMPTY_REPLACE).unwrap(),
        ];
        let filters = scripts_to_filters(&scripts);
        // 仅 HIDE_THOUGHT 通过三重筛选
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].start, "<thought>");
        assert_eq!(filters[0].end, "</thought>");
    }

    #[test]
    fn test_pr4_bom_tolerant() {
        // STR-01 复用：脚本 JSON 含 BOM 应被剥除
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let regex_dir = root.join("presets").join("test_preset").join("regex");
        fs::create_dir_all(&regex_dir).unwrap();
        let with_bom = format!("\u{FEFF}{}", SCRIPT_HIDE_THOUGHT);
        fs::write(regex_dir.join("a.json"), with_bom).unwrap();

        let scripts = load_preset_regex_scripts(root, "test_preset").unwrap();
        assert_eq!(scripts.len(), 1);
    }
}
