// Character-family tests for `agent::tools`.
//
// 从 `tools.rs` 原 inline `mod tests` 原样迁移，不改断言逻辑。
// 包含 list/get/delete 端到端、legacy card.json 兼容、invalid JSON 拒绝、
// 以及 issue #22 的角色级写锁互斥验证。

use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn character_tools_list_get_delete() {
    // 端到端：list(空) → 写 fixture card → list(1) → get → delete(dry-run) → delete(真) → list(空)
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    // 非法目录名不应出现在 list_characters 中，否则 list/get/delete 契约不对称。
    std::fs::create_dir_all(state.data_root.join("characters").join(".bad")).unwrap();

    // list 初始空
    let list = reg.get("list_characters").unwrap();
    let r = list.call(serde_json::json!({}), false).await.unwrap();
    assert_eq!(r.output.as_array().unwrap().len(), 0);

    // 写 fixture 角色卡
    let char_dir = state.data_root.join("characters").join("alice");
    std::fs::create_dir_all(char_dir.join("card")).unwrap();
    std::fs::write(
        char_dir.join("card").join("card.json"),
        r#"{"name":"Alice","description":"test char"}"#,
    )
    .unwrap();

    // list → 1
    let r = list.call(serde_json::json!({}), false).await.unwrap();
    assert_eq!(r.output.as_array().unwrap().len(), 1);
    assert_eq!(r.output[0], "alice");

    // get → card object
    let get = reg.get("get_character").unwrap();
    let r = get
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    assert_eq!(r.output["card"]["name"], "Alice");

    // get 不存在角色 → NotFound
    let err = get
        .call(serde_json::json!({"character_id": "ghost"}), false)
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::NotFound(_)));

    // delete dry-run → preview, dry_run=true
    let del = reg.get("delete_character").unwrap();
    let r = del
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    assert!(r.dry_run);
    assert_eq!(r.output["action"], "delete_character");
    assert_eq!(r.output["requires"], "confirm=true");
    assert!(r.output["will_delete"].is_array());
    assert!(
        char_dir.exists(),
        "dry-run must not delete the character dir"
    );

    // delete dry-run 对不存在角色也应报 NotFound，避免误导 agent 决策。
    let err = del
        .call(serde_json::json!({"character_id": "ghost"}), false)
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::NotFound(_)));

    // delete confirm=true → 真删
    let r = del
        .call(serde_json::json!({"character_id": "alice"}), true)
        .await
        .unwrap();
    assert!(!r.dry_run);
    assert_eq!(r.output["deleted"], "alice");

    // list → 0
    let r = list.call(serde_json::json!({}), false).await.unwrap();
    assert_eq!(r.output.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_character_reads_legacy_card_json_path() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let char_dir = state.data_root.join("characters").join("legacy");
    std::fs::create_dir_all(&char_dir).unwrap();
    std::fs::write(
        char_dir.join("card.json"),
        r#"{"name":"Legacy","description":"old layout"}"#,
    )
    .unwrap();

    let get = reg.get("get_character").unwrap();
    let r = get
        .call(serde_json::json!({"character_id": "legacy"}), false)
        .await
        .unwrap();

    assert_eq!(r.output["card"]["name"], "Legacy");
}

#[tokio::test]
async fn get_character_rejects_invalid_card_json() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let char_dir = state
        .data_root
        .join("characters")
        .join("broken")
        .join("card");
    std::fs::create_dir_all(&char_dir).unwrap();
    std::fs::write(char_dir.join("card.json"), "not json").unwrap();

    let get = reg.get("get_character").unwrap();
    let err = get
        .call(serde_json::json!({"character_id": "broken"}), false)
        .await
        .unwrap_err();

    assert!(matches!(err, AirpError::BadRequest(_)));
}

#[test]
fn delete_write_lock_excludes_session_writes() {
    // issue #22：delete_character 的角色级写锁必须与 append/rollback 的读锁
    // 互斥。持一把 read guard 时 delete 侧 write() 必须阻塞，直到 read 释放才
    // 推进——证明二者走同一把角色锁，不再各锁各的（旧实现 delete 与命名会话
    // 写属不同 Mutex entry，互不排斥）。用独立 key 避免污染并行测试的角色锁。
    use std::sync::atomic::{AtomicBool, Ordering};
    let key = "issue22-delete-lock-probe";
    let reader = crate::domain::character_lock(key);
    let read_guard = reader.read().unwrap();

    let writer = crate::domain::character_lock(key);
    let acquired = Arc::new(AtomicBool::new(false));
    let acquired2 = acquired.clone();
    let handle = std::thread::spawn(move || {
        let _w = writer.write().unwrap();
        acquired2.store(true, Ordering::SeqCst);
    });

    // read guard 仍持有：write 不可能拿到。
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !acquired.load(Ordering::SeqCst),
        "write lock must not be acquired while a read guard is held"
    );

    // 释放 read → write 应推进。
    drop(read_guard);
    handle.join().unwrap();
    assert!(acquired.load(Ordering::SeqCst));
}
