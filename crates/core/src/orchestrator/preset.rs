use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

use super::card::TavernPreset;

static SETVAR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\{setvar::([^:]+)::([^}]*)\}\}").expect("SETVAR_RE"));
static GETVAR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\{getvar::([^}]+)\}\}").expect("GETVAR_RE"));

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
