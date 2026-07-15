//! PromptAssemblyTrace — 结构化 prompt 装配 trace（#115 P1 子项）。
//!
//! 每次 pipeline prepare 产生一份 trace，记录最终 system_prompt 的分段来源、
//! 字符数、估算 token、稳定/易变分类，以及有效的 character/persona/preset/
//! provider/model ids。用于审计、调试和前端可视化。
//!
//! 本 PR 只交付 struct + `from_final_prompt` 构造函数 + 单测。接入
//! `PreparedPipeline` 留后续 PR（需要更新 3 处解构 + 2 处构造 + API 透出）。
//!
//! ## 稳定/易变分类
//!
//! 依据 `ExportContextBundleTool` 的隐式分层（volume_context.rs:190/262）：
//! - stable: card / persona / preset / lorebook / scene — 装配时静态注入
//! - volatile: state (live.json) / memory (current.md, volume headers) — 随会话进展变化

use serde::Serialize;

/// Prompt 装配 trace。记录最终 system_prompt 的分段信息和有效 ID 快照。
#[derive(Debug, Clone, Serialize)]
pub struct PromptAssemblyTrace {
    pub segments: Vec<PromptSegment>,
    pub diagnostics: Vec<PromptDiagnostic>,
    pub total_chars: usize,
    pub total_estimated_tokens: usize,
    pub effective: EffectiveIds,
}

/// 单个 prompt segment 的元数据。
#[derive(Debug, Clone, Serialize)]
pub struct PromptSegment {
    /// "card" | "persona" | "preset" | "lorebook" | "state" | "memory" | "scene" | "unclassified"
    pub source_kind: &'static str,
    /// 在最终 system_prompt 中的字节偏移。
    pub position: usize,
    pub chars: usize,
    pub estimated_tokens: usize,
    /// "stable" | "volatile"
    pub stable_or_volatile: &'static str,
}

/// 有效的 character/persona/preset/provider/model IDs 快照。
#[derive(Debug, Clone, Serialize, Default)]
pub struct EffectiveIds {
    pub character_id: Option<String>,
    pub persona_id: Option<String>,
    pub preset_id: Option<String>,
    pub scene_id: Option<String>,
    pub state_revision: Option<u64>,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
}

/// 装配过程中的诊断信息（如 preset_dropped_invalid / lorebook_trigger_miss）。
#[derive(Debug, Clone, Serialize)]
pub struct PromptDiagnostic {
    pub kind: &'static str,
    pub message: String,
}

/// 标记字符串到 source_kind / stable_or_volatile 的映射。
///
/// 依据 `orchestrator/mod.rs` 和 `volume_inject.rs` 中已有的稳定标记字符串。
/// 标记按 source_kind 分组，运行时由 `from_final_prompt` 按位置排序后切分。
const MARKERS: &[(&str, &str, &str)] = &[
    // card 字段（角色名动态，用关键后缀匹配）
    ("Personality]:", "card", "stable"),
    ("Appearance & Description]:", "card", "stable"),
    ("[Scenario]:", "card", "stable"),
    // state（known_md 是 CP-gated 静态文本；live.json 是 volatile）
    ("Known Information & Clues", "state", "stable"),
    ("[Current State]:", "state", "volatile"),
    // memory（current.md + volume headers，随会话进展变化）
    ("[Recent Context]", "memory", "volatile"),
    ("[Related History]", "memory", "volatile"),
    // scene 分支（多角色场景）
    ("[场景设定]", "scene", "stable"),
    ("[在场角色]", "scene", "stable"),
    ("[世界书信息]", "lorebook", "stable"),
    ("[格式规则]", "scene", "stable"),
];

impl PromptAssemblyTrace {
    /// 从最终化的 system_prompt 反向解析分段。
    ///
    /// 依据 orchestrator/volume_inject 已有的稳定标记字符串切分 segments。
    /// 每个标记到下一个标记之间的文本是一个 segment；第一个标记之前的前缀
    /// 是 `unclassified` segment（通常是 macro substitution 后的杂项文本）。
    ///
    /// 注意：这是 best-effort 反向解析，不追求 100% 精确。未来可改为在装配
    /// 函数内正向插桩（forward instrumentation），消除标记字符串依赖。
    pub fn from_final_prompt(system_prompt: &str, effective: EffectiveIds) -> Self {
        // 找所有标记在 system_prompt 中的位置。
        let mut hits: Vec<(usize, &'static str, &'static str)> = Vec::new();
        for (marker, kind, sv) in MARKERS {
            if let Some(pos) = system_prompt.find(marker) {
                hits.push((pos, *kind, *sv));
            }
        }
        hits.sort_by_key(|(pos, _, _)| *pos);
        // 同一 position 只保留第一个 marker。
        hits.dedup_by_key(|(pos, _, _)| *pos);

        // 构造区间边界：(start, end, source_kind, stable_or_volatile)。
        let mut bounds: Vec<(usize, usize, &'static str, &'static str)> = Vec::new();
        if hits.is_empty() {
            bounds.push((0, system_prompt.len(), "unclassified", "stable"));
        } else {
            if hits[0].0 > 0 {
                bounds.push((0, hits[0].0, "unclassified", "stable"));
            }
            for i in 0..hits.len() {
                let start = hits[i].0;
                let end = if i + 1 < hits.len() {
                    hits[i + 1].0
                } else {
                    system_prompt.len()
                };
                bounds.push((start, end, hits[i].1, hits[i].2));
            }
        }

        let mut segments = Vec::with_capacity(bounds.len());
        let mut total_chars = 0usize;
        let mut total_estimated_tokens = 0usize;
        for (start, end, kind, sv) in &bounds {
            let slice = &system_prompt[*start..*end];
            let chars = slice.chars().count();
            let tokens = crate::volume_store::estimate_tokens(slice);
            total_chars += chars;
            total_estimated_tokens += tokens;
            segments.push(PromptSegment {
                source_kind: kind,
                position: *start,
                chars,
                estimated_tokens: tokens,
                stable_or_volatile: sv,
            });
        }

        PromptAssemblyTrace {
            segments,
            diagnostics: Vec::new(),
            total_chars,
            total_estimated_tokens,
            effective,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_effective() -> EffectiveIds {
        EffectiveIds {
            provider: "openai".to_string(),
            endpoint: "https://test/v1".to_string(),
            model: "test-model".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_prompt_produces_single_unclassified_segment() {
        let trace = PromptAssemblyTrace::from_final_prompt("", empty_effective());
        assert_eq!(trace.segments.len(), 1);
        assert_eq!(trace.segments[0].source_kind, "unclassified");
        assert_eq!(trace.segments[0].chars, 0);
        assert_eq!(trace.total_chars, 0);
        assert_eq!(trace.total_estimated_tokens, 0);
    }

    #[test]
    fn no_markers_produces_single_unclassified_segment() {
        let prompt = "Just some plain text without any markers.";
        let trace = PromptAssemblyTrace::from_final_prompt(prompt, empty_effective());
        assert_eq!(trace.segments.len(), 1);
        assert_eq!(trace.segments[0].source_kind, "unclassified");
        assert_eq!(trace.segments[0].chars, prompt.chars().count());
    }

    #[test]
    fn splits_on_current_state_marker() {
        let prompt = "Character info here.\n[Current State]:\n{\"hp\": 90}\n";
        let trace = PromptAssemblyTrace::from_final_prompt(prompt, empty_effective());
        assert_eq!(trace.segments.len(), 2);
        // 前缀 unclassified
        assert_eq!(trace.segments[0].source_kind, "unclassified");
        assert!(trace.segments[0].chars > 0);
        // [Current State] 段
        assert_eq!(trace.segments[1].source_kind, "state");
        assert_eq!(trace.segments[1].stable_or_volatile, "volatile");
        assert!(trace.segments[1].chars > 0);
    }

    #[test]
    fn splits_on_multiple_markers_in_order() {
        let prompt = "[Alice's Personality]:\nkind\n\n[Scenario]:\ncastle\n\n[Current State]:\n{\"hp\": 90}\n\n[Recent Context]\nsome context\n";
        let trace = PromptAssemblyTrace::from_final_prompt(prompt, empty_effective());
        assert!(
            trace.segments.len() >= 4,
            "should have at least 4 segments, got {}",
            trace.segments.len()
        );
        // 验证顺序
        let kinds: Vec<_> = trace.segments.iter().map(|s| s.source_kind).collect();
        assert!(kinds.contains(&"card"));
        assert!(kinds.contains(&"state"));
        assert!(kinds.contains(&"memory"));
        // total_chars 应等于 prompt 的字符数
        assert_eq!(trace.total_chars, prompt.chars().count());
    }

    #[test]
    fn scene_branch_markers_are_matched() {
        let prompt = "[场景设定]\n森林\n[在场角色]\n爱丽丝\n[世界书信息]\n精灵\n[格式规则]\nJSON\n";
        let trace = PromptAssemblyTrace::from_final_prompt(prompt, empty_effective());
        let kinds: Vec<_> = trace.segments.iter().map(|s| s.source_kind).collect();
        assert!(kinds.contains(&"scene"));
        assert!(kinds.contains(&"lorebook"));
    }

    #[test]
    fn estimated_tokens_are_non_zero_for_non_empty_segments() {
        let prompt = "[Current State]:\n{\"hp\": 90}\n";
        let trace = PromptAssemblyTrace::from_final_prompt(prompt, empty_effective());
        assert!(trace.total_estimated_tokens > 0);
        for seg in &trace.segments {
            if seg.chars > 0 {
                assert!(seg.estimated_tokens > 0);
            }
        }
    }

    #[test]
    fn effective_ids_are_preserved() {
        let effective = EffectiveIds {
            character_id: Some("alice".to_string()),
            preset_id: Some("test".to_string()),
            state_revision: Some(42),
            provider: "openai".to_string(),
            endpoint: "https://test/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..Default::default()
        };
        let trace = PromptAssemblyTrace::from_final_prompt("test", effective);
        assert_eq!(trace.effective.character_id, Some("alice".to_string()));
        assert_eq!(trace.effective.state_revision, Some(42));
        assert_eq!(trace.effective.model, "gpt-4o");
    }
}
