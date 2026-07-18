//! Prompt assembly trace data model (#115 P1).
//!
//! The trace is populated explicitly by prompt assembly call sites so segment order and
//! provenance can match the provider payload. This module intentionally does not infer
//! provenance from marker text in a completed prompt.

use serde::Serialize;

/// #114 effective config summary：Persona 激活来源。
///
/// 反映 `resolve_request_persona` 实际命中 precedence。序列化为 snake_case 字符串，
/// 旧客户端若不识别该字段可安全忽略（`EffectiveIds.persona_activation_source` 是 `Option<String>`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaActivationSource {
    /// 请求体显式指定 `persona_id`。
    Explicit,
    /// 命中 session 维度 persona 绑定（`find_for_character` 精确匹配 session）。
    SessionBinding,
    /// 命中 character 维度 generic persona 绑定（`find_for_character` 匹配该角色 generic 绑定）。
    CharacterBinding,
    /// 未命中显式/绑定，回退到 default persona。
    Default,
    /// 无 `user_id`（单用户向后兼容路径），persona 未参与本轮装配。
    Absent,
}

impl PersonaActivationSource {
    /// 稳定字符串标签，用于填充 `EffectiveIds.persona_activation_source`。
    pub fn as_str(self) -> &'static str {
        match self {
            PersonaActivationSource::Explicit => "explicit",
            PersonaActivationSource::SessionBinding => "session_binding",
            PersonaActivationSource::CharacterBinding => "character_binding",
            PersonaActivationSource::Default => "default",
            PersonaActivationSource::Absent => "absent",
        }
    }
}

/// #114 effective config summary：本轮生效参数的来源标签。
///
/// 由 `resolve_param_sources` 在 `build_prompt_trace` 调用前一次性算好，
/// 让 trace 能回答 "model 来自请求体 / preset / snapshot" 这类问题。
/// 来源是 `&'static str` 而非 enum，便于将来增加新来源值时不破坏序列化兼容。
#[derive(Debug, Clone, Default)]
pub struct ParamSources {
    /// `provider` 来自 `request` 还是 `snapshot`。
    pub provider_source: Option<&'static str>,
    /// `model` 来自 `request` / `preset` / `snapshot`。
    pub model_source: Option<&'static str>,
    /// 本轮生效 temperature（如有）。
    pub temperature: Option<f32>,
    /// `temperature` 来自 `request` / `preset`；None 表示两者均未指定。
    pub temperature_source: Option<&'static str>,
    /// 本轮生效 max_tokens（如有）。
    pub max_tokens: Option<u32>,
    /// `max_tokens` 来自 `request` / `preset`；None 表示两者均未指定。
    pub max_tokens_source: Option<&'static str>,
}

/// Metadata describing one prepared prompt and its ordered source segments.
#[derive(Debug, Clone, Serialize)]
pub struct PromptAssemblyTrace {
    pub segments: Vec<PromptSegment>,
    pub diagnostics: Vec<PromptDiagnostic>,
    pub total_chars: usize,
    pub total_estimated_tokens: usize,
    pub effective: EffectiveIds,
}

/// One source segment in provider-payload order.
#[derive(Debug, Clone, Serialize)]
pub struct PromptSegment {
    /// Domain source such as card, persona, preset, lorebook, state, memory, history, or user.
    pub source_kind: String,
    pub source_id: Option<String>,
    pub item_id: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    /// Byte offset in the final prompt or message payload.
    pub position: usize,
    /// Why this segment was enabled and included.
    pub enabled_reason: Option<String>,
    pub chars: usize,
    pub estimated_tokens: usize,
    pub truncated: bool,
    pub stable_or_volatile: Stability,
}

/// Whether a segment is stable across turns or volatile session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stability {
    Stable,
    Volatile,
}

/// Effective configuration and revision snapshot used for assembly.
#[derive(Debug, Clone, Serialize, Default)]
pub struct EffectiveIds {
    pub character_id: Option<String>,
    pub character_revision: Option<u64>,
    pub persona_id: Option<String>,
    pub persona_revision: Option<u64>,
    pub preset_id: Option<String>,
    pub preset_revision: Option<u64>,
    pub lorebook_revision: Option<u64>,
    pub scene_id: Option<String>,
    pub state_revision: Option<u64>,
    pub memory_revision: Option<u64>,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    // ── #114 effective config summary：本轮生效 Persona / 参数来源 ──────────
    // 全部为 `Option<String>` / `Option<f32>` / `Option<u32>`，旧 trace 反序列化
    // 不需要这些字段；新字段缺失时视为 "未提供"。WebUI 仅在字段非空时追加 source 后缀。
    /// Persona 激活来源：`explicit` / `session_binding` / `character_binding` / `default` / `absent`。
    pub persona_activation_source: Option<String>,
    /// Persona 显示名（不暴露 variables / api_key 等敏感字段，只暴露 name）。
    ///
    /// 与 `persona_id` 的关系：两者同时为 `Some` 或同时为 `None`（除 absent 路径外）；
    /// `persona_name` 是 `persona_id` 对应 Persona 的显示名快照，避免 WebUI 二次查询存储。
    pub persona_name: Option<String>,
    /// `provider` 来源：`request` / `snapshot`。
    pub provider_source: Option<String>,
    /// `model` 来源：`request` / `preset` / `snapshot`。
    pub model_source: Option<String>,
    /// 本轮生效 temperature（如有）。
    pub temperature: Option<f32>,
    /// `temperature` 来源：`request` / `preset`；None 表示两者均未指定。
    pub temperature_source: Option<String>,
    /// 本轮生效 max_tokens（如有）。
    pub max_tokens: Option<u32>,
    /// `max_tokens` 来源：`request` / `preset`；None 表示两者均未指定。
    pub max_tokens_source: Option<String>,
}

/// A diagnostic produced while assembling or filtering segments.
#[derive(Debug, Clone, Serialize)]
pub struct PromptDiagnostic {
    pub kind: String,
    pub message: String,
}

impl PromptAssemblyTrace {
    /// Construct a trace from explicitly instrumented segments.
    ///
    /// Callers own segment ordering and provenance. Totals are derived here so they cannot
    /// drift from the serialized segment metadata.
    pub fn new(
        effective: EffectiveIds,
        segments: Vec<PromptSegment>,
        diagnostics: Vec<PromptDiagnostic>,
    ) -> Self {
        let total_chars = segments.iter().map(|segment| segment.chars).sum();
        let total_estimated_tokens = segments
            .iter()
            .map(|segment| segment.estimated_tokens)
            .sum();
        Self {
            segments,
            diagnostics,
            total_chars,
            total_estimated_tokens,
            effective,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effective() -> EffectiveIds {
        EffectiveIds {
            character_id: Some("alice".to_string()),
            character_revision: Some(3),
            persona_id: Some("default".to_string()),
            persona_revision: Some(7),
            preset_id: Some("balanced".to_string()),
            preset_revision: Some(2),
            state_revision: Some(42),
            provider: "openai".to_string(),
            endpoint: "https://test/v1".to_string(),
            model: "test-model".to_string(),
            ..Default::default()
        }
    }

    fn segment(kind: &str, position: usize, chars: usize, tokens: usize) -> PromptSegment {
        PromptSegment {
            source_kind: kind.to_string(),
            source_id: Some(format!("{kind}-source")),
            item_id: None,
            display_name: None,
            role: Some("system".to_string()),
            position,
            enabled_reason: Some("selected by effective configuration".to_string()),
            chars,
            estimated_tokens: tokens,
            truncated: false,
            stable_or_volatile: if kind == "state" {
                Stability::Volatile
            } else {
                Stability::Stable
            },
        }
    }

    #[test]
    fn empty_trace_has_zero_totals() {
        let trace = PromptAssemblyTrace::new(effective(), Vec::new(), Vec::new());
        assert!(trace.segments.is_empty());
        assert_eq!(trace.total_chars, 0);
        assert_eq!(trace.total_estimated_tokens, 0);
    }

    #[test]
    fn preserves_instrumented_order_and_derives_totals() {
        let segments = vec![
            segment("card", 0, 12, 3),
            segment("preset", 12, 20, 5),
            segment("state", 32, 8, 2),
        ];
        let trace = PromptAssemblyTrace::new(effective(), segments, Vec::new());

        let kinds: Vec<_> = trace
            .segments
            .iter()
            .map(|segment| segment.source_kind.as_str())
            .collect();
        assert_eq!(kinds, ["card", "preset", "state"]);
        assert_eq!(trace.total_chars, 40);
        assert_eq!(trace.total_estimated_tokens, 10);
        assert_eq!(trace.effective.character_revision, Some(3));
        assert_eq!(trace.effective.persona_revision, Some(7));
        assert_eq!(trace.effective.preset_revision, Some(2));
    }

    #[test]
    fn serializes_provenance_without_prompt_content() {
        let trace = PromptAssemblyTrace::new(
            effective(),
            vec![segment("lorebook", 0, 16, 4)],
            vec![PromptDiagnostic {
                kind: "lorebook_trigger_miss".to_string(),
                message: "entry was excluded".to_string(),
            }],
        );

        let value = serde_json::to_value(trace).unwrap();
        assert_eq!(value["segments"][0]["source_id"], "lorebook-source");
        assert_eq!(value["segments"][0]["chars"], 16);
        assert_eq!(value["segments"][0]["stable_or_volatile"], "stable");
        assert!(value["segments"][0].get("content").is_none());
        assert_eq!(value["diagnostics"][0]["kind"], "lorebook_trigger_miss");
    }
}
