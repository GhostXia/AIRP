pub mod card;
pub mod gating;
pub mod lorebook;
pub mod preset;
pub mod trace;
pub mod volume_inject;
pub mod worldbook_normalizer;

// Re-exports so callers keep `crate::orchestrator::Foo` paths unchanged.
pub use card::{CharacterData, TavernCardV2, TavernPreset, TavernPrompt};
pub use lorebook::{merge_lorebooks, Lorebook, LorebookEntry};
pub use volume_inject::{inject_current_context, inject_volume_context};
pub use worldbook_normalizer::{normalize_worldbook, WorldbookImportReport};

use crate::error::AirpError;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

pub struct Orchestrator {
    pub card: Option<CharacterData>,
    pub lorebook: Option<Lorebook>,
}

impl Orchestrator {
    pub fn new(card_json: Option<&str>, lorebook_json: Option<&str>) -> Result<Self, AirpError> {
        let card = if let Some(json) = card_json {
            let parsed = if let Ok(v2) = serde_json::from_str::<TavernCardV2>(json) {
                v2.data
            } else if let Ok(data) = serde_json::from_str::<CharacterData>(json) {
                data
            } else {
                return Err(AirpError::Orchestrator(
                    "无法解析角色卡 JSON，格式不符合 Tavern V2 或内层结构".to_string(),
                ));
            };
            Some(parsed)
        } else {
            None
        };

        let lorebook = if let Some(json) = lorebook_json {
            let lb = serde_json::from_str::<Lorebook>(json)
                .map_err(|e| AirpError::Orchestrator(format!("解析世界书 JSON 失败: {}", e)))?;
            Some(lb)
        } else {
            None
        };

        Ok(Self { card, lorebook })
    }

    // ── Lorebook ──────────────────────────────────────────────────────────────

    pub fn trigger_lorebook(&self, text: &str) -> String {
        self.lorebook
            .as_ref()
            .map(|lb| lb.trigger(text))
            .unwrap_or_default()
    }

    // ── Gating (static dispatch to gating module) ─────────────────────────────

    pub fn advance_timeline_and_checkpoint(data_root: &Path, character_id: &str) {
        gating::advance_timeline_and_checkpoint(data_root, character_id);
    }

    pub fn get_current_checkpoint(data_root: &Path, character_id: &str) -> String {
        gating::get_current_checkpoint(data_root, character_id)
    }

    pub fn load_filtered_known(data_root: &Path, character_id: &str, current_cp: &str) -> String {
        gating::load_filtered_known(data_root, character_id, current_cp)
    }

    // ── Preset (static dispatch to preset module) ──────────────────────────────

    pub fn assemble_preset_prompts(
        preset_json: &str,
        enabled_override_ids: Option<&Vec<String>>,
        char_name: &str,
        user_name: &str,
        last_message: &str,
    ) -> String {
        preset::assemble_preset_prompts(
            preset_json,
            enabled_override_ids,
            char_name,
            user_name,
            last_message,
        )
    }

    pub fn render_macros(
        text: &str,
        char_name: &str,
        user_name: &str,
        last_message: &str,
    ) -> String {
        preset::render_macros(text, char_name, user_name, last_message)
    }

    // ── System Prompt builders ─────────────────────────────────────────────────

    /// 装配基础 System Prompt（无预设）。
    pub fn build_system_prompt(
        &self,
        user_name: &str,
        variables: &HashMap<String, String>,
        triggered_lore: &str,
    ) -> String {
        let mut prompt = String::new();

        let fields = self.extract_card_fields();

        if let Some(ov) = fields.system_override {
            prompt.push_str(ov);
            prompt.push('\n');
        } else {
            prompt.push_str(&format!(
                "You are going to roleplay as {}. Always stay in character and act naturally.\n",
                fields.char_name
            ));
        }

        if !fields.personality.is_empty() {
            prompt.push_str(&format!(
                "[{}'s Personality]:\n{}\n\n",
                fields.char_name, fields.personality
            ));
        }
        if !fields.description.is_empty() {
            prompt.push_str(&format!(
                "[{}'s Appearance & Description]:\n{}\n\n",
                fields.char_name, fields.description
            ));
        }
        if !fields.scenario.is_empty() {
            prompt.push_str(&format!("[Scenario]:\n{}\n\n", fields.scenario));
        }
        if !triggered_lore.is_empty() {
            prompt.push_str(triggered_lore);
            prompt.push('\n');
        }

        let mut final_vars = variables.clone();
        final_vars.insert("char".to_string(), fields.char_name.into_owned());
        final_vars.insert("user".to_string(), user_name.to_string());
        for (key, val) in final_vars {
            prompt = prompt.replace(&format!("{{{{{}}}}}", key), &val);
        }

        prompt
    }

    /// 装配带有预设拼接的 System Prompt。
    #[allow(clippy::too_many_arguments)]
    pub fn build_system_prompt_with_preset(
        &self,
        data_root: &Path,
        character_id: Option<&str>,
        user_name: &str,
        variables: &HashMap<String, String>,
        triggered_lore: &str,
        preset_json: Option<&str>,
        enabled_override_ids: Option<&Vec<String>>,
        last_message: &str,
    ) -> String {
        let mut prompt = String::new();

        let fields = self.extract_card_fields();

        if let Some(ov) = fields.system_override {
            prompt.push_str(ov);
            prompt.push('\n');
        } else {
            prompt.push_str(&format!(
                "You are going to roleplay as {}. Always stay in character and act naturally.\n",
                fields.char_name
            ));
        }

        // CP-gated known.md
        if let Some(char_id) = character_id {
            let current_cp = gating::get_current_checkpoint(data_root, char_id);
            let filtered_known = gating::load_filtered_known(data_root, char_id, &current_cp);
            if !filtered_known.is_empty() {
                prompt.push_str(&format!(
                    "\n[{}'s Known Information & Clues (Current CP: {})]:\n{}\n\n",
                    fields.char_name, current_cp, filtered_known
                ));
            }
        }

        if !fields.personality.is_empty() {
            prompt.push_str(&format!(
                "[{}'s Personality]:\n{}\n\n",
                fields.char_name, fields.personality
            ));
        }
        if !fields.description.is_empty() {
            prompt.push_str(&format!(
                "[{}'s Appearance & Description]:\n{}\n\n",
                fields.char_name, fields.description
            ));
        }
        if !fields.scenario.is_empty() {
            prompt.push_str(&format!("[Scenario]:\n{}\n\n", fields.scenario));
        }
        if !triggered_lore.is_empty() {
            prompt.push_str(triggered_lore);
            prompt.push('\n');
        }

        // M_LS LS-4: inject live state so LLM sees current values and knows to update them
        if let Some(char_id) = character_id {
            inject_live_state(data_root, char_id, &mut prompt);
        }

        // Preset prompts
        if let Some(json) = preset_json {
            let pp = preset::assemble_preset_prompts(
                json,
                enabled_override_ids,
                &fields.char_name,
                user_name,
                last_message,
            );
            if !pp.is_empty() {
                prompt.push_str(&pp);
                prompt.push('\n');
            }
        }

        // Macro substitution
        let mut final_vars = variables.clone();
        final_vars.insert("char".to_string(), fields.char_name.into_owned());
        final_vars.insert("user".to_string(), user_name.to_string());
        for (key, val) in final_vars {
            prompt = prompt.replace(&format!("{{{{{}}}}}", key), &val);
        }

        prompt
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    pub(crate) fn extract_card_fields_pub(&self) -> CardFields<'_> {
        self.extract_card_fields()
    }

    fn extract_card_fields(&self) -> CardFields<'_> {
        if let Some(ref data) = self.card {
            CardFields {
                char_name: data
                    .name
                    .as_deref()
                    .map(Cow::Borrowed)
                    .unwrap_or(Cow::Borrowed("AI")),
                description: data.description.as_deref().unwrap_or(""),
                personality: data.personality.as_deref().unwrap_or(""),
                scenario: data.scenario.as_deref().unwrap_or(""),
                system_override: data.system_prompt.as_deref(),
            }
        } else {
            CardFields {
                char_name: Cow::Borrowed("AI"),
                description: "",
                personality: "",
                scenario: "",
                system_override: None,
            }
        }
    }
}

/// M_LS LS-4/8: Read `state/live.json` (and optionally `state/schema.json`) and append
/// a `[Current State]` block to `prompt`. When schema is present, renders labeled rows
/// alongside raw JSON and lists expected keys in the `<state>` instruction.
/// No-op if live.json doesn't exist or can't be parsed.
fn inject_live_state(data_root: &Path, character_id: &str, prompt: &mut String) {
    let state_dir = crate::data_dir::char_state_dir(data_root, character_id);
    let Ok(json) = std::fs::read_to_string(state_dir.join("live.json")) else {
        return;
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&json) else {
        return;
    };

    // Try to load schema for richer rendering (LS-7/8 enhancement).
    let schema_fields: Option<Vec<serde_json::Value>> =
        std::fs::read_to_string(state_dir.join("schema.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["fields"].as_array().cloned());

    prompt.push_str("[Current State]:\n");

    if let Some(fields) = &schema_fields {
        // Labeled rendering: "- 生命值 (hp): 80/100"
        for field in fields {
            let key = field["key"].as_str().unwrap_or("");
            let label = field["label"].as_str().unwrap_or(key);
            if let Some(val) = state.get(key) {
                // If there's a _max companion in state or schema, append it
                let max_val = state
                    .get(format!("{}_max", key).as_str())
                    .or_else(|| field.get("max"))
                    .filter(|v| v.is_number());
                if let Some(mv) = max_val {
                    prompt.push_str(&format!("- {} ({}): {}/{}\n", label, key, val, mv));
                } else {
                    prompt.push_str(&format!("- {} ({}): {}\n", label, key, val));
                }
            }
        }
        // Also show compact raw JSON for precise key reference
        if let Ok(compact) = serde_json::to_string(&state) {
            prompt.push_str(&format!("(raw: {})\n", compact));
        }
        // Build expected keys list for the instruction
        let keys: Vec<&str> = fields.iter().filter_map(|f| f["key"].as_str()).collect();
        prompt.push_str(&format!(
            "When any state value changes, output the complete updated state at the very end as: \
             <state>{{...}}</state> (keys: {})\n\n",
            keys.join(", ")
        ));
    } else {
        // No schema: fall back to pretty JSON block
        if let Ok(pretty) = serde_json::to_string_pretty(&state) {
            prompt.push_str(&format!("```json\n{}\n```\n", pretty));
        }
        prompt.push_str(
            "When any state value changes during your response, output the complete updated state \
             at the very end of your response as: <state>{...}</state>\n\n",
        );
    }
}

/// MS-4: Build a multi-character system prompt from a scene config + loaded card JSON strings.
///
/// Format:
/// ```text
/// [场景设定]
/// {description}
///
/// [在场角色]
/// ## {name}（主视角）
/// {personality, description, scenario, system_prompt}
///
/// ## {npc_name}（NPC）
/// {description}
///
/// [世界书信息]
/// {triggered lore}
///
/// [格式规则]
/// {format_hint}
/// 用户扮演 {user_name}，AI 不代写用户台词。
/// ```
pub fn build_multi_char_system_prompt(
    scene: &crate::scene::SceneConfig,
    cards: &[(&str, Option<&str>)], // (character_id, card_json_or_none)
    triggered_lore: &str,
    user_name: &str,
) -> String {
    let mut prompt = String::new();

    if !scene.description.is_empty() {
        prompt.push_str("[场景设定]\n");
        prompt.push_str(&scene.description);
        prompt.push_str("\n\n");
    }

    prompt.push_str("[在场角色]\n");

    for entry in &scene.characters {
        let card_json = cards
            .iter()
            .find(|(id, _)| *id == entry.character_id.as_str())
            .and_then(|(_, j)| *j);

        let role_label = if entry.role == crate::scene::CharacterRole::Primary {
            "（主视角）"
        } else {
            "（NPC）"
        };

        let char_name = card_json
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .and_then(|v| {
                v["data"]["name"]
                    .as_str()
                    .or_else(|| v["name"].as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| entry.character_id.clone());

        prompt.push_str(&format!("## {}{}\n", char_name, role_label));

        if !entry.intro.is_empty() {
            prompt.push_str(&entry.intro);
            prompt.push('\n');
        }

        if let Some(json) = card_json {
            // Inline-load card fields for the prompt
            let orch = Orchestrator::new(Some(json), None);
            if let Ok(o) = orch {
                let fields = o.extract_card_fields_pub();
                if !fields.personality.is_empty() {
                    prompt.push_str(&format!("[性格]: {}\n", fields.personality));
                }
                if !fields.description.is_empty() {
                    prompt.push_str(&format!("[描述]: {}\n", fields.description));
                }
                if !fields.scenario.is_empty() {
                    prompt.push_str(&format!("[场景设定]: {}\n", fields.scenario));
                }
            }
        }
        prompt.push('\n');
    }

    if !triggered_lore.is_empty() {
        prompt.push_str("[世界书信息]\n");
        prompt.push_str(triggered_lore);
        prompt.push_str("\n\n");
    }

    if !scene.format_hint.is_empty() {
        prompt.push_str("[格式规则]\n");
        prompt.push_str(&scene.format_hint);
        prompt.push('\n');
    }
    prompt.push_str(&format!("用户扮演 {}，AI 不代写用户台词。\n", user_name));

    prompt
}

/// M_LS LS-9: test-only shim so cross-module tests can call the private `inject_live_state`.
#[cfg(test)]
pub fn inject_live_state_for_test(data_root: &Path, character_id: &str, prompt: &mut String) {
    inject_live_state(data_root, character_id, prompt);
}

/// M0 F-40 / 6.0g + 6.0k：角色卡字段提取结果。
/// 字段全部借用自 `self.card`，避免每次 build_system_prompt 都 clone 5 个 String。
/// `char_name` 用 `Cow` 兼容 card 缺失时的常量 `"AI"` 兜底。
#[derive(Debug, Clone)]
pub(crate) struct CardFields<'a> {
    pub(crate) char_name: Cow<'a, str>,
    pub(crate) description: &'a str,
    pub(crate) personality: &'a str,
    pub(crate) scenario: &'a str,
    pub(crate) system_override: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_build() {
        let card_json = r#"{
            "spec": "chara_card_v2",
            "data": {
                "name": "艾米丽",
                "personality": "温柔, 善良",
                "description": "一头金发，手持{{weapon}}",
                "scenario": "酒馆"
            }
        }"#;

        let lorebook_json = r#"{
            "entries": [
                {
                    "keys": ["长剑", "精钢长剑"],
                    "content": "精钢长剑是精炼钢制成的武器，极其锋利。",
                    "enabled": true,
                    "priority": 100
                }
            ]
        }"#;

        let orchestrator = Orchestrator::new(Some(card_json), Some(lorebook_json)).unwrap();

        let triggered = orchestrator.trigger_lorebook("我亮出了一柄精钢长剑。");
        assert!(triggered.contains("精钢长剑是精炼钢制成的武器"));

        let mut vars = HashMap::new();
        vars.insert("weapon".to_string(), "精钢长剑".to_string());

        let system_prompt = orchestrator.build_system_prompt("小明", &vars, &triggered);

        assert!(system_prompt.contains("You are going to roleplay as 艾米丽"));
        assert!(system_prompt.contains("一头金发，手持精钢长剑"));
        assert!(system_prompt.contains("精钢长剑是精炼钢制成的武器"));
    }

    // M_LS LS-4: inject_live_state tests

    #[test]
    fn test_ls4_inject_live_state_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut prompt = String::from("Base.");
        super::inject_live_state(tmp.path(), "hero", &mut prompt);
        assert_eq!(prompt, "Base.", "no live.json → prompt unchanged");
    }

    #[test]
    fn test_ls4_inject_live_state_injects_block() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = crate::data_dir::char_state_dir(tmp.path(), "hero");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("live.json"), r#"{"hp":80,"mp":20}"#).unwrap();

        let mut prompt = String::from("Base.");
        super::inject_live_state(tmp.path(), "hero", &mut prompt);
        assert!(
            prompt.contains("[Current State]"),
            "should inject [Current State] header"
        );
        assert!(prompt.contains("\"hp\": 80"), "should include state values");
        assert!(
            prompt.contains("<state>"),
            "should include state tag instruction"
        );
    }

    // MS-9 tests for build_multi_char_system_prompt

    #[test]
    fn test_ms9_multi_char_prompt_contains_all_character_sections() {
        use crate::scene::{CharacterEntry, CharacterRole, LorebookMerge, SceneConfig};
        use crate::types::SceneId;

        let scene = SceneConfig {
            scene_id: SceneId::new("tavern").unwrap(),
            description: "茶馆初春".to_string(),
            characters: vec![
                CharacterEntry {
                    character_id: "alice".to_string(),
                    role: CharacterRole::Primary,
                    intro: "剑客".to_string(),
                },
                CharacterEntry {
                    character_id: "bob".to_string(),
                    role: CharacterRole::Npc,
                    intro: "掌柜".to_string(),
                },
            ],
            narrator_style: "third_person_limited".to_string(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: "角色名：台词".to_string(),
        };

        let alice_card = r#"{"spec":"chara_card_v2","data":{"name":"爱丽丝","personality":"勇敢","description":"女剑客"}}"#;
        let bob_card = r#"{"spec":"chara_card_v2","data":{"name":"鲍勃","personality":"谨慎","description":"茶馆掌柜"}}"#;

        let cards = [("alice", Some(alice_card)), ("bob", Some(bob_card))];
        let prompt = super::build_multi_char_system_prompt(&scene, &cards, "", "user");

        assert!(prompt.contains("[场景设定]"), "should have scene section");
        assert!(prompt.contains("茶馆初春"), "should have scene description");
        assert!(
            prompt.contains("[在场角色]"),
            "should have characters section"
        );
        assert!(
            prompt.contains("爱丽丝"),
            "should have alice's name from card"
        );
        assert!(prompt.contains("（主视角）"), "should mark primary role");
        assert!(prompt.contains("鲍勃"), "should have bob's name from card");
        assert!(prompt.contains("（NPC）"), "should mark npc role");
        assert!(
            prompt.contains("[格式规则]"),
            "should have format hint section"
        );
        assert!(prompt.contains("user"), "should mention user name");
    }

    #[test]
    fn test_ms9_multi_char_prompt_no_cards_uses_ids() {
        use crate::scene::{CharacterEntry, CharacterRole, LorebookMerge, SceneConfig};
        use crate::types::SceneId;

        let scene = SceneConfig {
            scene_id: SceneId::new("void").unwrap(),
            description: String::new(),
            characters: vec![CharacterEntry {
                character_id: "char1".to_string(),
                role: CharacterRole::Npc,
                intro: String::new(),
            }],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };

        let prompt = super::build_multi_char_system_prompt(&scene, &[], "", "User");
        assert!(
            prompt.contains("char1"),
            "should fall back to character_id when no card"
        );
    }

    #[test]
    fn test_ms9_multi_char_prompt_includes_triggered_lore() {
        use crate::scene::{CharacterEntry, CharacterRole, LorebookMerge, SceneConfig};
        use crate::types::SceneId;

        let scene = SceneConfig {
            scene_id: SceneId::new("test").unwrap(),
            description: String::new(),
            characters: vec![CharacterEntry {
                character_id: "x".to_string(),
                role: CharacterRole::Primary,
                intro: String::new(),
            }],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };

        let lore = "这里是传说中的禁地";
        let prompt = super::build_multi_char_system_prompt(&scene, &[], lore, "user");
        assert!(prompt.contains("[世界书信息]"), "should have lore section");
        assert!(prompt.contains("禁地"), "should include lore content");
    }

    #[test]
    fn test_ls8_inject_live_state_uses_schema_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = crate::data_dir::char_state_dir(tmp.path(), "hero");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(
            state_dir.join("live.json"),
            r#"{"hp":75,"hp_max":100,"location":"tavern"}"#,
        )
        .unwrap();
        let schema = serde_json::json!({
            "fields": [
                {"key": "hp", "type": "number", "min": 0, "max": 100, "label": "生命值"},
                {"key": "location", "type": "string", "label": "当前位置"}
            ]
        });
        std::fs::write(
            state_dir.join("schema.json"),
            serde_json::to_string(&schema).unwrap(),
        )
        .unwrap();

        let mut prompt = String::from("Base.");
        super::inject_live_state(tmp.path(), "hero", &mut prompt);
        assert!(
            prompt.contains("[Current State]"),
            "should inject [Current State] header"
        );
        assert!(prompt.contains("生命值 (hp)"), "should use schema label");
        assert!(
            prompt.contains("当前位置 (location)"),
            "should use schema label for string field"
        );
        assert!(prompt.contains("75/100"), "should show hp/max");
        assert!(
            prompt.contains("keys: hp, location"),
            "should list expected keys in instruction"
        );
        assert!(
            prompt.contains("<state>"),
            "should include state tag instruction"
        );
    }

    #[test]
    fn test_ls4_inject_live_state_invalid_json_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = crate::data_dir::char_state_dir(tmp.path(), "hero");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("live.json"), b"not json").unwrap();

        let mut prompt = String::from("Base.");
        super::inject_live_state(tmp.path(), "hero", &mut prompt);
        assert_eq!(prompt, "Base.", "invalid JSON → prompt unchanged");
    }
}
