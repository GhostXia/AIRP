// Session-family tests for `agent::tools`.
//
// 从 `tools.rs` 原 inline `mod tests` 原样迁移，不改断言逻辑。
// 测试通过 `default_registry` 端到端验证 5 个 session 工具的
// roundtrip / 隔离 / 边界 / 错误路径。

use super::*;
use crate::adapter::{ChatMessage, MessageRole};
use crate::chat_store::{ChatLog, MAX_MESSAGES};
use tempfile::tempdir;

#[tokio::test]
async fn session_tools_roundtrip_append_recent_rollback() {
    // 端到端：start → list → append×2 → recent → rollback(dry-run) → rollback(真)
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    // start_session
    let start = reg.get("start_session").unwrap();
    let r = start
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    assert!(!r.dry_run);
    assert!(r.output["session_id"].is_string());
    let session_id = r.output["session_id"].as_str().unwrap().to_string();

    // list_sessions → 至少 1
    let list = reg.get("list_sessions").unwrap();
    let r = list
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    let arr = r.output.as_array().unwrap();
    assert!(
        !arr.is_empty(),
        "list_sessions should find the started session"
    );

    // append_message ×2 (user + assistant)
    let append = reg.get("append_message").unwrap();
    for (role, content) in [("user", "hello"), ("assistant", "hi there")] {
        let r = append
            .call(
                serde_json::json!({
                    "character_id": "alice",
                    "session_id": session_id.clone(),
                    "role": role,
                    "content": content,
                }),
                false,
            )
            .await
            .unwrap();
        assert!(r.output["total"].as_u64().unwrap() >= 1);
    }

    // get_recent_context n=10 → 2 条
    let recent = reg.get("get_recent_context").unwrap();
    let r = recent
        .call(
            serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "n": 10}),
            false,
        )
        .await
        .unwrap();
    let msgs = r.output["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[1]["content"], "hi there");

    // rollback index=0 dry-run → dropped=1, dry_run=true
    let rb = reg.get("rollback_messages").unwrap();
    let r = rb
        .call(
            serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "index": 0}),
            false,
        )
        .await
        .unwrap();
    assert!(r.dry_run);
    assert_eq!(r.output["dropped"].as_u64().unwrap(), 1);

    // rollback index=0 confirm=true → 真回滚，剩 1 条
    let r = rb
        .call(
            serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "index": 0}),
            true,
        )
        .await
        .unwrap();
    assert!(!r.dry_run);
    let r = recent
        .call(
            serde_json::json!({"character_id": "alice", "session_id": session_id.clone(), "n": 10}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(r.output["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn session_history_isolated_from_character_history() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state);

    let start = reg.get("start_session").unwrap();
    let session = start
        .call(serde_json::json!({"character_id": "scope"}), false)
        .await
        .unwrap()
        .output["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let append = reg.get("append_message").unwrap();
    append
        .call(
            serde_json::json!({"character_id": "scope", "role": "user", "content": "global"}),
            false,
        )
        .await
        .unwrap();
    append
        .call(
            serde_json::json!({
                "character_id": "scope",
                "session_id": session.clone(),
                "role": "user",
                "content": "session",
            }),
            false,
        )
        .await
        .unwrap();

    let recent = reg.get("get_recent_context").unwrap();
    let global = recent
        .call(serde_json::json!({"character_id": "scope", "n": 10}), false)
        .await
        .unwrap();
    assert_eq!(global.output["messages"][0]["content"], "global");

    let scoped = recent
        .call(
            serde_json::json!({"character_id": "scope", "session_id": session.clone(), "n": 10}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(scoped.output["messages"][0]["content"], "session");
}

#[tokio::test]
async fn append_reports_full_history_index_after_context_threshold() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let mut log = ChatLog::load_or_create(&state.data_root, "overflow").unwrap();
    for i in 0..MAX_MESSAGES {
        log.append(
            &state.data_root,
            ChatMessage {
                role: MessageRole::User,
                content: format!("seed-{i}"),
            },
        )
        .unwrap();
    }

    let reg = default_registry(state);
    let append = reg.get("append_message").unwrap();
    let r = append
        .call(
            serde_json::json!({
                "character_id": "overflow",
                "role": "assistant",
                "content": "after-cap",
            }),
            false,
        )
        .await
        .unwrap();

    assert_eq!(r.output["index"], MAX_MESSAGES);
    assert_eq!(r.output["total"], MAX_MESSAGES + 1);
    assert_eq!(r.output["truncated"], false);
    assert_eq!(r.output["truncated_count"], 0);
}

#[tokio::test]
async fn recent_context_rejects_over_cap() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state);
    let recent = reg.get("get_recent_context").unwrap();
    let err = recent
        .call(
            serde_json::json!({"character_id": "cap", "n": MAX_RECENT_CONTEXT + 1}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));
}

#[tokio::test]
async fn rollback_rejects_out_of_range_index() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state);
    let append = reg.get("append_message").unwrap();
    append
        .call(
            serde_json::json!({"character_id": "bob", "role": "user", "content": "x"}),
            false,
        )
        .await
        .unwrap();
    let rb = reg.get("rollback_messages").unwrap();
    let err = rb
        .call(
            serde_json::json!({"character_id": "bob", "index": 99}),
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));
}

#[tokio::test]
async fn append_rejects_invalid_role() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state);
    let append = reg.get("append_message").unwrap();
    let err = append
        .call(
            serde_json::json!({"character_id": "cat", "role": "narrator", "content": "x"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));
}
