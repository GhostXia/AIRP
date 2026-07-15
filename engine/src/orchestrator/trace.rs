//! Prompt assembly trace data model (#115 P1).
//!
//! The trace is populated explicitly by prompt assembly call sites so segment order and
//! provenance can match the provider payload. This module intentionally does not infer
//! provenance from marker text in a completed prompt.

use serde::Serialize;

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
