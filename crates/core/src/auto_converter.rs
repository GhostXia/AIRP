//! Legacy JSON 资产到 Markdown 的一次性转换器。
//!
//! 早期版本以 JSON 存预设和世界书；当前版本读 `.md`。启动期扫描 `data/presets/`
//! 和 `data/characters/*/worldbooks/`，对每个无对应 `.md` 的 JSON 文件做一次性
//! 转换。仅在源 JSON 新于 / 缺失 Markdown 时触发。

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Legacy preset 的单条 prompt 条目。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LegacyPresetPrompt {
    /// 显示名。
    pub name: String,
    /// prompt 文本内容。
    pub value: String,
    /// 是否启用。
    pub enabled: bool,
    /// 唯一标识符（用于 enabled_presets 过滤）。
    pub identifier: String,
}

/// Legacy preset 整体结构（SillyTavern 早期 JSON 格式）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LegacyPreset {
    /// 预设名。
    pub name: String,
    /// prompt 列表。
    pub prompts: Vec<LegacyPresetPrompt>,
}

/// Legacy 世界书条目（SillyTavern 早期 JSON 格式）。
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyLorebookEntry {
    /// 触发关键词列表。
    pub keys: Vec<String>,
    /// 命中后注入的文本。
    pub content: String,
    /// 优先级排序值。
    pub order: Option<i32>,
    /// 注入位置：0 = before user message, 1 = after。
    pub position: Option<i32>,
    /// 是否启用。
    pub enabled: Option<bool>,
}

// 注：原 `LegacyLorebook` 包装结构未使用（实际解析直接走 `serde_json::Value`），
// 6.0n pub(crate) 收紧后 dead_code lint 暴露，移除。

/// Automatically scans the data directory for legacy JSON files (presets, worldbooks)
/// and converts them to formatted Markdown files if the Markdown counterpart does not exist
/// or is older than the JSON file.
pub fn auto_convert_legacy_files(root: &Path) -> Result<(), AirpError> {
    let presets_dir = root.join("presets");
    if presets_dir.exists() {
        let entries = fs::read_dir(&presets_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                let md_path = path.with_extension("md");
                // 用户可能已经手动编辑过 .md，mtime 对比不可靠 (git checkout/clone
                // 会重置 JSON 的 mtime → 覆盖用户成果)。改为"仅当 .md 不存在时转换"。
                if !md_path.exists() {
                    if let Err(e) = convert_preset_json_to_md(&path, &md_path) {
                        tracing::warn!(path = ?path, err = %e, "Failed to convert preset JSON");
                    }
                }
            }
        }
    }

    // Check for character worldbooks to auto-convert
    let chars_dir = root.join("characters");
    if chars_dir.exists() {
        let entries = fs::read_dir(&chars_dir)?;
        for entry in entries.flatten() {
            let char_path = entry.path();
            if !char_path.is_dir() {
                continue;
            }
            // 旧目录：characters/{id}/worldbooks/
            convert_worldbooks_in_dir(&char_path.join("worldbooks"));
            // CF-5：新目录 characters/{id}/world/extra/
            convert_worldbooks_in_dir(&char_path.join("world").join("extra"));
        }
    }

    Ok(())
}

/// CF-5：扫描某目录下的所有 `.json` 世界书并转换为 `.md`（仅当 `.md` 不存在时）。
///
/// 失败仅 tracing::warn，不阻塞其它文件转换。
fn convert_worldbooks_in_dir(dir: &Path) {
    if !dir.exists() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(path = ?dir, err = %err, "无法读取目录，跳过");
            return;
        }
    };
    for wb_entry in entries.flatten() {
        let path = wb_entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            let md_path = path.with_extension("md");
            if !md_path.exists() {
                if let Err(e) = convert_worldbook_json_to_md(&path, &md_path) {
                    tracing::warn!(path = ?path, err = %e, "Failed to convert worldbook JSON");
                }
            }
        }
    }
}

fn convert_preset_json_to_md(json_path: &Path, md_path: &Path) -> Result<(), String> {
    let raw =
        fs::read_to_string(json_path).map_err(|e| format!("Failed to read JSON file: {}", e))?;
    // STR-01: 剥除 PowerShell/Windows 工具写入的 UTF-8 BOM
    let raw = crate::data_dir::strip_utf8_bom(&raw);
    let preset: LegacyPreset =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse Preset JSON: {}", e))?;

    let mut md_content = String::new();
    md_content.push_str("---\n");
    md_content.push_str(&format!(
        "name: \"{}\"\n",
        preset.name.replace('\"', "\\\"")
    ));
    md_content.push_str("---\n\n");
    md_content.push_str(&format!("# {}\n\n", preset.name));
    md_content.push_str(
        "此预设由 Legacy JSON 格式自动转换。您可直接在此 Markdown 中修改条目内容，或增删条目。\n\n",
    );

    for prompt in preset.prompts {
        md_content.push_str(&format!("## [{}] {}\n", prompt.identifier, prompt.name));
        md_content.push_str(&format!("- Enabled: {}\n", prompt.enabled));
        md_content.push_str("- Content:\n```markdown\n");
        // Ensure values don't break markdown code blocks easily
        let mut clean_val = prompt.value;
        if !clean_val.ends_with('\n') {
            clean_val.push('\n');
        }
        md_content.push_str(&clean_val);
        md_content.push_str("```\n\n");
    }

    fs::write(md_path, md_content).map_err(|e| format!("Failed to write MD file: {}", e))?;
    Ok(())
}

fn convert_worldbook_json_to_md(json_path: &Path, md_path: &Path) -> Result<(), String> {
    let raw =
        fs::read_to_string(json_path).map_err(|e| format!("Failed to read JSON file: {}", e))?;
    // STR-01: 剥除 PowerShell/Windows 工具写入的 UTF-8 BOM
    let raw = crate::data_dir::strip_utf8_bom(&raw);

    // Parse dynamically because SillyTavern worldbooks can be structured differently (e.g. {"entries": {...}})
    let json_val: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse Worldbook JSON: {}", e))?;

    let entries_val = if let Some(ent) = json_val.get("entries") {
        ent
    } else {
        &json_val
    };

    let mut parsed_entries = Vec::new();

    if let Some(map) = entries_val.as_object() {
        for (_, entry_val) in map {
            if let Ok(entry) = serde_json::from_value::<LegacyLorebookEntry>(entry_val.clone()) {
                parsed_entries.push(entry);
            }
        }
    } else if let Some(arr) = entries_val.as_array() {
        for entry_val in arr {
            if let Ok(entry) = serde_json::from_value::<LegacyLorebookEntry>(entry_val.clone()) {
                parsed_entries.push(entry);
            }
        }
    }

    // Sort entries by order if present
    parsed_entries.sort_by_key(|e| e.order.unwrap_or(100));

    let mut md_content = String::new();
    md_content.push_str("# 世界书 lorebook\n\n");
    md_content.push_str("此世界书由 Legacy JSON 格式自动转换，以 Markdown 表格渲染，人类与 Agent 可直接读写编辑。\n\n");
    md_content.push_str("| 触发关键词 | 排序 (Order) | 激活位置 (Position) | 启用状态 | 提示词设定内容 (Prompt) |\n");
    md_content.push_str("| :--- | :--- | :--- | :--- | :--- |\n");

    for entry in parsed_entries {
        let keys_str = entry.keys.join(", ");
        let order_val = entry.order.unwrap_or(100);
        let pos_str = match entry.position.unwrap_or(1) {
            0 => "before (0)",
            _ => "after (1)",
        };
        let enabled_val = entry.enabled.unwrap_or(true);
        // Replace raw newlines in table to prevent breaking formatting
        let content_clean = entry.content.replace('\n', "<br>");

        md_content.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            keys_str, order_val, pos_str, enabled_val, content_clean
        ));
    }

    fs::write(md_path, md_content).map_err(|e| format!("Failed to write MD file: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const SAMPLE_WB: &str = r#"{
        "entries": {
            "0": {
                "keys": ["天剑阁"],
                "content": "天剑阁是江湖第一大派",
                "order": 10,
                "position": 0,
                "enabled": true
            }
        }
    }"#;

    #[test]
    fn test_cf5_extra_dir_scanned() {
        // CF-5: characters/{id}/world/extra/*.json 应被转换为 .md
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let extra = root
            .join("characters")
            .join("alice")
            .join("world")
            .join("extra");
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("custom_wb.json"), SAMPLE_WB).unwrap();

        auto_convert_legacy_files(root).unwrap();

        let md_path = extra.join("custom_wb.md");
        assert!(md_path.exists(), "world/extra/ JSON 应被转为 MD");
        let md = fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("天剑阁"));
    }

    #[test]
    fn test_cf5_legacy_worldbooks_dir_still_scanned() {
        // CF-5: 旧 worldbooks/ 目录扫描仍生效（向后兼容）
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let wb = root.join("characters").join("bob").join("worldbooks");
        fs::create_dir_all(&wb).unwrap();
        fs::write(wb.join("legacy.json"), SAMPLE_WB).unwrap();

        auto_convert_legacy_files(root).unwrap();

        assert!(wb.join("legacy.md").exists());
    }

    #[test]
    fn test_cf5_does_not_overwrite_existing_md() {
        // 已存在 .md 时跳过转换
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let extra = root
            .join("characters")
            .join("carol")
            .join("world")
            .join("extra");
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("x.json"), SAMPLE_WB).unwrap();
        fs::write(extra.join("x.md"), "USER_EDITED").unwrap();

        auto_convert_legacy_files(root).unwrap();

        let md = fs::read_to_string(extra.join("x.md")).unwrap();
        assert_eq!(md, "USER_EDITED", "已存在的 MD 不应被覆盖");
    }

    #[test]
    fn test_cf5_main_lorebook_not_touched() {
        // CF-5: world/lorebook.json (主 lorebook) 不在 extra/，不应被转 MD
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let world = root.join("characters").join("dave").join("world");
        fs::create_dir_all(&world).unwrap();
        fs::write(world.join("lorebook.json"), SAMPLE_WB).unwrap();

        auto_convert_legacy_files(root).unwrap();

        // 主 lorebook 仍保持 JSON，无对应 MD
        assert!(world.join("lorebook.json").exists());
        assert!(
            !world.join("lorebook.md").exists(),
            "主 lorebook 不该被转 MD"
        );
    }
}
