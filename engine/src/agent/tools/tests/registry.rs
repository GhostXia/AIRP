// Registry contract tests for `agent::tools`.
//
// 本文件固定 `default_registry` 的 30 工具契约：名称、排序、description、
// side_effect 精确快照。任何工具的新增/删除/改名/metadata 变更都会立刻
// 被快照测试捕获，迫使开发者 conscious 更新而非静默漂移。

use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn echo_still_works_after_registry_change() {
    // default_registry 改签名不应破坏 M_AGENT-1 的 echo
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    let reg = default_registry(state);
    let echo = reg.get("echo").unwrap();
    let r = echo
        .call(serde_json::json!({"probe": "still-here"}), false)
        .await
        .unwrap();
    assert_eq!(r.output["probe"], "still-here");
}

#[test]
fn tools_are_not_generation_evidence_sources_by_default() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    let reg = default_registry(state);
    let ordinary_tool = reg.get("list_characters").unwrap();
    let result = ToolResult {
        output: serde_json::json!({"secret-shaped-but-untrusted": "raw output"}),
        dry_run: false,
    };
    assert!(ordinary_tool.evidence_candidates(&result).is_empty());
    assert!(ordinary_tool.planner_projection(&result).is_none());
    let planner_tools = reg.planner_list();
    assert!(planner_tools
        .iter()
        .all(|(tool, _)| tool.name != "list_characters"));
    assert!(planner_tools.iter().any(|(tool, mode)| {
        tool.name == "get_character_state" && *mode == PlannerResultMode::Projected
    }));
    assert!(planner_tools.iter().any(|(tool, mode)| {
        tool.name == "list_world_events" && *mode == PlannerResultMode::Projected
    }));
    assert!(reg
        .get("echo")
        .unwrap()
        .planner_projection(&result)
        .is_some());
    let state_result = ToolResult {
        output: serde_json::json!({
            "revision": 7,
            "updated_at": "must-not-be-projected",
            "state": {"hp": 90}
        }),
        dry_run: false,
    };
    let state_projection = reg
        .get("get_character_state")
        .unwrap()
        .planner_projection(&state_result)
        .unwrap();
    assert_eq!(state_projection.revision, Some(7));
    assert_eq!(state_projection.content["state"]["hp"], 90);
    assert!(state_projection.content.get("updated_at").is_none());

    let events_result = ToolResult {
        output: serde_json::json!([{
            "id": "storm",
            "name": "Storm",
            "description": "A storm approaches",
            "triggered": false,
            "content": "must-not-be-projected",
            "future_sensitive_field": "must-not-be-projected"
        }]),
        dry_run: false,
    };
    let events_projection = reg
        .get("list_world_events")
        .unwrap()
        .planner_projection(&events_result)
        .unwrap();
    assert_eq!(events_projection.content["events"][0]["id"], "storm");
    assert!(events_projection.content["events"][0]
        .get("content")
        .is_none());
    assert!(events_projection.content["events"][0]
        .get("future_sensitive_field")
        .is_none());
}

#[test]
fn default_registry_includes_expected_tool_names() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    let reg = default_registry(state);

    for name in [
        "echo",
        "list_sessions",
        "start_session",
        "append_message",
        "get_recent_context",
        "rollback_messages",
        "list_characters",
        "get_character",
        "delete_character",
        "get_character_state",
        "update_character_state",
        "get_lorebook",
        "update_lorebook",
        "apply_lorebook",
        "merge_lorebooks",
        "get_preset",
        "update_preset",
        "seal_volume",
        "export_context_bundle",
        "enhance_analysis",
        "apply_enhanced_analysis",
    ] {
        assert!(reg.get(name).is_some(), "missing tool: {name}");
    }
}

#[test]
fn registry_capability_and_allowlist_are_authoritative() {
    let tmp = tempdir().unwrap();
    let reg = default_registry(make_state(tmp.path().to_path_buf()));
    assert!(!reg.allowed("echo", &[], None));
    assert!(reg.allowed("echo", &[Capability::CallTool], None));
    assert!(!reg.allowed(
        "echo",
        &[Capability::CallTool],
        Some(&["list_characters".to_string()])
    ));
    assert!(!reg.allowed("not_registered", &[Capability::CallTool], None));
}

#[test]
fn register_rejects_duplicate_tool_name() {
    // 同名工具二次注册必须报错，绝不静默覆盖（issue #24）。
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(EchoTool))
        .expect("first echo registers");
    let err = reg
        .register(Box::new(EchoTool))
        .expect_err("duplicate echo must be rejected");
    assert!(matches!(err, AirpError::Config(_)));
    // 首个注册仍在，未被顶掉。
    assert!(reg.get("echo").is_some());
}

#[test]
fn register_rejects_internal_planner_tool_name() {
    struct ReservedNameTool;
    impl Tool for ReservedNameTool {
        fn meta(&self) -> ToolMeta {
            ToolMeta {
                name: INTERNAL_GENERATE_TOOL,
                description: "must never register",
                side_effect: ToolSideEffect::Readonly,
            }
        }

        fn call(
            &self,
            _params: serde_json::Value,
            _confirm: bool,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<ToolResult, AirpError>> + Send + '_>,
        > {
            Box::pin(async { unreachable!() })
        }
    }

    let mut registry = ToolRegistry::new();
    assert!(matches!(
        registry.register(Box::new(ReservedNameTool)),
        Err(AirpError::Config(_))
    ));
}

/// #155 PR 2 强化：对 `default_registry` 的 30 个内建工具做精确快照，
/// 固定每个工具的 name / description / side_effect。
///
/// 任何 metadata 文案改动（哪怕一个字符）都会被此测试捕获，迫使开发者
/// 有意识地更新快照——防止“趁移动改文案”或“不知不觉改了 side_effect”。
///
/// 快照按 name 字典序排列，与 `ToolRegistry::list` 的排序一致。
#[test]
fn default_registry_exposes_sorted_30_tool_snapshot_with_descriptions_and_side_effects() {
    let tmp = tempdir().unwrap();
    let reg = default_registry(make_state(tmp.path().to_path_buf()));
    let tools = reg.list();

    // ── 数量 ────────────────────────────────────────────────────────────
    assert_eq!(
        tools.len(),
        30,
        "default_registry must expose exactly 30 built-in tools"
    );

    // ── 排序 ────────────────────────────────────────────────────────────
    let names: Vec<_> = tools.iter().map(|t| t.name).collect();
    assert!(
        names.windows(2).all(|pair| pair[0] <= pair[1]),
        "ToolRegistry::list must return tools sorted by name"
    );

    // ── 精确快照：name + description + side_effect ──────────────────────
    // 顺序与 names 一致（字典序）。
    let expected: &[(&str, &str, ToolSideEffect)] = &[
        (
            "advance_clock",
            "Advance the world clock by N time units (default: advance_per_turn). Checks and triggers any time-based world events.",
            ToolSideEffect::Mutate,
        ),
        (
            "advance_plot",
            "Advance the plot by introducing a new development, resolving a subplot, or escalating tension.",
            ToolSideEffect::Mutate,
        ),
        (
            "append_message",
            "Append a message to the character's current chat log. role \u{2208} {user,assistant,system}.",
            ToolSideEffect::Append,
        ),
        (
            "apply_enhanced_analysis",
            "Write a confirmed enhanced_md to a character analysis MD file. Destructive \u{2014} dry-run unless confirm=true. World book entries are read-only and rejected.",
            ToolSideEffect::Destructive,
        ),
        (
            "apply_lorebook",
            "Return enabled lorebook entries triggered by the supplied text.",
            ToolSideEffect::Readonly,
        ),
        (
            "delete_character",
            "Delete a character's entire directory subtree (all files under data/characters/{id}/). Destructive \u{2014} dry-run unless confirm=true.",
            ToolSideEffect::Destructive,
        ),
        (
            "echo",
            "M_AGENT-1 mock: returns its input verbatim. Verifies loop\u{2192}tool\u{2192}subagent wiring.",
            ToolSideEffect::Readonly,
        ),
        (
            "enhance_analysis",
            "Read a character analysis MD file, call LLM to fill placeholders, and return a diff preview (readonly, no write). World book entries are read-only and rejected.",
            ToolSideEffect::Readonly,
        ),
        (
            "export_context_bundle",
            "Write a bounded generic-Markdown context bundle for an isolated subagent under the AIRP data root.",
            ToolSideEffect::Mutate,
        ),
        (
            "get_character",
            "Read a character's card.json as a parsed JSON object. Invalid JSON is reported as data corruption.",
            ToolSideEffect::Readonly,
        ),
        (
            "get_character_state",
            "Read a character's current state with revision metadata.",
            ToolSideEffect::Readonly,
        ),
        (
            "get_clock",
            "Get the current world clock state for a character.",
            ToolSideEffect::Readonly,
        ),
        (
            "get_lorebook",
            "Read the normalized AIRP v1 lorebook for a character.",
            ToolSideEffect::Readonly,
        ),
        (
            "get_plot_status",
            "Get the current plot progress, including recent developments and pending plotlines.",
            ToolSideEffect::Readonly,
        ),
        (
            "get_preset",
            "Read a preset's canonical prompts array by preset_id. Returns AIRP v1 normalized prompts.",
            ToolSideEffect::Readonly,
        ),
        (
            "get_recent_context",
            "Get the most recent N messages of a character's chat log (default N=20).",
            ToolSideEffect::Readonly,
        ),
        (
            "list_characters",
            "List all available character ids (folder names under data/characters/).",
            ToolSideEffect::Readonly,
        ),
        (
            "list_sessions",
            "List all named sessions for a character.",
            ToolSideEffect::Readonly,
        ),
        (
            "list_world_events",
            "List all world events for a character.",
            ToolSideEffect::Readonly,
        ),
        (
            "merge_lorebooks",
            "Merge character lorebooks without writing them; strategy is union or primary_only.",
            ToolSideEffect::Readonly,
        ),
        (
            "npc_action",
            "Execute an NPC autonomous action. The action result will be injected into the narrative context.",
            ToolSideEffect::Mutate,
        ),
        (
            "rollback_messages",
            "Rollback the chat log to keep only messages [0..=index]. Destructive \u{2014} dry-run unless confirm=true.",
            ToolSideEffect::Destructive,
        ),
        (
            "seal_volume",
            "Summarize current session memory into the next volume and clear current.md. Destructive \u{2014} dry-run unless confirmed.",
            ToolSideEffect::Destructive,
        ),
        (
            "session_search",
            "Search through all historical conversations using full-text search.",
            ToolSideEffect::Readonly,
        ),
        (
            "start_session",
            "Create a new named session for a character. session_id is auto-generated (UUID).",
            ToolSideEffect::Mutate,
        ),
        (
            "trigger_world_event",
            "Trigger a world event by ID. The event content will be injected into the narrative context.",
            ToolSideEffect::Mutate,
        ),
        (
            "update_character_state",
            "Replace a character's whole state using character_id, state, and expected_revision from get_character_state; stale revisions conflict.",
            ToolSideEffect::Mutate,
        ),
        (
            "update_lorebook",
            "Replace a character's lorebook. Accepts AIRP canonical or SillyTavern form; normalizes via shared WorldbookNormalizer.",
            ToolSideEffect::Destructive,
        ),
        (
            "update_preset",
            "Replace a preset. Accepts SillyTavern or AIRP canonical form; normalizes via shared normalizer. Destructive: requires confirm=true.",
            ToolSideEffect::Destructive,
        ),
        (
            "update_relationship",
            "Update the relationship between two characters. Stores in state/live.json relationships matrix.",
            ToolSideEffect::Mutate,
        ),
    ];

    assert_eq!(
        expected.len(),
        30,
        "expected snapshot must also have 30 entries"
    );

    for (actual, (name, description, side_effect)) in tools.iter().zip(expected) {
        assert_eq!(
            actual.name, *name,
            "tool name mismatch at index {}",
            actual.name
        );
        assert_eq!(
            actual.description, *description,
            "description mismatch for tool {:?}",
            actual.name
        );
        assert_eq!(
            actual.side_effect, *side_effect,
            "side_effect mismatch for tool {:?}",
            actual.name
        );
    }
}

/// #329 N2：`plugin_tool::BUILTIN_TOOL_NAMES` 必须与 `default_registry` 实际注册的
/// 工具名保持完全一致（数量、名称、拼写、字典序）。
///
/// 新增/删除/改名内建工具时若忘记更新 `plugin_tool::BUILTIN_TOOL_NAMES`，此测试会
/// 立刻失败，迫使开发者 conscious 同步——防止"忘记更新清单"导致
/// `validate_tool_name` 漏拒插件工具与内建工具重名。
#[test]
fn builtin_tool_names_match_default_registry() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    let reg = default_registry(state);
    let actual: Vec<&str> = reg.list().iter().map(|t| t.name).collect();

    // BUILTIN_TOOL_NAMES 在源码中已按字典序声明；这里再 sort 一次防御性编程。
    let mut expected: Vec<&str> = crate::plugin_tool::BUILTIN_TOOL_NAMES.to_vec();
    expected.sort();

    assert_eq!(
        actual, expected,
        "BUILTIN_TOOL_NAMES must exactly match default_registry tool names; \
         if you added/removed/renamed a builtin tool, update BUILTIN_TOOL_NAMES \
         in engine/src/plugin_tool.rs"
    );
}
