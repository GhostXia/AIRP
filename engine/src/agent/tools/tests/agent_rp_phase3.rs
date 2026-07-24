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
// 不覆盖：world_events.json 的 revision 合同（审计遗留项，本 PR 未接入）。

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
