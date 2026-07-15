// Preset family tests for `agent::tools`（#115 P1 第二阶段）。
//
// 端到端验证 get_preset + update_preset 工具的 dry-run / confirm / roundtrip /
// rejection 行为，对齐 state_lorebook 的测试风格。

use super::*;
use tempfile::tempdir;

/// update_preset dry-run 不写盘，confirm=true 写盘后 get_preset 能读到 canonical prompts。
#[tokio::test]
async fn preset_tools_roundtrip_with_confirmation() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let preset_json = "\u{feff}{\n  \"prompts\": [\n    {\"identifier\": \"main\", \"name\": \"Main\", \"role\": \"system\", \"content\": \"hi\", \"enabled\": true}\n  ],\n  \"prompt_order\": [{\"character_id\": \"main\", \"order\": [\"main\"]}],\n  \"temperature\": 0.8,\n  \"note\": \"first\",\n  \"note\": \"second\"\n}";

    // dry-run：返回 import_report，不写盘
    let update = reg.get("update_preset").unwrap();
    let preview = update
        .call(
            serde_json::json!({
                "preset_id": "test-preset",
                "preset_json": preset_json,
            }),
            false,
        )
        .await
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.output["requires"], "confirm=true");
    assert_eq!(preview.output["converted"], 1);
    assert!(
        preview.output["import_report"]["advisory_preserved"]
            .as_u64()
            .unwrap()
            >= 1
    );
    // dry-run 不写盘
    assert!(!state
        .data_root
        .join("presets")
        .join("test-preset")
        .join("preset.json")
        .exists());
    assert!(!state
        .data_root
        .join("presets")
        .join("test-preset")
        .join("current")
        .exists());

    // confirm=true：写盘 canonical + raw
    let written = update
        .call(
            serde_json::json!({
                "preset_id": "test-preset",
                "preset_json": preset_json,
            }),
            true,
        )
        .await
        .unwrap();
    assert!(!written.dry_run);
    assert_eq!(written.output["updated"], "test-preset");

    // canonical preset.json + raw.json 都落盘
    let preset_dir = state.data_root.join("presets").join("test-preset");
    assert!(preset_dir.join("current").exists());
    assert!(crate::data_dir::preset_json_path(&state.data_root, "test-preset").exists());
    let raw_path = crate::data_dir::preset_json_path(&state.data_root, "test-preset")
        .with_file_name("raw.json");
    assert!(raw_path.exists());
    assert_eq!(
        std::fs::read_to_string(raw_path).unwrap(),
        crate::data_dir::strip_utf8_bom(preset_json)
    );

    // get_preset 能读到 canonical prompts
    let get = reg.get("get_preset").unwrap();
    let current = get
        .call(serde_json::json!({"preset_id": "test-preset"}), false)
        .await
        .unwrap();
    assert_eq!(current.output["prompts_count"], 1);
    assert_eq!(current.output["prompts"][0]["identifier"], "main");
}

/// update_preset 对非 JSON 源返回 BadRequest。
#[tokio::test]
async fn update_preset_rejects_invalid_json() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state);

    let update = reg.get("update_preset").unwrap();
    let err = update
        .call(
            serde_json::json!({
                "preset_id": "bad",
                "preset_json": "not json at all",
            }),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));
}

/// update_preset 对顶层非对象源返回 BadRequest（replacement_error 守门）。
#[tokio::test]
async fn update_preset_rejects_non_object_source() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let update = reg.get("update_preset").unwrap();
    let err = update
        .call(
            serde_json::json!({
                "preset_id": "bad",
                "preset_json": "[\"not\", \"an\", \"object\"]",
            }),
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));
    // 不写盘
    assert!(!state.data_root.join("presets").join("bad").exists());
}

/// get_preset 对不存在的 preset 返回 NotFound。
#[tokio::test]
async fn get_preset_returns_not_found_for_missing() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state);

    let get = reg.get("get_preset").unwrap();
    let err = get
        .call(serde_json::json!({"preset_id": "nonexistent"}), false)
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::NotFound(_)));
}

/// update_preset 允许覆盖已有 preset（destructive update 语义）。
#[tokio::test]
async fn update_preset_allows_overwrite() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let update = reg.get("update_preset").unwrap();

    // 第一次写入
    let first = serde_json::json!({
        "prompts": [{"identifier": "v1", "name": "V1", "role": "system", "content": "a", "enabled": true}]
    })
    .to_string();
    update
        .call(
            serde_json::json!({"preset_id": "overwrite", "preset_json": first}),
            true,
        )
        .await
        .unwrap();

    // 第二次覆盖写入（不同内容）
    let second = serde_json::json!({
        "prompts": [{"identifier": "v2", "name": "V2", "role": "system", "content": "b", "enabled": true}]
    })
    .to_string();
    let result = update
        .call(
            serde_json::json!({"preset_id": "overwrite", "preset_json": second}),
            true,
        )
        .await
        .unwrap();
    assert!(!result.dry_run);

    // get_preset 读到的是第二次的内容
    let get = reg.get("get_preset").unwrap();
    let current = get
        .call(serde_json::json!({"preset_id": "overwrite"}), false)
        .await
        .unwrap();
    assert_eq!(current.output["prompts"][0]["identifier"], "v2");
    let raw_path =
        crate::data_dir::preset_json_path(&state.data_root, "overwrite").with_file_name("raw.json");
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(raw_path).unwrap()).unwrap();
    assert_eq!(raw["prompts"][0]["identifier"], "v2");
}
