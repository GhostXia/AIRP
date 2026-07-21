// PR #272 阶段三：Agent RP 差异化工具（world_event / npc / plot）端到端测试。
//
// 测试策略：
// - 通过 `default_registry` 拉起 6 个新工具，端到端验证 call → output → 落盘；
// - 重点覆盖审计修复点：
//   * `update_relationship` / `advance_plot` 走 `StateService::mutate` 后，
//     live.json 必须出现 revision 合同产物（history.jsonl + revisions/{n}/ 快照）；
//   * `trigger_world_event` 的 `triggered` 标志幂等：重复触发同一 event_id
//     应返回 `success: false`，且 current.md 不被二次注入；
//   * `update_relationship` 与 `advance_plot` 并发不会丢失任何一方的更新
//     （同一 character_id 下，state_lock 串行化）。
//
// 不覆盖：world_events.json 的 revision 合同（审计遗留项，本 PR 未接入）。

use super::*;
use futures_util::future::join_all;
use tempfile::tempdir;

/// Helper：在 data_root 下创建一个空 character 目录（card.json 占位），
/// 让 `resolve_session_dir` / `char_state_dir` 等 helper 能正常工作。
fn seed_character(data_root: &std::path::Path, id: &str) {
    let card_dir = data_root.join("characters").join(id).join("card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(card_dir.join("card.json"), r#"{"name":"Test"}"#).unwrap();
}

#[tokio::test]
async fn update_relationship_writes_live_json_with_revision_contract() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    let tool = reg.get("update_relationship").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "alice",
                "from": "alice",
                "to": "bob",
                "relation_type": "ally",
                "intensity": 0.8
            }),
            false,
        )
        .await
        .unwrap();

    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["from"], "alice");
    assert_eq!(result.output["to"], "bob");
    assert_eq!(result.output["relation_type"], "ally");
    assert_eq!(result.output["intensity"], 0.8);
    // 审计修复：output 必须含 revision（来自 StateService::mutate）。
    assert_eq!(result.output["revision"], 1);

    // live.json 必须落盘且包含 relationships 字段。
    let live_path = state.data_root.join("characters/alice/state/live.json");
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&live_path).unwrap()).unwrap();
    assert_eq!(live["relationships"]["alice->bob"]["type"], "ally");
    assert_eq!(live["relationships"]["alice->bob"]["intensity"], 0.8);

    // 审计修复：必须接入 #115 Phase 2e revision 合同。
    // history.jsonl 应有 1 行，revisions/1/state.json 应存在。
    let history_path = state.data_root.join("characters/alice/state/history.jsonl");
    let history = std::fs::read_to_string(&history_path).unwrap();
    assert_eq!(history.lines().count(), 1);

    let revision_state = state
        .data_root
        .join("characters/alice/state/revisions/1/state.json");
    assert!(revision_state.exists(), "revision 1 snapshot must exist");
}

#[tokio::test]
async fn advance_plot_appends_plot_history_under_revision_contract() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    let tool = reg.get("advance_plot").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "alice",
                "development": "The tower doors swung open",
                "type": "progression"
            }),
            false,
        )
        .await
        .unwrap();

    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["type"], "progression");
    assert_eq!(
        result.output["development"],
        "The tower doors swung open"
    );
    assert_eq!(result.output["revision"], 1);

    // current.md 应被注入剧情推进 entry。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("[剧情推进: progression]"));
    assert!(current.contains("The tower doors swung open"));

    // live.json 应含 plot_history 数组。
    let live_path = state.data_root.join("characters/alice/state/live.json");
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&live_path).unwrap()).unwrap();
    let history = live["plot_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["type"], "progression");
    assert_eq!(history[0]["development"], "The tower doors swung open");
}

#[tokio::test]
async fn get_plot_status_returns_history_and_pending_clues() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    // 先 advance_plot 写入一条 plot_history。
    let advance = reg.get("advance_plot").unwrap();
    advance
        .call(
            serde_json::json!({
                "character_id": "alice",
                "development": "Setup scene",
                "type": "setup"
            }),
            false,
        )
        .await
        .unwrap();

    let get_status = reg.get("get_plot_status").unwrap();
    let result = get_status
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();

    let history = result.output["plot_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["type"], "setup");
    assert_eq!(history[0]["development"], "Setup scene");
    // pending_clues 为空字符串（index.md 不存在 → unwrap_or_default）。
    assert_eq!(result.output["pending_clues"], "");
}

#[tokio::test]
async fn trigger_world_event_injects_and_marks_triggered() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    // 准备 world_events.json
    let events_path = state.data_root.join("characters/alice/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([{
            "id": "evt_001",
            "name": "Storm",
            "description": "A sudden storm",
            "trigger_keywords": ["storm"],
            "content": "Lightning split the sky."
        }])
        .to_string(),
    )
    .unwrap();

    let tool = reg.get("trigger_world_event").unwrap();
    let result = tool
        .call(
            serde_json::json!({"character_id": "alice", "event_id": "evt_001"}),
            false,
        )
        .await
        .unwrap();

    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["event"]["id"], "evt_001");
    assert_eq!(result.output["event"]["name"], "Storm");

    // current.md 应含事件注入。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("[世界事件: Storm]"));
    assert!(current.contains("Lightning split the sky."));

    // world_events.json 中 triggered 应为 true。
    let events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(events[0]["triggered"], true);
}

#[tokio::test]
async fn trigger_world_event_is_idempotent_for_already_triggered() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    let events_path = state.data_root.join("characters/alice/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([{
            "id": "evt_002",
            "name": "Festival",
            "description": "Annual festival",
            "content": "The town square fills with color."
        }])
        .to_string(),
    )
    .unwrap();

    let tool = reg.get("trigger_world_event").unwrap();

    // 第一次触发：成功。
    let first = tool
        .call(
            serde_json::json!({"character_id": "alice", "event_id": "evt_002"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(first.output["success"], true);

    // 第二次触发：应返回 success: false，且 current.md 不再被注入。
    let second = tool
        .call(
            serde_json::json!({"character_id": "alice", "event_id": "evt_002"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(second.output["success"], false);
    assert_eq!(second.output["message"], "event already triggered");

    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    // 仅出现一次 festival 内容，杜绝审计前双重注入的竞态。
    let occurrences = current.matches("The town square fills with color.").count();
    assert_eq!(occurrences, 1);
}

#[tokio::test]
async fn trigger_world_event_unknown_id_returns_not_found() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state);

    let tool = reg.get("trigger_world_event").unwrap();
    let err = tool
        .call(
            serde_json::json!({"character_id": "alice", "event_id": "missing"}),
            false,
        )
        .await
        .expect_err("unknown event_id must error");
    assert!(matches!(err, crate::error::AirpError::NotFound(_)));
}

#[tokio::test]
async fn list_world_events_reflects_triggered_state() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    let events_path = state.data_root.join("characters/alice/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([
            {"id": "a", "name": "A", "description": "desc a", "content": "x"},
            {"id": "b", "name": "B", "description": "desc b", "content": "y", "triggered": true}
        ])
        .to_string(),
    )
    .unwrap();

    let list = reg.get("list_world_events").unwrap();
    let result = list
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    let arr = result.output.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["triggered"], false);
    assert_eq!(arr[1]["triggered"], true);
}

#[tokio::test]
async fn npc_action_appends_to_current_md() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    let tool = reg.get("npc_action").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "alice",
                "npc_name": "Goblin",
                "action": "steals an apple",
                "result": "the merchant shouts"
            }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["npc_name"], "Goblin");

    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("[NPC行动: Goblin] steals an apple"));
    assert!(current.contains("结果: the merchant shouts"));
}

/// 审计修复关键测试：`update_relationship` 与 `advance_plot` 并发执行时，
/// 两者都写入 live.json（relationships + plot_history 字段），state_lock
/// 必须串行化它们的 read-modify-write，任何一方的更新不能被另一方覆盖。
///
/// 审计前的 bug：两个工具都做无锁 read-modify-write，并发时后写者会
/// 覆盖先写者的 relationships / plot_history 字段。审计后通过
/// StateService::mutate 共享 state_lock，此测试应稳定通过。
///
/// 实现说明：用 `join_all` 在同一 task 内并发 poll 多个 tool future。
/// 由于 `tool.call(...)` 返回的 future 借用 `&dyn Tool`（生命周期绑定
/// `reg`），无法 `tokio::spawn`；`join_all` 不要求 `'static`，可在同一
/// task 内并发调度。
#[tokio::test]
async fn concurrent_update_relationship_and_advance_plot_do_not_lose_updates() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "alice");
    let reg = default_registry(state.clone());

    // 同时发起 10 个调用（5 个 update_relationship + 5 个 advance_plot 交替）。
    let mut futures = Vec::new();
    for i in 0..5u32 {
        let update_tool = reg.get("update_relationship").unwrap();
        futures.push(update_tool.call(
            serde_json::json!({
                "character_id": "alice",
                "from": "alice",
                "to": format!("npc{i}"),
                "relation_type": "rival",
                "intensity": 0.3
            }),
            false,
        ));
        let advance_tool = reg.get("advance_plot").unwrap();
        futures.push(advance_tool.call(
            serde_json::json!({
                "character_id": "alice",
                "development": format!("event {i}"),
                "type": "progression"
            }),
            false,
        ));
    }

    // 并发 poll 全部 future，等待完成。任何一个返回 Err 都会让 join_all 整体 panic。
    let results = join_all(futures).await;
    for (i, result) in results.into_iter().enumerate() {
        assert!(result.is_ok(), "tool call #{i} failed: {:?}", result.err());
    }

    // 验证：live.json 必须同时包含 5 个 relationships 条目和 5 个 plot_history 条目。
    let live_path = state.data_root.join("characters/alice/state/live.json");
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&live_path).unwrap()).unwrap();

    let relationships = live["relationships"].as_object().unwrap();
    let plot_history = live["plot_history"].as_array().unwrap();

    assert_eq!(
        relationships.len(),
        5,
        "all 5 update_relationship calls must be reflected, got {}",
        relationships.len()
    );
    assert_eq!(
        plot_history.len(),
        5,
        "all 5 advance_plot calls must be reflected, got {}",
        plot_history.len()
    );
}
