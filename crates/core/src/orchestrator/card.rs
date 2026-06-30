use serde::{Deserialize, Serialize};

/// Tavern V2 规范的预设 Prompts。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavernPrompt {
    pub identifier: String,
    pub name: String,
    /// SillyTavern 规范：缺少 `enabled` 字段时视为 `true`（默认启用）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// marker 类型 prompt 可能不含 role 字段。
    #[serde(default)]
    pub role: String,
    pub content: Option<String>,
    pub system_prompt: Option<bool>,
}

fn default_true() -> bool {
    true
}

/// Tavern V2 规范的预设配置包。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavernPreset {
    pub prompts: Option<Vec<TavernPrompt>>,
    /// SillyTavern 预设级 temperature，作为 API 层默认值（可被 request body 覆盖）。
    pub temperature: Option<f32>,
    /// SillyTavern 预设级 max_tokens，字段名 `openai_max_tokens` 和 `max_tokens` 均接受。
    #[serde(alias = "openai_max_tokens")]
    pub max_tokens: Option<u32>,
    /// SillyTavern 预设级 model，作为 API 层默认值（可被 request body 覆盖）。
    #[serde(alias = "openai_model")]
    pub model: Option<String>,
}

/// Tavern V2 规范的角色卡内层数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterData {
    pub name: Option<String>,
    pub description: Option<String>,
    pub personality: Option<String>,
    pub scenario: Option<String>,
    pub first_mes: Option<String>,
    pub mes_template: Option<String>,
    pub system_prompt: Option<String>,
    /// 示例对话（SillyTavern mes_example 字段）。
    pub mes_example: Option<String>,
    /// ���个开场语（SillyTavern alternate_greetings）。
    #[serde(default)]
    pub alternate_greetings: Vec<String>,
    /// 角色卡内嵌世界书（SillyTavern character_book）。
    /// 保留为原始 JSON Value 以兼容不同 SillyTavern 版��的 entries 结构。
    pub character_book: Option<serde_json::Value>,
}

/// Tavern V2 规范的角色卡外层包装。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavernCardV2 {
    pub spec: Option<String>,
    pub spec_version: Option<String>,
    pub data: CharacterData,
}

/// 老 v1 平铺字段名（TavernAI / 早期 ST）→ v2 schema 字段名。
const V1_LEGACY_MAP: &[(&str, &str)] = &[
    ("char_name", "name"),
    ("char_persona", "personality"),
    ("char_greeting", "first_mes"),
    ("world_scenario", "scenario"),
    ("example_dialogue", "mes_example"),
];

/// v1 卡可能直接放在顶层的字段（v1.5 起多用 v2 风格名）。
const V1_FLAT_FIELDS: &[&str] = &[
    "name",
    "description",
    "personality",
    "scenario",
    "first_mes",
    "mes_example",
    "creator_notes",
    "system_prompt",
    "post_history_instructions",
    "alternate_greetings",
    "tags",
    "creator",
    "character_version",
    "extensions",
    "character_book",
];

/// 把 v1 平铺角色卡归一化为 v2 schema（`spec` + `data:{...}`）。
///
/// 下游 [`TavernCardV2`] 反序列化要求 `data` 嵌套对象；v1 卡字段平铺在顶层，
/// 不归一化则 `first_mes` / `character_book` 等全部丢失。已是 v2/v3
/// （有 `spec` 且 `data` 为对象）的卡原样返回。解析失败也原样返回，
/// 把错误留给下游统一处理。
///
/// 字段映射依据公开的 TavernAI / SillyTavern v1 卡规范。
pub fn normalize_v1_to_v2(json: &str) -> String {
    use serde_json::{Map, Value};

    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return json.to_string();
    };
    let Some(obj) = root.as_object() else {
        return json.to_string();
    };

    // 已是 v2/v3 且 data 为对象 → 无需归一化。
    let spec = obj.get("spec").and_then(Value::as_str).unwrap_or("");
    let data_is_object = obj.get("data").is_some_and(Value::is_object);
    if matches!(spec, "chara_card_v2" | "chara_card_v3") && data_is_object {
        return json.to_string();
    }

    let mut data = Map::new();
    // 1. v1.5 风格的平铺字段直接搬（v2 风格名优先）。
    for field in V1_FLAT_FIELDS {
        if let Some(v) = obj.get(*field) {
            data.entry(field.to_string()).or_insert_with(|| v.clone());
        }
    }
    // 2. 老字段名 → 新字段名；仅当源卡未直接提供 v2 风格名时才用别名。
    for (old, new) in V1_LEGACY_MAP {
        if let Some(v) = obj.get(*old) {
            data.entry(new.to_string()).or_insert_with(|| v.clone());
        }
    }
    // 3. 混合形态：原卡已有部分 data 块，合并（不覆盖已抬升的字段）。
    if let Some(existing) = obj.get("data").and_then(Value::as_object) {
        for (k, v) in existing {
            data.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    let normalized = serde_json::json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": Value::Object(data),
        "_normalized_from_v1": true,
    });
    // 序列化失败的概率为 0（输入已是合法 Value），兜底回退原文。
    serde_json::to_string(&normalized).unwrap_or_else(|_| json.to_string())
}

/// JSON 顶层形状判定结果，用于 import 边界校验，防 card/preset 互相误导入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonShape {
    /// 角色卡：有 `spec: chara_card_v2/v3` + `data{}`，或 v1 平铺（有 `name`/`first_mes`/`char_name` 等）。
    Card,
    /// SillyTavern 预设：有 `prompts[]` + `prompt_order`，或模型参数集，且无角色卡特征字段。
    Preset,
    /// 两类特征都不足，无法判定。
    Unknown,
}

/// 探测 JSON 顶层属于角色卡还是 SillyTavern 预设。
///
/// 机械判定，不读语义：
/// - 预设特征：顶层 `prompts` 数组 + (`prompt_order` 或 模型参数 temperature/top_p/openai_model)。
/// - 角色卡特征：`spec=chara_card_v2/v3` + `data` 对象，或 v1 平铺 `name`+`first_mes`（或 legacy `char_name`）。
/// - 角色卡判定优先于预设：避免内嵌 prompts 的卡被误判（卡的 prompts 在 `data` 内，不在顶层）。
pub fn detect_json_shape(json: &str) -> JsonShape {
    use serde_json::Value;
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(json) else {
        return JsonShape::Unknown;
    };

    // 角色卡特征（优先）。
    let spec = obj.get("spec").and_then(Value::as_str).unwrap_or("");
    let is_v2v3_card = matches!(spec, "chara_card_v2" | "chara_card_v3")
        && obj.get("data").is_some_and(Value::is_object);
    let is_v1_card = obj.contains_key("char_name")
        || (obj.contains_key("first_mes")
            && (obj.contains_key("name") || obj.contains_key("personality")));
    if is_v2v3_card || is_v1_card {
        return JsonShape::Card;
    }

    // 预设特征：顶层 prompts 数组 + 预设侧佐证字段。
    let has_prompts = obj.get("prompts").is_some_and(Value::is_array);
    let has_preset_markers = obj.contains_key("prompt_order")
        || obj.contains_key("temperature")
        || obj.contains_key("top_p")
        || obj.contains_key("openai_model");
    if has_prompts && has_preset_markers {
        return JsonShape::Preset;
    }

    JsonShape::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> TavernCardV2 {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn test_v1_legacy_names_mapped() {
        let v1 = r#"{"char_name":"Bob","char_persona":"勇敢","char_greeting":"你好","world_scenario":"酒馆"}"#;
        let out = normalize_v1_to_v2(v1);
        let card = parse(&out);
        assert_eq!(card.data.name.as_deref(), Some("Bob"));
        assert_eq!(card.data.personality.as_deref(), Some("勇敢"));
        assert_eq!(card.data.first_mes.as_deref(), Some("你好"));
        assert_eq!(card.data.scenario.as_deref(), Some("酒馆"));
    }

    #[test]
    fn test_v1_flat_v2style_names_lifted() {
        let v1 = r#"{"name":"Ann","first_mes":"hi","alternate_greetings":["a","b"]}"#;
        let out = normalize_v1_to_v2(v1);
        let card = parse(&out);
        assert_eq!(card.data.name.as_deref(), Some("Ann"));
        assert_eq!(card.data.first_mes.as_deref(), Some("hi"));
        assert_eq!(card.data.alternate_greetings, vec!["a", "b"]);
    }

    #[test]
    fn test_v2_card_unchanged() {
        let v2 = r#"{"spec":"chara_card_v2","data":{"name":"X"}}"#;
        let out = normalize_v1_to_v2(v2);
        assert!(!out.contains("_normalized_from_v1"));
        assert_eq!(parse(&out).data.name.as_deref(), Some("X"));
    }

    #[test]
    fn test_v3_card_unchanged() {
        let v3 = r#"{"spec":"chara_card_v3","data":{"name":"Y"}}"#;
        let out = normalize_v1_to_v2(v3);
        assert!(!out.contains("_normalized_from_v1"));
        assert_eq!(parse(&out).data.name.as_deref(), Some("Y"));
    }

    #[test]
    fn test_invalid_json_returned_asis() {
        let bad = "not json {{";
        assert_eq!(normalize_v1_to_v2(bad), bad);
    }

    #[test]
    fn test_legacy_name_not_overriding_v2style() {
        // 同时存在 char_name(legacy) 和 name(v2)：name 优先，不被 legacy 覆盖。
        let v1 = r#"{"char_name":"legacy","name":"modern"}"#;
        let card = parse(&normalize_v1_to_v2(v1));
        assert_eq!(card.data.name.as_deref(), Some("modern"));
    }

    // ── detect_json_shape ────────────────────────────────────────────────

    #[test]
    fn test_shape_preset_prompts_plus_params() {
        // 真实误识别场景：LENI 预设 = 顶层 prompts[] + temperature 等模型参数。
        let preset = r#"{"prompts":[{"identifier":"main","name":"Main"}],"temperature":1.19,"prompt_order":[]}"#;
        assert_eq!(detect_json_shape(preset), JsonShape::Preset);
    }

    #[test]
    fn test_shape_v2_card() {
        let card = r#"{"spec":"chara_card_v2","data":{"name":"X","first_mes":"hi"}}"#;
        assert_eq!(detect_json_shape(card), JsonShape::Card);
    }

    #[test]
    fn test_shape_v3_card() {
        let card = r#"{"spec":"chara_card_v3","data":{"name":"Y"}}"#;
        assert_eq!(detect_json_shape(card), JsonShape::Card);
    }

    #[test]
    fn test_shape_v1_flat_card() {
        let card = r#"{"name":"Bob","first_mes":"你好","personality":"勇敢"}"#;
        assert_eq!(detect_json_shape(card), JsonShape::Card);
    }

    #[test]
    fn test_shape_v1_legacy_card() {
        let card = r#"{"char_name":"Bob","char_greeting":"hi"}"#;
        assert_eq!(detect_json_shape(card), JsonShape::Card);
    }

    #[test]
    fn test_shape_card_wins_over_embedded_prompts() {
        // 卡内可能含 prompts，但在 data 内、非顶层；顶层有 spec+data → 判 Card。
        let card = r#"{"spec":"chara_card_v2","data":{"name":"Z"},"prompts":[{"identifier":"x","name":"y"}],"temperature":1.0}"#;
        assert_eq!(detect_json_shape(card), JsonShape::Card);
    }

    #[test]
    fn test_shape_unknown_when_ambiguous() {
        // 既无卡特征也无预设佐证字段。
        assert_eq!(detect_json_shape(r#"{"foo":"bar"}"#), JsonShape::Unknown);
        // prompts[] 但无任何预设佐证字段 → 不强判 Preset。
        assert_eq!(detect_json_shape(r#"{"prompts":[]}"#), JsonShape::Unknown);
        // 非法 JSON。
        assert_eq!(detect_json_shape("not json"), JsonShape::Unknown);
    }
}
