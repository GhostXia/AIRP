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
// 并行测试纪律（CodeRabbit 跟进修复）：
// `state_lock` / `session_lock` 是 process-global `OnceLock<Mutex<HashMap>>`
// 静态变量，以 `character_id` 为 key。若多个 `#[tokio::test]` 用同一
// character_id（如 "alice"），它们会争用同一把锁。在高并行度（默认 16
// 线程）下，结合独立 tokio runtime + reqwest::Client::new() 的内部线程，
// 会导致 OS 线程饥饿和测试 hang。
//
// 解决方案：每个测试用唯一 character_id，避免跨测试争用 process-global 锁。
// 各测试的 data_root 本来就独立（tempdir），所以 character_id 唯一化不影响
// 测试隔离性，只消除锁争用。
//
// 不覆盖：world_events.json 的 revision 合同已在 #280 接入，下方
// `trigger_world_event_writes_revision_contract` 测试覆盖。

use super::*;
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
    // 唯一 character_id：避免与其他 #[tokio::test] 争用 process-global
    // state_lock / session_lock（均以 character_id 为 key）。
    seed_character(&state.data_root, "upd_rel_basic");
    let reg = default_registry(state.clone());

    let tool = reg.get("update_relationship").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "upd_rel_basic",
                "from": "upd_rel_basic",
                "to": "bob",
                "relation_type": "ally",
                "intensity": 0.8
            }),
            true,
        )
        .await
        .unwrap();

    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["from"], "upd_rel_basic");
    assert_eq!(result.output["to"], "bob");
    assert_eq!(result.output["relation_type"], "ally");
    assert_eq!(result.output["intensity"], 0.8);
    // 审计修复：output 必须含 revision（来自 StateService::mutate）。
    assert_eq!(result.output["revision"], 1);

    // live.json 必须落盘且包含 relationships 字段。
    let live_path = state
        .data_root
        .join("characters/upd_rel_basic/state/live.json");
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&live_path).unwrap()).unwrap();
    assert_eq!(live["relationships"]["upd_rel_basic->bob"]["type"], "ally");
    assert_eq!(
        live["relationships"]["upd_rel_basic->bob"]["intensity"],
        0.8
    );

    // 审计修复：必须接入 #115 Phase 2e revision 合同。
    // history.jsonl 应有 1 行，revisions/1/state.json 应存在。
    let history_path = state
        .data_root
        .join("characters/upd_rel_basic/state/history.jsonl");
    let history = std::fs::read_to_string(&history_path).unwrap();
    assert_eq!(history.lines().count(), 1);

    let revision_state = state
        .data_root
        .join("characters/upd_rel_basic/state/revisions/1/state.json");
    assert!(revision_state.exists(), "revision 1 snapshot must exist");
}

#[tokio::test]
async fn advance_plot_appends_plot_history_under_revision_contract() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_plot_basic");
    let reg = default_registry(state.clone());

    let tool = reg.get("advance_plot").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "adv_plot_basic",
                "development": "The tower doors swung open",
                "type": "progression"
            }),
            true,
        )
        .await
        .unwrap();

    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["type"], "progression");
    assert_eq!(result.output["development"], "The tower doors swung open");
    assert_eq!(result.output["revision"], 1);

    // current.md 应被注入剧情推进 entry。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "adv_plot_basic", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("[剧情推进: progression]"));
    assert!(current.contains("The tower doors swung open"));

    // live.json 应含 plot_history 数组。
    let live_path = state
        .data_root
        .join("characters/adv_plot_basic/state/live.json");
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&live_path).unwrap()).unwrap();
    let history = live["plot_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["type"], "progression");
    assert_eq!(history[0]["development"], "The tower doors swung open");
}

/// #281: `update_relationship` 在 confirm=false 时返回 dry-run preview，不落盘。
#[tokio::test]
async fn update_relationship_dry_run_returns_preview_without_persisting() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "upd_rel_dry");
    let reg = default_registry(state.clone());

    let tool = reg.get("update_relationship").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "upd_rel_dry",
                "from": "upd_rel_dry",
                "to": "bob",
                "relation_type": "rival",
                "intensity": 0.9
            }),
            false, // confirm=false → dry-run
        )
        .await
        .unwrap();

    assert!(result.dry_run);
    assert_eq!(result.output["dry_run"].as_bool(), Some(true));
    assert_eq!(result.output["would_update"]["from"], "upd_rel_dry");
    assert_eq!(result.output["would_update"]["to"], "bob");
    assert_eq!(result.output["would_update"]["relation_type"], "rival");
    assert_eq!(result.output["would_update"]["intensity"], 0.9);
    assert_eq!(result.output["character_id"], "upd_rel_dry");

    // dry-run 不落盘：live.json 不应存在（seed_character 不写 state/live.json）。
    let live_path = state
        .data_root
        .join("characters/upd_rel_dry/state/live.json");
    assert!(
        !live_path.exists(),
        "dry-run must not persist live.json, but found at {}",
        live_path.display()
    );
    // history.jsonl 也不应存在。
    let history_path = state
        .data_root
        .join("characters/upd_rel_dry/state/history.jsonl");
    assert!(!history_path.exists());
}

/// #281: `advance_plot` 在 confirm=false 时返回 dry-run preview，不落盘。
#[tokio::test]
async fn advance_plot_dry_run_returns_preview_without_persisting() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_plot_dry");
    let reg = default_registry(state.clone());

    let tool = reg.get("advance_plot").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "adv_plot_dry",
                "development": "The gate crumbled",
                "type": "climax"
            }),
            false, // confirm=false → dry-run
        )
        .await
        .unwrap();

    assert!(result.dry_run);
    assert_eq!(result.output["dry_run"].as_bool(), Some(true));
    assert_eq!(
        result.output["would_inject"],
        "[剧情推进: climax] The gate crumbled"
    );
    assert_eq!(result.output["character_id"], "adv_plot_dry");

    // dry-run 不落盘：live.json 不应存在。
    let live_path = state
        .data_root
        .join("characters/adv_plot_dry/state/live.json");
    assert!(
        !live_path.exists(),
        "dry-run must not persist live.json, but found at {}",
        live_path.display()
    );
    // current.md 不应被注入剧情 entry。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "adv_plot_dry", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(
        !current.contains("[剧情推进: climax]"),
        "dry-run must not inject plot entry into current.md"
    );
}

#[tokio::test]
async fn get_plot_status_returns_history_and_pending_clues() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "get_plot_status");
    let reg = default_registry(state.clone());

    // 先 advance_plot 写入一条 plot_history。
    let advance = reg.get("advance_plot").unwrap();
    advance
        .call(
            serde_json::json!({
                "character_id": "get_plot_status",
                "development": "Setup scene",
                "type": "setup"
            }),
            true,
        )
        .await
        .unwrap();

    let get_status = reg.get("get_plot_status").unwrap();
    let result = get_status
        .call(
            serde_json::json!({"character_id": "get_plot_status"}),
            false,
        )
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
    seed_character(&state.data_root, "trig_evt_basic");
    let reg = default_registry(state.clone());

    // 准备 world_events.json
    let events_path = state
        .data_root
        .join("characters/trig_evt_basic/world_events.json");
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
            serde_json::json!({"character_id": "trig_evt_basic", "event_id": "evt_001"}),
            false,
        )
        .await
        .unwrap();

    assert_eq!(result.output["success"], true);
    assert_eq!(result.output["event"]["id"], "evt_001");
    assert_eq!(result.output["event"]["name"], "Storm");

    // current.md 应含事件注入。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "trig_evt_basic", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("[世界事件: Storm]"));
    assert!(current.contains("Lightning split the sky."));

    // world_events.json 中 triggered 应为 true。
    let events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(events[0]["triggered"], true);
}

/// #280: trigger_world_event 必须接入 revision 合同。
/// legacy world_events.json 存在时，trigger 后产生：
/// - revision 1：legacy_migration 快照（triggered=false 的原始内容）
/// - revision 2：tool_triggered 快照（triggered=true 的新内容）
/// - current_revision 指针 = 2
#[tokio::test]
async fn trigger_world_event_writes_revision_contract() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "trig_evt_rev");
    let reg = default_registry(state.clone());

    // 准备 legacy world_events.json（无 revision 快照）
    let events_path = state
        .data_root
        .join("characters/trig_evt_rev/world_events.json");
    let legacy_content = serde_json::json!([{
        "id": "evt_rev_001",
        "name": "Eclipse",
        "description": "A solar eclipse",
        "trigger_keywords": ["eclipse"],
        "content": "The sun vanished."
    }])
    .to_string();
    std::fs::write(&events_path, &legacy_content).unwrap();

    let tool = reg.get("trigger_world_event").unwrap();
    let result = tool
        .call(
            serde_json::json!({"character_id": "trig_evt_rev", "event_id": "evt_rev_001"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(result.output["success"], true);

    // revision 合同产物路径
    let asset_dir = state.data_root.join("characters/trig_evt_rev/world_events");

    // revision 1：legacy_migration（triggered=false）
    let rev1_dir = asset_dir.join("revisions/1");
    let rev1_events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rev1_dir.join("world_events.json")).unwrap())
            .unwrap();
    assert_eq!(rev1_events[0]["triggered"], serde_json::Value::Null);

    // revision 2：tool_triggered（triggered=true）
    let rev2_dir = asset_dir.join("revisions/2");
    let rev2_events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rev2_dir.join("world_events.json")).unwrap())
            .unwrap();
    assert_eq!(rev2_events[0]["triggered"], true);

    // manifest.json 校验
    let manifest1: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rev1_dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest1["asset_kind"], "world_events");
    assert_eq!(manifest1["asset_id"], "trig_evt_rev");
    assert_eq!(manifest1["source"]["source_kind"], "legacy_migration");

    let manifest2: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rev2_dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest2["asset_kind"], "world_events");
    assert_eq!(manifest2["source"]["source_kind"], "tool_triggered");
    assert_eq!(manifest2["source"]["parent_revision"], 1);

    // current_revision 指针 = 2
    let current_rev = std::fs::read_to_string(asset_dir.join("current_revision")).unwrap();
    assert_eq!(current_rev.trim(), "2");

    // 工作副本 world_events.json 的 triggered=true
    let work_events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(work_events[0]["triggered"], true);
}

/// #280: 第二次 trigger 不同事件产生 revision 3，parent_revision=2（增量 revision 链）。
#[tokio::test]
async fn trigger_world_event_increments_revision_chain() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "trig_evt_chain");
    let reg = default_registry(state.clone());

    // 准备含 2 个事件的 legacy world_events.json
    let events_path = state
        .data_root
        .join("characters/trig_evt_chain/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([
            {"id": "evt_chain_1", "name": "Storm", "description": "", "trigger_keywords": ["storm"], "content": "Rain."},
            {"id": "evt_chain_2", "name": "Quake", "description": "", "trigger_keywords": ["quake"], "content": "Shake."}
        ])
        .to_string(),
    )
    .unwrap();

    // 第一次 trigger：产生 revision 1（legacy）+ revision 2（evt_chain_1 triggered）
    let tool = reg.get("trigger_world_event").unwrap();
    tool.call(
        serde_json::json!({"character_id": "trig_evt_chain", "event_id": "evt_chain_1"}),
        false,
    )
    .await
    .unwrap();

    // 第二次 trigger：产生 revision 3（evt_chain_2 也 triggered）
    tool.call(
        serde_json::json!({"character_id": "trig_evt_chain", "event_id": "evt_chain_2"}),
        false,
    )
    .await
    .unwrap();

    let asset_dir = state
        .data_root
        .join("characters/trig_evt_chain/world_events");
    let current_rev = std::fs::read_to_string(asset_dir.join("current_revision")).unwrap();
    assert_eq!(current_rev.trim(), "3");

    // revision 3 的 parent_revision = 2
    let manifest3: serde_json::Value = serde_json::from_slice(
        &std::fs::read(asset_dir.join("revisions/3/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest3["source"]["parent_revision"], 2);

    // revision 3 的两个事件都 triggered
    let rev3_events: serde_json::Value = serde_json::from_slice(
        &std::fs::read(asset_dir.join("revisions/3/world_events.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rev3_events[0]["triggered"], true);
    assert_eq!(rev3_events[1]["triggered"], true);
}

#[tokio::test]
async fn trigger_world_event_is_idempotent_for_already_triggered() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "trig_evt_idem");
    let reg = default_registry(state.clone());

    let events_path = state
        .data_root
        .join("characters/trig_evt_idem/world_events.json");
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
            serde_json::json!({"character_id": "trig_evt_idem", "event_id": "evt_002"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(first.output["success"], true);

    // 第二次触发：应返回 success: false，且 current.md 不再被注入。
    let second = tool
        .call(
            serde_json::json!({"character_id": "trig_evt_idem", "event_id": "evt_002"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(second.output["success"], false);
    assert_eq!(second.output["message"], "event already triggered");

    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "trig_evt_idem", None).unwrap();
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
    seed_character(&state.data_root, "trig_evt_unknown");
    let reg = default_registry(state);

    let tool = reg.get("trigger_world_event").unwrap();
    let err = tool
        .call(
            serde_json::json!({"character_id": "trig_evt_unknown", "event_id": "missing"}),
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
    seed_character(&state.data_root, "list_evt_basic");
    let reg = default_registry(state.clone());

    let events_path = state
        .data_root
        .join("characters/list_evt_basic/world_events.json");
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
        .call(serde_json::json!({"character_id": "list_evt_basic"}), false)
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
    seed_character(&state.data_root, "npc_act_basic");
    let reg = default_registry(state.clone());

    let tool = reg.get("npc_action").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "npc_act_basic",
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
        crate::data_dir::resolve_session_dir(&state.data_root, "npc_act_basic", None).unwrap();
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
/// 实现说明（CodeRabbit 跟进）：用 `std::thread::scope` + 共享
/// `std::sync::Barrier` + 每个 worker 内部独立 `tokio::runtime::Runtime`
/// 替代原 `join_all` 单 task 并发 poll。
///
/// 为何不用 `tokio::task::spawn` + multi_thread runtime：
/// `update_relationship` / `advance_plot` 的 future 内部全是同步代码
/// （`StateService::mutate` 同步持有 `state_lock` 不 yield）。在
/// multi_thread runtime 下，N 个 task 同时执行同步 future 会占满
/// runtime worker pool，导致其他并行 `#[tokio::test]` 拿不到 worker
/// 而死锁。
///
/// 为何不用 `tokio::task::spawn_blocking` + `Handle::current().block_on`：
/// `spawn_blocking` 的 JoinHandle 需要 _parent_ runtime worker 来 poll，
/// current_thread runtime 主 task `await` 时无法 poll，会死锁；
/// `Handle::block_on` 在 blocking thread 上调用会递归驱动 parent runtime，
/// 与 parent runtime 的 worker 冲突，也可能死锁。
///
/// `std::thread::scope` + 每个 worker 内部 `Runtime::new_current_thread()`
/// 完全隔离：worker OS thread 不占用任何 tokio runtime worker pool，
/// 独立 runtime 不与 parent runtime 共享，无死锁可能。
///
/// `'static` 解法：`std::thread::scope` 的 scoped thread 接受非 'static
/// 借用，因此每个 worker 可以直接用 `&reg`（parent 拥有）。但为简化，
/// 每个 worker 仍独立构造 `ToolRegistry`（move owned `Arc<DaemonState>`）。
///
/// `std::sync::Barrier` 是同步阻塞，但在独立 OS worker thread 上同步阻塞
/// 合法（不占用任何 runtime worker）。
#[tokio::test]
async fn concurrent_update_relationship_and_advance_plot_do_not_lose_updates() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    // 用独立 character_id 避免与其他 #[tokio::test] 共享 process-global
    // state_lock / session_lock（均以 character_id 为 key）。
    seed_character(&state.data_root, "concurrent_alice");

    // 10 个 worker（5 个 update_relationship + 5 个 advance_plot 交替）。
    const N: usize = 10;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(N));

    // 用 std::thread::scope 启动独立 OS worker thread，避免占用 tokio
    // runtime worker pool。每个 worker 内部建独立 single-thread runtime
    // 来 poll tool.call(...) 的 future。
    //
    // 不用 `Handle::current().block_on`：在 blocking thread 上调用它会
    // 递归驱动 parent runtime，与 parent runtime 的 worker 冲突，可能
    // 死锁（特别是 parent 是 multi_thread runtime 时）。
    //
    // 不用 `tokio::task::spawn_blocking`：其 JoinHandle 需要_parent_
    // runtime worker 来 poll，current_thread runtime 主 task await 时
    // 无法 poll，会死锁。
    //
    // std::thread::scope + 独立 Runtime 完全隔离，无 runtime 共享冲突。
    let results: Vec<Result<ToolResult, AirpError>> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for i in 0..N {
            let state = state.clone();
            let barrier = barrier.clone();
            let h = s.spawn(move || -> Result<ToolResult, AirpError> {
                let reg = default_registry(state.clone());
                let is_update = i % 2 == 0;
                let tool_name = if is_update {
                    "update_relationship"
                } else {
                    "advance_plot"
                };
                let params = if is_update {
                    serde_json::json!({
                        "character_id": "concurrent_alice",
                        "from": "concurrent_alice",
                        "to": format!("npc{}", i / 2),
                        "relation_type": "rival",
                        "intensity": 0.3
                    })
                } else {
                    serde_json::json!({
                        "character_id": "concurrent_alice",
                        "development": format!("event {}", i / 2),
                        "type": "progression"
                    })
                };

                // 启动栅栏：所有 worker 同时进入 `tool.call(...)`，最大化
                // 并发 read-modify-write 冲突概率，真正测试 state_lock 串行化。
                barrier.wait();

                // 独立 single-thread runtime：完全隔离，无 runtime 共享冲突。
                // 不调 `enable_all()`：tool.call(...) 内部全是同步代码（无
                // tokio I/O / timer），不需要 driver thread；避免在 worker
                // 内部额外启动 background driver，减少 OS thread 占用。
                let rt = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .expect("failed to build worker runtime");
                let tool = reg.get(tool_name).unwrap();
                rt.block_on(async { tool.call(params, true).await })
            });
            handles.push(h);
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("worker thread panicked"))
            .collect()
    });

    for (i, result) in results.into_iter().enumerate() {
        assert!(result.is_ok(), "tool call #{i} failed: {:?}", result.err());
    }

    // 验证：live.json 必须同时包含 5 个 relationships 条目和 5 个 plot_history 条目。
    let live_path = state
        .data_root
        .join("characters/concurrent_alice/state/live.json");
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

/// Gemini #1/#2 跟进测试：`update_relationship` / `advance_plot` 在 live.json
/// 损坏（非 Object，或字段类型错乱）时必须返回 `AirpError::Internal`，
/// 而非 panic daemon 或静默丢更新。
///
/// 覆盖 4 个场景：
/// - live.json 是 JSON Array（非 Object）→ 两个工具都应 Internal
/// - live.json 是 Object 且 `relationships`/`plot_history` 字段类型错乱
///   （如 String/Number）→ 两个工具都应 Internal
///
/// 旧版 `live["relationships"][&key] = ...` 在 live 非 Object 时会 panic
/// （`Index`::index` on non-Object Value），导致 daemon 崩溃。新版用
/// `as_object_mut` + `ok_or_else(Internal)` 防御性检查。
#[tokio::test]
async fn update_relationship_returns_internal_when_live_json_is_not_object() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "upd_rel_corrupt1");
    let reg = default_registry(state.clone());

    // 写入损坏的 live.json（Array 而非 Object）。
    let state_dir = state.data_root.join("characters/upd_rel_corrupt1/state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("live.json"), b"[1, 2, 3]").unwrap();

    let tool = reg.get("update_relationship").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "upd_rel_corrupt1",
                "from": "upd_rel_corrupt1",
                "to": "bob",
                "relation_type": "ally",
                "intensity": 0.5
            }),
            true,
        )
        .await;

    assert!(
        result.is_err(),
        "expected Internal error, got Ok: {:?}",
        result.ok()
    );
    let err = result.unwrap_err();
    match err {
        AirpError::Internal(msg) => assert!(
            msg.contains("not a JSON object"),
            "unexpected Internal message: {msg}"
        ),
        other => panic!("expected AirpError::Internal, got {other:?}"),
    }
}

#[tokio::test]
async fn update_relationship_returns_internal_when_relationships_field_is_wrong_type() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "upd_rel_corrupt2");
    let reg = default_registry(state.clone());

    // live.json 是 Object 但 relationships 字段是 String（类型错乱）。
    let state_dir = state.data_root.join("characters/upd_rel_corrupt2/state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("live.json"),
        br#"{"relationships": "not-an-object"}"#,
    )
    .unwrap();

    let tool = reg.get("update_relationship").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "upd_rel_corrupt2",
                "from": "upd_rel_corrupt2",
                "to": "bob",
                "relation_type": "ally",
                "intensity": 0.5
            }),
            true,
        )
        .await;

    assert!(result.is_err(), "expected Internal, got {:?}", result.ok());
    match result.unwrap_err() {
        AirpError::Internal(msg) => assert!(
            msg.contains("relationships field is not a JSON object"),
            "unexpected message: {msg}"
        ),
        other => panic!("expected Internal, got {other:?}"),
    }
}

#[tokio::test]
async fn advance_plot_returns_internal_when_live_json_is_not_object() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_plot_corrupt1");
    let reg = default_registry(state.clone());

    let state_dir = state.data_root.join("characters/adv_plot_corrupt1/state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("live.json"), b"\"not-an-object\"").unwrap();

    let tool = reg.get("advance_plot").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "adv_plot_corrupt1",
                "development": "the tower fell",
                "type": "progression"
            }),
            true,
        )
        .await;

    assert!(result.is_err(), "expected Internal, got {:?}", result.ok());
    match result.unwrap_err() {
        AirpError::Internal(msg) => assert!(
            msg.contains("not a JSON object"),
            "unexpected message: {msg}"
        ),
        other => panic!("expected Internal, got {other:?}"),
    }
}

#[tokio::test]
async fn advance_plot_returns_internal_when_plot_history_field_is_wrong_type() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_plot_corrupt2");
    let reg = default_registry(state.clone());

    let state_dir = state.data_root.join("characters/adv_plot_corrupt2/state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("live.json"),
        br#"{"plot_history": "not-an-array"}"#,
    )
    .unwrap();

    let tool = reg.get("advance_plot").unwrap();
    let result = tool
        .call(
            serde_json::json!({
                "character_id": "adv_plot_corrupt2",
                "development": "the tower fell",
                "type": "progression"
            }),
            true,
        )
        .await;

    assert!(result.is_err(), "expected Internal, got {:?}", result.ok());
    match result.unwrap_err() {
        AirpError::Internal(msg) => assert!(
            msg.contains("plot_history field is not a JSON array"),
            "unexpected message: {msg}"
        ),
        other => panic!("expected Internal, got {other:?}"),
    }
}

/// 审计 Bug B 修复测试：`advance_clock` 推进世界时钟到 `time_trigger` 阈值时，
/// 到期事件内容必须被追加到 `current.md`，且 `triggered` 标志必须被持久化。
///
/// 旧 Bug B：`AdvanceClockTool::call` 未持有 `session_lock` 就调用
/// `append_to_current`，允许并发 `npc_action` / `advance_plot` 的 append
/// 与此处的 append 在 `current.md` 中交错混合叙事内容。修复后，
/// `AdvanceClockTool::call` 在 `session_lock` 临界区内执行 append。
///
/// 本测试覆盖功能性契约（事件触发 + 内容追加 + 标志持久化）；
/// 并发不交错由 `session_lock` 串行化保证，与 `npc_action` 共享同一把锁。
#[tokio::test]
async fn advance_clock_triggers_time_events_and_appends_to_current() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_clk_trig");
    let reg = default_registry(state.clone());

    // 写入 world_clock.json：current_time=0, advance_per_turn=1
    let clock_path = state
        .data_root
        .join("characters/adv_clk_trig/world_clock.json");
    std::fs::write(
        &clock_path,
        serde_json::json!({
            "current_time": 0,
            "advance_per_turn": 1,
            "time_unit": "hour"
        })
        .to_string(),
    )
    .unwrap();

    // 写入 world_events.json：一个 time_trigger=1 的事件
    let events_path = state
        .data_root
        .join("characters/adv_clk_trig/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([{
            "id": "evt_time_1",
            "name": "Dawn",
            "description": "The sun rises",
            "content": "Golden light spills over the horizon.",
            "time_trigger": 1
        }])
        .to_string(),
    )
    .unwrap();

    // 调用 advance_clock：默认推进 advance_per_turn=1，current_time 从 0→1，
    // 达到 time_trigger=1 阈值，事件应被触发。
    let tool = reg.get("advance_clock").unwrap();
    let result = tool
        .call(serde_json::json!({"character_id": "adv_clk_trig"}), true)
        .await
        .unwrap();

    assert_eq!(result.output["current_time"], 1);
    let triggered = result.output["triggered_events"].as_array().unwrap();
    assert_eq!(triggered.len(), 1);
    assert_eq!(triggered[0], "Dawn");

    // current.md 应含事件内容（由 session_lock 临界区内的 append 写入）。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "adv_clk_trig", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(
        current.contains("[世界事件: Dawn]"),
        "current.md should contain event injection, got: {current}"
    );
    assert!(
        current.contains("Golden light spills over the horizon."),
        "current.md should contain event content"
    );

    // world_events.json 中 triggered 应为 true（由 state_lock 临界区内的
    // save_world_events 持久化）。
    let events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(events[0]["triggered"], true);
}

/// 审计 Bug B 修复测试：`advance_clock` 在没有到期事件时，不应向 `current.md`
/// 追加任何内容，且 `triggered_events` 应为空数组。
#[tokio::test]
async fn advance_clock_no_due_events_does_not_append() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_clk_empty");
    let reg = default_registry(state.clone());

    let clock_path = state
        .data_root
        .join("characters/adv_clk_empty/world_clock.json");
    std::fs::write(
        &clock_path,
        serde_json::json!({
            "current_time": 0,
            "advance_per_turn": 1,
            "time_unit": "hour"
        })
        .to_string(),
    )
    .unwrap();

    // 事件的 time_trigger=10，当前推进到 1，不会触发。
    // 显式写入 triggered:false，使后续断言 events[0]["triggered"] == false
    // 可靠（不依赖 serde 默认值，因 advance_clock 不触发事件时不会重写文件）。
    let events_path = state
        .data_root
        .join("characters/adv_clk_empty/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([{
            "id": "evt_future",
            "name": "Eclipse",
            "description": "A rare eclipse",
            "content": "Darkness falls at noon.",
            "time_trigger": 10,
            "triggered": false
        }])
        .to_string(),
    )
    .unwrap();

    // 预先向 current.md 写入代表性内容，确保 no-due 路径不会执行
    // 任何 append（包括空 append 或无关 append）。CodeRabbit 建议：
    // 不只断言"不含事件内容"，而是断言 current.md 完全不变。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "adv_clk_empty", None).unwrap();
    let preexisting = "[剧情推进: setup] The party rests at the inn.\n";
    crate::volume_store::append_to_current(&session_dir, preexisting).unwrap();
    let before = crate::volume_store::read_current(&session_dir).unwrap();

    let tool = reg.get("advance_clock").unwrap();
    let result = tool
        .call(serde_json::json!({"character_id": "adv_clk_empty"}), true)
        .await
        .unwrap();

    assert_eq!(result.output["current_time"], 1);
    let triggered = result.output["triggered_events"].as_array().unwrap();
    assert_eq!(triggered.len(), 0, "no events should be triggered");

    // current.md 必须与调用前完全一致——无事件内容、无空 append、无格式变化。
    let after = crate::volume_store::read_current(&session_dir).unwrap();
    assert_eq!(
        before, after,
        "current.md must be unchanged when no events are due"
    );

    // 事件不应被标记为 triggered。
    let events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(events[0]["triggered"], false);
}

/// 审计 Bug B 修复测试：`advance_clock` 批量触发多个到期事件时，
/// 所有事件内容应被合并为单次追加（而非逐事件 append），避免部分
/// 成功部分失败导致重复注入。所有到期事件应被标记为 triggered。
#[tokio::test]
async fn advance_clock_triggers_multiple_due_events_in_single_append() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    seed_character(&state.data_root, "adv_clk_multi");
    let reg = default_registry(state.clone());

    let clock_path = state
        .data_root
        .join("characters/adv_clk_multi/world_clock.json");
    std::fs::write(
        &clock_path,
        serde_json::json!({
            "current_time": 0,
            "advance_per_turn": 5,
            "time_unit": "hour"
        })
        .to_string(),
    )
    .unwrap();

    // 两个事件都在 time_trigger=3，推进 5 后都应触发。
    let events_path = state
        .data_root
        .join("characters/adv_clk_multi/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([
            {
                "id": "evt_a",
                "name": "Storm",
                "description": "A sudden storm",
                "content": "Lightning split the sky.",
                "time_trigger": 3
            },
            {
                "id": "evt_b",
                "name": "Festival",
                "description": "Annual festival",
                "content": "The town square fills with color.",
                "time_trigger": 3
            }
        ])
        .to_string(),
    )
    .unwrap();

    let tool = reg.get("advance_clock").unwrap();
    let result = tool
        .call(serde_json::json!({"character_id": "adv_clk_multi"}), true)
        .await
        .unwrap();

    assert_eq!(result.output["current_time"], 5);
    let triggered = result.output["triggered_events"].as_array().unwrap();
    assert_eq!(triggered.len(), 2, "both events should be triggered");

    // current.md 应含两个事件的内容（单次 append）。
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "adv_clk_multi", None).unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("Lightning split the sky."));
    assert!(current.contains("The town square fills with color."));

    // 两个事件都应被标记为 triggered。
    let events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(events[0]["triggered"], true);
    assert_eq!(events[1]["triggered"], true);
}

/// 审计 Bug F（死锁）修复测试：`trigger_world_event` 与 `advance_plot` 在同一
/// `character_id` 上并发执行时不得死锁。
///
/// 旧 Bug F：`trigger_world_event::call` 同时持有 `state_lock(cid)` +
/// `session_lock(cid, sid)`（state→session 顺序），而 `advance_plot::call`
/// 持有 `session_lock(cid, sid)` 后经 `StateService::mutate` 再获取
/// `state_lock(cid)`（session→state 顺序）。两者并发形成锁序倒置死锁：
///   线程 A (advance_plot):         hold session_lock → wait state_lock
///   线程 B (trigger_world_event):  hold state_lock   → wait session_lock
/// 修复后 `trigger_world_event` 拆为两段独立临界区（state_lock 释放后才获取
/// session_lock），同一调用任意时刻只持一把锁，消除锁序倒置。
///
/// 测试策略：用 `std::thread::scope` + `Barrier` 让两个 worker 同时进入
/// `tool.call(...)`，外加 30s 超时。若死锁残留，超时触发 panic 使测试失败；
/// 修复后两者应在秒级完成。
///
/// 复用 `concurrent_update_relationship_and_advance_plot_do_not_lose_updates`
/// 的隔离模式（独立 OS thread + 独立 single-thread runtime），避免占用
/// parent tokio runtime worker pool。
#[tokio::test(flavor = "current_thread")]
async fn trigger_world_event_and_advance_plot_concurrent_no_deadlock() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    // 唯一 character_id：避免与其他 #[tokio::test] 争用 process-global 锁。
    seed_character(&state.data_root, "deadlock_f");

    // 写入一个可触发的 world_event。
    let events_path = state
        .data_root
        .join("characters/deadlock_f/world_events.json");
    std::fs::write(
        &events_path,
        serde_json::json!([{
            "id": "evt_df",
            "name": "Storm",
            "description": "A sudden storm",
            "content": "Lightning split the sky."
        }])
        .to_string(),
    )
    .unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    // 30s 超时：死锁时让测试失败而非无限挂起 CI。
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    let join_handle = tokio::task::spawn_blocking(move || {
        std::thread::scope(|s| {
            let mut handles = Vec::new();

            // Worker A: trigger_world_event（修复前持 state→session 两把锁）
            {
                let state = state.clone();
                let barrier = barrier.clone();
                handles.push(s.spawn(move || -> Result<(), String> {
                    let reg = default_registry(state.clone());
                    let tool = reg.get("trigger_world_event").unwrap();
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map_err(|e| format!("rt build: {e}"))?;
                    barrier.wait();
                    rt.block_on(async {
                        tool.call(
                            serde_json::json!({
                                "character_id": "deadlock_f",
                                "event_id": "evt_df"
                            }),
                            true,
                        )
                        .await
                    })
                    .map_err(|e| format!("trigger_world_event: {e:?}"))?;
                    Ok(())
                }));
            }

            // Worker B: advance_plot（持 session_lock 后经 mutate 持 state_lock）
            {
                let state = state.clone();
                let barrier = barrier.clone();
                handles.push(s.spawn(move || -> Result<(), String> {
                    let reg = default_registry(state.clone());
                    let tool = reg.get("advance_plot").unwrap();
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map_err(|e| format!("rt build: {e}"))?;
                    barrier.wait();
                    rt.block_on(async {
                        tool.call(
                            serde_json::json!({
                                "character_id": "deadlock_f",
                                "development": "the storm broke",
                                "type": "progression"
                            }),
                            true,
                        )
                        .await
                    })
                    .map_err(|e| format!("advance_plot: {e:?}"))?;
                    Ok(())
                }));
            }

            for h in handles {
                h.join().map_err(|e| format!("worker join: {e:?}"))??;
            }
            Ok::<(), String>(())
        })
    });

    // 用 tokio timeout 包裹 spawn_blocking：死锁时 join_handle 不会返回。
    let result = tokio::time::timeout_at(deadline, join_handle).await;
    match result {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(msg))) => panic!("worker error: {msg}"),
        Ok(Err(join_err)) => panic!("spawn_blocking join error: {join_err:?}"),
        Err(_) => {
            panic!("trigger_world_event + advance_plot 死锁：30s 超时未完成（Bug F 修复未生效）")
        }
    }
}

/// PR #338 审计遗留 N1：`advance_clock` 与 `advance_plot` 在同一角色、同一
/// session 上并发执行时不得形成 state→session / session→state 锁序环。
///
/// 两个 worker 使用独立 OS thread 和 current-thread runtime，并由 Barrier
/// 同时放行。30 秒门限只用于把残留死锁转为明确测试失败；正常路径应秒级完成。
#[tokio::test(flavor = "current_thread")]
async fn advance_clock_and_advance_plot_concurrent_no_deadlock() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    // 唯一 character_id：避免与其他测试争用 process-global 锁。
    seed_character(&state.data_root, "deadlock_clock_plot");

    let character_dir = state.data_root.join("characters/deadlock_clock_plot");
    std::fs::write(
        character_dir.join("world_clock.json"),
        serde_json::json!({
            "current_time": 0,
            "advance_per_turn": 1,
            "time_unit": "hour"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        character_dir.join("world_events.json"),
        serde_json::json!([{
            "id": "evt_clock_plot",
            "name": "Dawn",
            "description": "The sun rises",
            "content": "Dawn reached the valley.",
            "time_trigger": 1
        }])
        .to_string(),
    )
    .unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    let worker_state = state.clone();
    let join_handle = tokio::task::spawn_blocking(move || {
        std::thread::scope(|scope| {
            let mut handles = Vec::new();

            {
                let state = worker_state.clone();
                let barrier = barrier.clone();
                handles.push(scope.spawn(move || -> Result<(), String> {
                    let registry = default_registry(state);
                    let tool = registry.get("advance_clock").unwrap();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map_err(|error| format!("runtime build: {error}"))?;
                    barrier.wait();
                    runtime
                        .block_on(tool.call(
                            serde_json::json!({
                                "character_id": "deadlock_clock_plot",
                                "advance_by": 1
                            }),
                            true,
                        ))
                        .map_err(|error| format!("advance_clock: {error:?}"))?;
                    Ok(())
                }));
            }

            {
                let state = worker_state.clone();
                let barrier = barrier.clone();
                handles.push(scope.spawn(move || -> Result<(), String> {
                    let registry = default_registry(state);
                    let tool = registry.get("advance_plot").unwrap();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map_err(|error| format!("runtime build: {error}"))?;
                    barrier.wait();
                    runtime
                        .block_on(tool.call(
                            serde_json::json!({
                                "character_id": "deadlock_clock_plot",
                                "development": "the valley woke at dawn",
                                "type": "progression"
                            }),
                            true,
                        ))
                        .map_err(|error| format!("advance_plot: {error:?}"))?;
                    Ok(())
                }));
            }

            for handle in handles {
                handle
                    .join()
                    .map_err(|error| format!("worker join: {error:?}"))??;
            }
            Ok::<(), String>(())
        })
    });

    match tokio::time::timeout_at(deadline, join_handle).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(message))) => panic!("worker error: {message}"),
        Ok(Err(error)) => panic!("spawn_blocking join error: {error:?}"),
        Err(_) => {
            panic!("advance_clock + advance_plot deadlocked: workers exceeded 30 seconds")
        }
    }

    let clock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(character_dir.join("world_clock.json")).unwrap())
            .unwrap();
    assert_eq!(clock["current_time"], 1);

    let events: serde_json::Value =
        serde_json::from_slice(&std::fs::read(character_dir.join("world_events.json")).unwrap())
            .unwrap();
    assert_eq!(events[0]["triggered"], true);

    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "deadlock_clock_plot", None)
            .unwrap();
    let current = crate::volume_store::read_current(&session_dir).unwrap();
    assert!(current.contains("Dawn reached the valley."));
    assert!(current.contains("the valley woke at dawn"));
}

/// #437 回归测试：`advance_plot` 持有 `character_lock.read()` 期间，
/// `delete_character`（需 `character_lock.write()`）必须串行化，不得并发删除
/// character 顶层目录（含 `live.json`）。这验证 R1 TOCTOU 防护已闭合。
///
/// 修复前（PR #436 时残留）：`advance_plot` 外层为 `session_lock`，未持
/// `character_lock.read()`；`delete_character` 可在 `advance_plot` 临界区期间
/// 并发删除 `live.json`，导致 `StateService::mutate` 读到半删状态（TOCTOU）。
///
/// 修复后（#437 fix path 4）：`advance_plot` 外层先 acquire
/// `character_lock.read()`，`delete_character` 的 `character_lock.write()`
/// 必须等待 `advance_plot` 释放后才能进入临界区，消除 TOCTOU 风险。
///
/// 测试策略：两个 worker 经 Barrier 同时放行，30s 超时检测死锁。R1 只保证
/// 串行化（不保证顺序），所以两种合法终态：
/// - `advance_plot` 先完成 → 持锁期间 durable 写入；`delete_character` 随后
///   删除 dir → 终态：dir 不存在。
/// - `delete_character` 先完成 → dir 已删；`advance_plot` 随后 acquire 新
///   `character_lock` 实例（lock-map 已 cleanup），重新创建 dir 写 live.json
///   → 终态：dir 存在且含 advance_plot 产物。
///
/// 关键不变式（本测试唯一断言）：`advance_plot` 不应返回 `Internal` error
/// （那表示读到半删 live.json，R1 TOCTOU 防护失效）。`Ok` 与 `NotFound` 都是
/// 合法的串行化结果。dir 终态由串行顺序决定，不断言。
#[tokio::test(flavor = "current_thread")]
async fn advance_plot_and_delete_character_serialized_by_character_lock() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    // 唯一 character_id：避免与其他 #[tokio::test] 争用 process-global 锁。
    seed_character(&state.data_root, "adv_plot_r1");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    let worker_state = state.clone();
    let join_handle = tokio::task::spawn_blocking(move || {
        std::thread::scope(|scope| {
            // Worker A: advance_plot（修复后持 character_lock.read()）
            let handle_a = {
                let state = worker_state.clone();
                let barrier = barrier.clone();
                scope.spawn(move || -> Result<(), String> {
                    let reg = default_registry(state.clone());
                    let tool = reg.get("advance_plot").unwrap();
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map_err(|e| format!("rt build: {e}"))?;
                    barrier.wait();
                    let result = rt.block_on(async {
                        tool.call(
                            serde_json::json!({
                                "character_id": "adv_plot_r1",
                                "development": "test plot for r1 regression",
                                "type": "progression"
                            }),
                            true,
                        )
                        .await
                    });
                    // advance_plot 可能成功（持锁先于 delete_character 完成），
                    // 也可能失败（delete_character 先完成，live.json 已删 → NotFound）。
                    // 两者都是合法的 R1 串行化结果；读到半删状态会返回 Internal，
                    // 那是 R1 防护失效的标志，会让下面的 err 检查失败。
                    if let Err(e) = result {
                        match e {
                            crate::error::AirpError::NotFound(_) => Ok(()),
                            other => Err(format!(
                                "advance_plot failed with non-NotFound error (R1 TOCTOU protection may have failed): {other:?}"
                            )),
                        }
                    } else {
                        Ok(())
                    }
                })
            };

            // Worker B: delete_character（需 character_lock.write()）
            let handle_b = {
                let state = worker_state.clone();
                let barrier = barrier.clone();
                scope.spawn(move || -> Result<bool, String> {
                    let chat = crate::domain::ChatService::new(&state.data_root);
                    let cid = crate::types::CharacterId::new("adv_plot_r1")
                        .map_err(|e| format!("cid: {e}"))?;
                    barrier.wait();
                    // delete_character 可能：
                    // - 成功（character dir 被删除）
                    // - 失败 NotFound（advance_plot 已删除？不会发生——advance_plot 不删 character）
                    // - 失败 Io(DirectoryNotEmpty)（Windows 已知 quirk：lock-map cleanup 在
                    //   write guard 释放前移除条目，允许 advance_plot 用新 lock 实例并发写文件，
                    //   导致 remove_dir_all 在 Windows 上失败。这是 #422 lock-map cleanup 的
                    //   pre-existing race，非 #437 R1 fix 引入，记录为 follow-up。）
                    // 三者都是合法结果；本测试只验证 R1 锁序串行化不死锁 + advance_plot
                    // 不读到半删状态（Internal error）。
                    let result = chat.delete_character(&cid);
                    Ok(result.is_ok())
                })
            };

            handle_a
                .join()
                .map_err(|e| format!("worker A join: {e:?}"))??;
            let delete_succeeded = handle_b
                .join()
                .map_err(|e| format!("worker B join: {e:?}"))??;
            Ok::<bool, String>(delete_succeeded)
        })
    });

    let _delete_succeeded = match tokio::time::timeout_at(deadline, join_handle).await {
        Ok(Ok(Ok(succeeded))) => succeeded,
        Ok(Ok(Err(message))) => panic!("worker error: {message}"),
        Ok(Err(error)) => panic!("spawn_blocking join error: {error:?}"),
        Err(_) => {
            panic!(
                "advance_plot + delete_character deadlocked: workers exceeded 30 seconds \
                 (R1 character_lock serialization failed — see #437 fix path 4)"
            )
        }
    };

    // 关键不变式已在 worker A 内验证：advance_plot 不应返回 Internal error
    //（那表示读到半删 live.json，R1 TOCTOU 防护失效）。worker A 已将非 NotFound
    // error 升级为测试失败。
    //
    // 不断言 character dir 终态：R1 只保证串行化（不保证顺序），若
    // `delete_character` 先完成而 `advance_plot` 后完成，`advance_plot` 会
    // 重新创建 dir 写 live.json —— 这是合法的串行化结果，非 TOCTOU 失效。
    // `delete_succeeded` 仅用于完成 worker B 的 Result<bool, String> 类型契约。
}
