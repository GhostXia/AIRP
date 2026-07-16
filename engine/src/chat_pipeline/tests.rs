// Tests for chat_pipeline — declared as `#[cfg(test)] mod tests;` in chat_pipeline.rs.
// This file is the `chat_pipeline::tests` child module.
// `use super::*;` here imports ALL accessible items from `chat_pipeline` (including
// private ones, since child modules can see parent private items in Rust).
// Sub-modules below then do their own `use super::X` to pull from THIS scope.

use super::*;

// ── tests (M6.2) ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests_state_extract {
    use super::extract_state_content;

    #[test]
    fn no_tag_unchanged() {
        let (text, state) = extract_state_content("Hello world");
        assert_eq!(text, "Hello world");
        assert!(state.is_none());
    }

    #[test]
    fn single_tag_stripped_and_parsed() {
        let (text, state) = extract_state_content("Turn\n<state>{\"hp\":100}</state>\nEnd");
        assert!(!text.contains("<state>"));
        assert_eq!(text, "Turn\n\nEnd");
        assert_eq!(state.unwrap()["hp"], 100);
    }

    #[test]
    fn multiple_tags_last_wins() {
        let (text, state) =
            extract_state_content("<state>{\"hp\":50}</state>mid<state>{\"hp\":99}</state>tail");
        assert_eq!(text, "midtail");
        assert_eq!(state.unwrap()["hp"], 99);
    }

    #[test]
    fn invalid_json_block_removed_state_none() {
        let (text, state) = extract_state_content("<state>not json</state>content");
        assert_eq!(text, "content");
        assert!(state.is_none());
    }

    #[test]
    fn unclosed_tag_kept_in_text() {
        let (text, state) = extract_state_content("before<state>unclosed");
        assert!(text.contains("<state>unclosed"), "text={:?}", text);
        assert!(state.is_none());
    }

    #[test]
    fn mixed_valid_invalid_takes_last_valid() {
        let input = "<state>bad</state>sep<state>{\"x\":1}</state>sep2<state>bad2</state>end";
        let (text, state) = extract_state_content(input);
        assert_eq!(text, "sepsep2end");
        assert_eq!(state.unwrap()["x"], 1);
    }
}

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;
    use crate::adapter::{BackendEngine, Provider};
    use crate::config::VolumeConfig;
    use crate::daemon::{MutableConfig, UserProfile};
    use crate::types::CharacterId;
    use std::collections::HashMap;
    use tempfile::tempdir;

    /// 构造一份最小可用的 `DaemonState`，data_root 指向临时目录。
    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            settings_update: Default::default(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        })
    }

    /// 构造一份默认 `ChatCompletionRequest`，调用方按需覆盖字段。
    fn base_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            character_id: None,
            character_card_id: None,
            lorebook_path: None,
            user_profile: UserProfile {
                name: "Tester".to_string(),
                variables: HashMap::new(),
            },
            message: "hello".to_string(),
            messages_history: None,
            regex_filters: None,
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider: None,
            endpoint: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            scene_id: None,
            user_id: None,
            persona_id: None,
        }
    }

    #[test]
    fn preview_pipeline_is_write_free_and_traces_actual_payload() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let mut req = base_request();
        req.character_id = Some(CharacterId::new("alice").unwrap());
        req.character_card_id = Some(
            r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"Alice","description":"A careful archivist","personality":"observant","scenario":"A quiet library","first_mes":"","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#.to_string(),
        );
        req.endpoint = Some("https://example.test/v1/chat/completions?token=secret".to_string());
        req.api_key = Some("never-serialize-me".to_string());
        req.messages_history = Some(vec![ChatMessage {
            role: crate::adapter::MessageRole::Assistant,
            content: "Earlier reply".to_string(),
        }]);

        let pipeline = preview_pipeline(&req, &state).unwrap();

        assert!(
            !tmp.path().join("characters/alice").exists(),
            "preview must not advance timeline or create chat history"
        );
        let kinds: Vec<_> = pipeline
            .prompt_trace
            .segments
            .iter()
            .map(|segment| segment.source_kind.as_str())
            .collect();
        assert_eq!(kinds, ["card", "history", "user"]);
        let payload_chars = pipeline.system_prompt.chars().count()
            + pipeline
                .messages
                .iter()
                .map(|message| message.content.chars().count())
                .sum::<usize>();
        assert_eq!(pipeline.prompt_trace.total_chars, payload_chars);
        assert_eq!(pipeline.prompt_trace.effective.model, "test-model");
        assert_eq!(pipeline.prompt_trace.effective.endpoint, "configured");

        let json = serde_json::to_string(&pipeline.prompt_trace).unwrap();
        assert!(!json.contains("never-serialize-me"));
        assert!(!json.contains("token=secret"));
        assert!(!json.contains("A careful archivist"));
    }

    #[test]
    fn prepare_rejects_traversal_in_character_card_id() {
        // character_card_id 是裸路径，必须拒绝 `..` 跨出 data_root。
        // 构造一个真实存在的"外部"文件，让 canonicalize 能成功 → 触发
        // safe_resolve_under_data_root 的 PathEscape 检查（而非 NotFound）。
        let outer = tempdir().unwrap();
        let data_root = outer.path().join("data");
        std::fs::create_dir_all(&data_root).unwrap();
        let outside_file = outer.path().join("secret.json");
        std::fs::write(&outside_file, "{}").unwrap();

        let state = make_state(data_root);
        let mut req = base_request();
        req.character_card_id = Some("../secret.json".to_string());

        let res = prepare_pipeline(&req, &state);
        assert!(res.is_err(), "应拒绝 ../ traversal");
        let err_msg = format!("{:?}", res.err().unwrap());
        assert!(
            err_msg.contains("PathEscape"),
            "expected PathEscape, got: {}",
            err_msg
        );
    }

    #[test]
    fn prepare_rejects_absolute_lorebook_path() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let mut req = base_request();
        req.lorebook_path = Some("/etc/passwd".to_string());

        let res = prepare_pipeline(&req, &state);
        assert!(res.is_err(), "应拒绝绝对路径");
    }

    #[test]
    fn prepare_rejects_malformed_inline_character_card_json() {
        // `{...}` 起头识别为内联 JSON；非法 JSON 应被 Orchestrator 拒绝
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let mut req = base_request();
        req.character_card_id = Some("{ not valid json".to_string());

        let res = prepare_pipeline(&req, &state);
        assert!(res.is_err(), "非法内联角色卡 JSON 应失败");
        let err_msg = format!("{:?}", res.err().unwrap());
        assert!(
            err_msg.contains("Orchestrator") || err_msg.contains("角色卡"),
            "unexpected err: {}",
            err_msg
        );
    }

    #[test]
    fn prepare_rejects_malformed_inline_lorebook_json() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let mut req = base_request();
        req.lorebook_path = Some("{ malformed".to_string());

        let res = prepare_pipeline(&req, &state);
        assert!(res.is_err(), "非法内联世界书 JSON 应失败");
    }

    #[test]
    fn prepare_succeeds_with_minimal_inputs() {
        // 没有角色卡 / 预设 / 历史 / 卷上下文的最小 payload 应走通
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let req = base_request();
        let res = prepare_pipeline(&req, &state);
        assert!(res.is_ok(), "minimal pipeline 失败: {:?}", res.err());
        let pipeline = res.unwrap();
        // 最末的 user message 应当在 messages 列表里
        assert_eq!(pipeline.messages.last().unwrap().content, "hello");
    }

    #[test]
    fn prepare_resolves_provider_priority_request_over_default() {
        // 请求体里指定 endpoint 应覆盖 daemon 默认
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let mut req = base_request();
        req.endpoint = Some("https://override.test/v1".to_string());
        req.api_key = Some("override-key".to_string());
        req.model = Some("override-model".to_string());

        let pipeline = prepare_pipeline(&req, &state).unwrap();
        assert_eq!(
            pipeline.provider_config.endpoint,
            "https://override.test/v1"
        );
        assert_eq!(
            pipeline.provider_config.api_key.as_deref(),
            Some("override-key")
        );
        assert_eq!(pipeline.gen_params.model, "override-model");
    }

    #[test]
    fn prepare_includes_default_seal_filter() {
        // FSM 必须默认带 <卷评估/> 过滤器，否则信号会泄漏到 UI
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let req = base_request();
        let pipeline = prepare_pipeline(&req, &state).unwrap();
        // FSM 不暴露 filters 字段，通过 process_chunk 间接验证：
        // 传入含 <卷评估/> 的文本，应被过滤掉
        let mut fsm = pipeline.fsm;
        let out = fsm.process_chunk("正文 <卷评估 封存=\"true\"/> 结尾");
        let tail = fsm.finish();
        let total = format!("{}{}", out, tail);
        assert!(
            !total.contains("卷评估"),
            "<卷评估/> 应被默认 filter 剥除，实际: {:?}",
            total
        );
    }

    #[test]
    fn prepare_loads_chat_history_when_omitted() {
        // R-04：客户端不传 messages_history 时，应从 ChatLog 自动加载
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let cid = CharacterId::new("alice").unwrap();
        // 预先写入一条历史
        let mut log =
            crate::chat_store::ChatLog::load_or_create(&state.data_root, cid.as_str()).unwrap();
        log.append(
            &state.data_root,
            crate::adapter::ChatMessage {
                role: crate::adapter::MessageRole::User,
                content: "上一轮用户消息".to_string(),
            },
        )
        .unwrap();
        log.append(
            &state.data_root,
            crate::adapter::ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content: "上一轮 AI 回复".to_string(),
            },
        )
        .unwrap();

        let mut req = base_request();
        req.character_id = Some(cid);
        // 故意不传 messages_history

        let pipeline = prepare_pipeline(&req, &state).unwrap();
        // 期望：messages 包含 [上一轮 user, 上一轮 assistant, 当前 user, 当前 user("hello")]
        // 注意 user message 在 prepare 里被先 append 到 ChatLog（步骤 7），
        // 然后步骤 12 又 push 一次当前 user。所以 ChatLog.recent 已含三条（上轮 user/assistant + 当前 user），
        // 加上步骤 12 重新 push，总数 = 4 条。验证最末是当前 user，且包含上轮内容。
        assert!(pipeline.messages.len() >= 3, "应至少加载历史 + 当前 user");
        assert_eq!(pipeline.messages.last().unwrap().content, "hello");
        let serialized = pipeline
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("|");
        assert!(serialized.contains("上一轮用户消息"));
        assert!(serialized.contains("上一轮 AI 回复"));
    }
}

// ── M_LS-3 tests: persist_live_state → history.jsonl ─────────────────────────

#[cfg(test)]
mod tests_mls3 {
    use super::persist_live_state;
    use std::io::BufRead;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_mls3_history_appended_on_first_call() {
        let tmp = tempdir().unwrap();
        let state = serde_json::json!({"hp": 80, "location": "dock"});
        persist_live_state(tmp.path(), "bob", &state).await;

        let history_path = crate::data_dir::char_state_history_path(tmp.path(), "bob");
        assert!(history_path.exists(), "history.jsonl should exist");

        let file = std::fs::File::open(&history_path).unwrap();
        let lines: Vec<String> = std::io::BufReader::new(file)
            .lines()
            .map(|l| l.unwrap())
            .collect();
        assert_eq!(lines.len(), 1);

        let entry: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert!(
            entry.get("timestamp").is_some(),
            "entry must have timestamp"
        );
        assert_eq!(entry["state"]["hp"], 80);
        assert_eq!(entry["state"]["location"], "dock");
    }

    #[tokio::test]
    async fn test_mls3_history_accumulates_multiple_calls() {
        let tmp = tempdir().unwrap();
        let s1 = serde_json::json!({"hp": 100});
        let s2 = serde_json::json!({"hp": 50});
        let s3 = serde_json::json!({"hp": 20});

        persist_live_state(tmp.path(), "carol", &s1).await;
        persist_live_state(tmp.path(), "carol", &s2).await;
        persist_live_state(tmp.path(), "carol", &s3).await;

        let history_path = crate::data_dir::char_state_history_path(tmp.path(), "carol");
        let file = std::fs::File::open(&history_path).unwrap();
        let lines: Vec<String> = std::io::BufReader::new(file)
            .lines()
            .map(|l| l.unwrap())
            .collect();
        assert_eq!(lines.len(), 3);

        let last: serde_json::Value = serde_json::from_str(&lines[2]).unwrap();
        assert_eq!(last["state"]["hp"], 20);
    }

    #[tokio::test]
    async fn test_mls3_live_json_overwritten_history_appended() {
        let tmp = tempdir().unwrap();
        let s1 = serde_json::json!({"turn": 1});
        let s2 = serde_json::json!({"turn": 2});

        persist_live_state(tmp.path(), "dave", &s1).await;
        persist_live_state(tmp.path(), "dave", &s2).await;

        let state_dir = crate::data_dir::char_state_dir(tmp.path(), "dave");
        let live_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(state_dir.join("live.json")).unwrap())
                .unwrap();
        assert_eq!(
            live_json["turn"], 2,
            "live.json should be overwritten with latest state"
        );

        let history_path = crate::data_dir::char_state_history_path(tmp.path(), "dave");
        let line_count = std::io::BufReader::new(std::fs::File::open(&history_path).unwrap())
            .lines()
            .count();
        assert_eq!(line_count, 2, "history should contain 2 entries");
    }
}

// ── M_LS LS-9 tests: extended coverage ───────────────────────────────────────

#[cfg(test)]
mod tests_mls9 {
    use super::{extract_state_content, persist_live_state};
    use crate::data_dir::{char_state_dir, char_state_history_path};
    use std::io::BufRead;
    use tempfile::tempdir;

    /// Full flow: extract → persist → inject in system prompt (LS-4/8 integration).
    #[tokio::test]
    async fn test_ls9_extract_persist_inject_roundtrip() {
        let tmp = tempdir().unwrap();
        let response = "Adventure awaits!\n<state>{\"hp\":90,\"location\":\"forest\"}</state>";

        let (stripped, state_opt) = extract_state_content(response);
        assert!(
            !stripped.contains("<state>"),
            "state block should be stripped"
        );
        assert_eq!(state_opt.as_ref().unwrap()["hp"], 90);

        let state = state_opt.unwrap();
        persist_live_state(tmp.path(), "eve", &state).await;

        // Inject into prompt and verify presence
        let mut prompt = String::new();
        crate::orchestrator::inject_live_state_for_test(tmp.path(), "eve", &mut prompt);
        assert!(
            prompt.contains("[Current State]"),
            "prompt should have state header"
        );
        assert!(
            prompt.contains("<state>"),
            "prompt should include update instruction"
        );
    }

    /// Empty state JSON `{}` persists live.json but renders empty state in prompt.
    #[tokio::test]
    async fn test_ls9_empty_state_object_persisted() {
        let tmp = tempdir().unwrap();
        let state = serde_json::json!({});
        persist_live_state(tmp.path(), "frank", &state).await;

        let live_path = char_state_dir(tmp.path(), "frank").join("live.json");
        assert!(
            live_path.exists(),
            "live.json should exist even for empty state"
        );
        let history = char_state_history_path(tmp.path(), "frank");
        let lines: Vec<_> = std::io::BufReader::new(std::fs::File::open(&history).unwrap())
            .lines()
            .map(|l| l.unwrap())
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "should have 1 history entry even for empty state"
        );
    }

    /// Schema `_max` priority: state `hp_max` overrides schema `max`.
    #[test]
    fn test_ls9_inject_state_max_from_state_takes_priority() {
        let tmp = tempdir().unwrap();
        let state_dir = char_state_dir(tmp.path(), "grace");
        std::fs::create_dir_all(&state_dir).unwrap();
        // State has hp=60, hp_max=120 (different from schema max=100)
        std::fs::write(
            state_dir.join("live.json"),
            r#"{"hp":60,"hp_max":120,"location":"cave"}"#,
        )
        .unwrap();
        let schema = serde_json::json!({
            "fields": [{"key":"hp","type":"number","min":0,"max":100,"label":"HP"}]
        });
        std::fs::write(
            state_dir.join("schema.json"),
            serde_json::to_string(&schema).unwrap(),
        )
        .unwrap();

        let mut prompt = String::new();
        crate::orchestrator::inject_live_state_for_test(tmp.path(), "grace", &mut prompt);
        // Should use state's hp_max=120, not schema's max=100
        assert!(
            prompt.contains("60/120"),
            "state hp_max=120 should take priority over schema max=100"
        );
    }
}

// ── MS-6 tests: scene pipeline branch ────────────────────────────────────────

#[cfg(test)]
mod tests_ms6 {
    use super::*;
    use crate::adapter::{BackendEngine, Provider};
    use crate::config::VolumeConfig;
    use crate::daemon::{MutableConfig, UserProfile};
    use crate::scene::{CharacterEntry, CharacterRole, LorebookMerge, SceneConfig};
    use crate::types::SceneId;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            settings_update: Default::default(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        })
    }

    fn scene_request(scene_id: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            character_id: None,
            character_card_id: None,
            lorebook_path: None,
            user_profile: UserProfile {
                name: "Player".to_string(),
                variables: HashMap::new(),
            },
            message: "hello scene".to_string(),
            messages_history: None,
            regex_filters: None,
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider: None,
            endpoint: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            scene_id: Some(scene_id.to_string()),
            user_id: None,
            persona_id: None,
        }
    }

    #[test]
    fn test_ms6_scene_pipeline_builds_multi_char_prompt() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let scene = SceneConfig {
            scene_id: SceneId::new("forest_scene").unwrap(),
            description: "A dark enchanted forest".to_string(),
            characters: vec![
                CharacterEntry {
                    character_id: "alice".to_string(),
                    role: CharacterRole::Primary,
                    intro: "Hero of the story".to_string(),
                },
                CharacterEntry {
                    character_id: "bob".to_string(),
                    role: CharacterRole::Npc,
                    intro: "A mysterious guide".to_string(),
                },
            ],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(tmp.path()).unwrap();

        let req = scene_request("forest_scene");
        let pipeline = prepare_pipeline(&req, &state).unwrap();

        assert!(
            pipeline.system_prompt.contains("A dark enchanted forest"),
            "prompt should contain scene description; got: {}",
            pipeline.system_prompt
        );
        assert!(
            pipeline.system_prompt.contains("alice") || pipeline.system_prompt.contains("Hero"),
            "prompt should mention primary character"
        );
        assert_eq!(
            pipeline.messages.last().unwrap().content,
            "hello scene",
            "current user message should be last"
        );
    }

    #[test]
    fn test_ms6_scene_pipeline_character_id_none_in_finalizer() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let scene = SceneConfig {
            scene_id: SceneId::new("empty_scene").unwrap(),
            description: String::new(),
            characters: vec![],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(tmp.path()).unwrap();

        let req = scene_request("empty_scene");
        let pipeline = prepare_pipeline(&req, &state).unwrap();
        assert!(
            pipeline.finalizer.character_id.is_none(),
            "scene mode must have no character_id in finalizer"
        );
    }

    #[test]
    fn test_ms6_invalid_scene_id_rejected() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());

        let mut req = scene_request("../evil");
        req.scene_id = Some("../evil".to_string());
        let res = prepare_pipeline(&req, &state);
        assert!(res.is_err(), "traversal scene_id must be rejected");
    }
}

// ── issue #27 tests: single / scene filter parity ────────────────────────────
//
// 回归：single 分支加载 PR-4 预设正则（presets/{id}/regex/*.json），而 scene 分支
// 此前漏加载，导致同一 preset 在 scene 模式下本应隐藏的 thought/status 段泄露。
// 抽出 assemble_regex_filters 共享后，两分支必须产出一致的过滤器集合。

#[cfg(test)]
mod tests_issue27 {
    use super::*;
    use crate::adapter::{BackendEngine, Provider};
    use crate::config::VolumeConfig;
    use crate::daemon::{MutableConfig, UserProfile};
    use crate::scene::{CharacterEntry, CharacterRole, LorebookMerge, SceneConfig};
    use crate::types::SceneId;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            settings_update: Default::default(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        })
    }

    fn base_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            character_id: None,
            character_card_id: None,
            lorebook_path: None,
            user_profile: UserProfile {
                name: "Tester".to_string(),
                variables: HashMap::new(),
            },
            message: "hello".to_string(),
            messages_history: Some(vec![]),
            regex_filters: Some(vec!["\\[系统:[\\s\\S]*?\\]".to_string()]),
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider: None,
            endpoint: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            scene_id: None,
            user_id: None,
            persona_id: None,
        }
    }

    /// 在 presets/{pid}/regex/ 写入一个隐藏 <thought> 的 SillyTavern 正则脚本。
    fn write_hide_thought_preset(root: &std::path::Path, pid: &str) {
        let regex_dir = root.join("presets").join(pid).join("regex");
        std::fs::create_dir_all(&regex_dir).unwrap();
        let script = r#"{
            "scriptName": "Hide Thoughts",
            "findRegex": "/<thought>[\\s\\S]*?<\\/thought>/gi",
            "replaceString": "",
            "placement": [2],
            "disabled": false
        }"#;
        std::fs::write(regex_dir.join("hide.json"), script).unwrap();
    }

    /// 核心回归：同一 payload（同一 preset）下，single 与 scene 分支必须产出
    /// **完全一致**的过滤器集合——证明两分支复用同一份加载逻辑，无泄露性不对称。
    #[test]
    fn test_issue27_single_and_scene_filters_are_consistent() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let pid = "hidepreset";
        write_hide_thought_preset(tmp.path(), pid);

        // scene 需要一个 SceneConfig；含 alice，与 single 分支同名角色。
        let scene = SceneConfig {
            scene_id: SceneId::new("s1").unwrap(),
            description: "parity scene".to_string(),
            characters: vec![CharacterEntry {
                character_id: "alice".to_string(),
                role: CharacterRole::Primary,
                intro: "hero".to_string(),
            }],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(tmp.path()).unwrap();

        // single 分支请求：character_id + 同一 preset_id + 同一 regex_filters。
        let mut single_req = base_request();
        single_req.character_id = Some(crate::types::CharacterId::new("alice").unwrap());
        single_req.preset_id = Some(crate::types::PresetId::new(pid).unwrap());

        // scene 分支请求：scene_id + 同一 preset_id，其余过滤器相关字段一致。
        let mut scene_req = base_request();
        scene_req.scene_id = Some("s1".to_string());
        scene_req.preset_id = Some(crate::types::PresetId::new(pid).unwrap());

        let single_pipe = prepare_pipeline(&single_req, &state).unwrap();
        let scene_pipe = prepare_pipeline(&scene_req, &state).unwrap();

        // 两分支过滤器集合逐项一致（顺序 + 内容）。RegexFilter 派生 PartialEq。
        assert_eq!(
            single_pipe.fsm.filters_for_test(),
            scene_pipe.fsm.filters_for_test(),
            "single 与 scene 分支应产出一致的过滤器集合"
        );
    }

    /// 具体泄露断言：scene 分支的过滤器集合必须像 single 一样含 preset 的
    /// <thought> 过滤器。修复前 scene 分支不加载 PR-4 预设正则，此断言会失败。
    #[test]
    fn test_issue27_scene_includes_preset_thought_filter() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let pid = "hidepreset";
        write_hide_thought_preset(tmp.path(), pid);

        let scene = SceneConfig {
            scene_id: SceneId::new("s1").unwrap(),
            description: "leak scene".to_string(),
            characters: vec![],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(tmp.path()).unwrap();

        let mut scene_req = base_request();
        scene_req.regex_filters = None;
        scene_req.scene_id = Some("s1".to_string());
        scene_req.preset_id = Some(crate::types::PresetId::new(pid).unwrap());

        let scene_pipe = prepare_pipeline(&scene_req, &state).unwrap();
        let filters = scene_pipe.fsm.filters_for_test();
        assert!(
            filters
                .iter()
                .any(|f| f.start == "<thought>" && f.end == "</thought>"),
            "scene 分支应含 preset <thought> 过滤器，实际: {:?}",
            filters
        );
    }
}

#[cfg(test)]
mod tests_dx1 {
    use super::*;
    use crate::adapter::{BackendEngine, Provider};
    use crate::config::VolumeConfig;
    use crate::daemon::{MutableConfig, UserProfile};

    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            settings_update: Default::default(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        })
    }

    #[test]
    fn test_dx1_no_user_id_uses_data_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let effective = data_dir::resolve_effective_root(&root, None).unwrap();
        assert_eq!(effective, root);
    }

    #[test]
    fn test_dx1_empty_user_id_uses_data_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let effective = data_dir::resolve_effective_root(&root, Some("")).unwrap();
        assert_eq!(effective, root);
    }

    #[test]
    fn test_dx1_user_id_resolves_under_users_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let effective = data_dir::resolve_effective_root(&root, Some("alice")).unwrap();
        assert_eq!(effective, root.join("users").join("alice"));
        // subdirs created
        assert!(effective.join("characters").exists());
        assert!(effective.join("presets").exists());
        assert!(effective.join("scenes").exists());
    }

    #[test]
    fn test_dx1_different_users_get_different_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let alice = data_dir::resolve_effective_root(&root, Some("alice")).unwrap();
        let bob = data_dir::resolve_effective_root(&root, Some("bob")).unwrap();
        assert_ne!(alice, bob);
        assert!(alice.ends_with("alice"));
        assert!(bob.ends_with("bob"));
    }

    #[test]
    fn test_dx1_invalid_user_id_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        assert!(data_dir::resolve_effective_root(&root, Some("../evil")).is_err());
        assert!(data_dir::resolve_effective_root(&root, Some("a/b")).is_err());
        assert!(data_dir::resolve_effective_root(&root, Some("")).is_ok()); // empty = no-op
    }

    #[test]
    fn test_dx1_pipeline_user_id_creates_isolated_root() {
        let tmp = tempfile::tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let req = ChatCompletionRequest {
            character_id: None,
            character_card_id: Some(
                r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"T","description":"","personality":"","scenario":"","first_mes":"Hi","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#
                    .to_string(),
            ),
            lorebook_path: None,
            user_profile: UserProfile {
                name: "User".to_string(),
                variables: std::collections::HashMap::new(),
            },
            message: "Hello".to_string(),
            messages_history: Some(vec![]),
            regex_filters: None,
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider: None,
            endpoint: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            scene_id: None,
            user_id: Some("alice".to_string()),
            persona_id: None,
        };
        // Pipeline should build without error; alice's user root is created
        let result = prepare_pipeline(&req, &state);
        assert!(
            result.is_ok(),
            "pipeline with user_id should succeed: {:?}",
            result.err()
        );
        // Verify effective root was alice's dir
        assert!(tmp.path().join("users").join("alice").exists());
    }
}

#[cfg(test)]
mod tests_a1b_merge {
    use super::*;
    use crate::daemon::UserProfile;
    use std::collections::HashMap;

    fn persona_with(name: &str, vars: &[(&str, &str)]) -> Persona {
        let mut p = Persona::initial(name);
        p.variables = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        p
    }

    #[test]
    fn none_persona_returns_user_profile_unchanged() {
        let up = UserProfile {
            name: "Alice".to_string(),
            variables: HashMap::from([("k".to_string(), "v".to_string())]),
        };
        let (name, vars) = merge_persona_into_user_profile(&up, None);
        assert_eq!(name, "Alice");
        assert_eq!(vars.get("k").unwrap(), "v");
    }

    #[test]
    fn empty_user_name_falls_back_to_persona_name() {
        let up = UserProfile {
            name: String::new(),
            variables: HashMap::new(),
        };
        let persona = persona_with("PersonaName", &[]);
        let (name, _) = merge_persona_into_user_profile(&up, Some(&persona));
        assert_eq!(name, "PersonaName");
    }

    #[test]
    fn nonempty_user_name_overrides_persona_name() {
        let up = UserProfile {
            name: "Client".to_string(),
            variables: HashMap::new(),
        };
        let persona = persona_with("PersonaName", &[]);
        let (name, _) = merge_persona_into_user_profile(&up, Some(&persona));
        assert_eq!(name, "Client");
    }

    #[test]
    fn request_variables_override_persona_variables() {
        let up = UserProfile {
            name: "User".to_string(),
            variables: HashMap::from([("tone".to_string(), "casual".to_string())]),
        };
        let persona = persona_with("Persona", &[("tone", "formal"), ("mood", "calm")]);
        let (_, vars) = merge_persona_into_user_profile(&up, Some(&persona));
        assert_eq!(vars.get("tone").unwrap(), "casual", "request must override");
        assert_eq!(
            vars.get("mood").unwrap(),
            "calm",
            "persona-only key must survive"
        );
    }
}

#[cfg(test)]
mod tests_a1b_resolve {
    use super::*;
    use crate::daemon::UserProfile;
    use crate::domain::{Persona, PersonaBinding, PersonaService};
    use crate::types::{CharacterId, SessionId};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn base_request_with_user(user_id: Option<&str>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            character_id: None,
            character_card_id: None,
            lorebook_path: None,
            user_profile: UserProfile {
                name: "Client".to_string(),
                variables: HashMap::new(),
            },
            message: "hi".to_string(),
            messages_history: None,
            regex_filters: None,
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider: None,
            endpoint: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            scene_id: None,
            user_id: user_id.map(str::to_string),
            persona_id: None,
        }
    }

    #[test]
    fn no_user_id_returns_none_and_is_backward_compatible() {
        let tmp = tempdir().unwrap();
        let req = base_request_with_user(None);
        let persona = resolve_request_persona(&req, tmp.path()).unwrap();
        assert!(persona.is_none(), "user_id absent → no persona resolution");
    }

    #[test]
    fn default_persona_is_returned_when_no_explicit_id_and_no_bindings() {
        let tmp = tempdir().unwrap();
        let req = base_request_with_user(Some("alice"));
        let persona = resolve_request_persona(&req, tmp.path()).unwrap().unwrap();
        assert_eq!(persona.id, "default");
        assert_eq!(
            persona.revision, 0,
            "fresh default is initial, no disk write"
        );
        // No persona file should have been written.
        assert!(!tmp
            .path()
            .join("users")
            .join("alice")
            .join("persona.json")
            .exists());
    }

    #[test]
    fn explicit_persona_id_resolves_stored_persona() {
        let tmp = tempdir().unwrap();
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "Writer".to_string(),
            description: String::new(),
            variables: HashMap::from([("tone".to_string(), "concise".to_string())]),
            id: "writer".to_string(),
            bindings: Vec::new(),
        };
        let saved = service.save(&uid, "writer", 0, stored).unwrap();
        assert_eq!(saved.revision, 1);

        let mut req = base_request_with_user(Some("alice"));
        req.persona_id = Some("writer".to_string());
        let persona = resolve_request_persona(&req, tmp.path()).unwrap().unwrap();
        assert_eq!(persona.id, "writer");
        assert_eq!(persona.name, "Writer");
        assert_eq!(persona.variables.get("tone").unwrap(), "concise");
    }

    #[test]
    fn explicit_persona_id_canonicalizes_default_case_insensitive() {
        let tmp = tempdir().unwrap();
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona::initial("StoredDefault");
        service.save_default(&uid, 0, stored).unwrap();

        let mut req = base_request_with_user(Some("alice"));
        req.persona_id = Some("DEFAULT".to_string());
        let persona = resolve_request_persona(&req, tmp.path()).unwrap().unwrap();
        assert_eq!(persona.id, "default");
    }

    #[test]
    fn explicit_nonexistent_persona_id_returns_not_found() {
        let tmp = tempdir().unwrap();
        let mut req = base_request_with_user(Some("alice"));
        req.persona_id = Some("ghost".to_string());
        let err = resolve_request_persona(&req, tmp.path()).unwrap_err();
        assert!(
            matches!(err, crate::error::AirpError::NotFound(_)),
            "expected NotFound, got {:?}",
            err
        );
    }

    #[test]
    fn explicit_persona_id_rejects_path_traversal_at_pipeline_boundary() {
        let tmp = tempdir().unwrap();
        let mut req = base_request_with_user(Some("alice"));
        req.persona_id = Some("../escape".to_string());
        assert!(matches!(
            resolve_request_persona(&req, tmp.path()),
            Err(crate::error::AirpError::BadRequest(_))
        ));
    }

    #[test]
    fn bound_persona_is_resolved_via_find_for_character() {
        let tmp = tempdir().unwrap();
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "Adventurer".to_string(),
            description: String::new(),
            variables: HashMap::new(),
            id: "adventurer".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "adventurer", 0, stored).unwrap();
        service
            .bind(
                &uid,
                "adventurer",
                PersonaBinding {
                    character_id: "hero".to_string(),
                    session_id: None,
                },
            )
            .unwrap();

        let mut req = base_request_with_user(Some("alice"));
        req.character_id = Some(CharacterId::new("hero").unwrap());
        let persona = resolve_request_persona(&req, tmp.path()).unwrap().unwrap();
        assert_eq!(persona.id, "adventurer");
        assert_eq!(persona.name, "Adventurer");
    }

    #[test]
    fn bound_persona_session_scoped_wins_over_generic() {
        let tmp = tempdir().unwrap();
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        // generic persona: bound to character "hero" without session_id
        let generic = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "GenericHero".to_string(),
            description: String::new(),
            variables: HashMap::new(),
            id: "generic-hero".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "generic-hero", 0, generic).unwrap();
        service
            .bind(
                &uid,
                "generic-hero",
                PersonaBinding {
                    character_id: "hero".to_string(),
                    session_id: None,
                },
            )
            .unwrap();
        // session-scoped persona: bound to (hero, session-a)
        let scoped = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "ScopedHero".to_string(),
            description: String::new(),
            variables: HashMap::new(),
            id: "scoped-hero".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "scoped-hero", 0, scoped).unwrap();
        service
            .bind(
                &uid,
                "scoped-hero",
                PersonaBinding {
                    character_id: "hero".to_string(),
                    session_id: Some("00000000-0000-0000-0000-00000000000a".to_string()),
                },
            )
            .unwrap();

        let mut req = base_request_with_user(Some("alice"));
        req.character_id = Some(CharacterId::new("hero").unwrap());
        req.session_id = Some(SessionId::parse("00000000-0000-0000-0000-00000000000a").unwrap());
        let persona = resolve_request_persona(&req, tmp.path()).unwrap().unwrap();
        assert_eq!(
            persona.id, "scoped-hero",
            "session-scoped binding must win over generic"
        );
    }

    #[test]
    fn scene_id_skips_find_for_character_and_falls_back_to_default() {
        let tmp = tempdir().unwrap();
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        // bind a persona to a character; scene mode should ignore this.
        let bound = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "ShouldNotActivate".to_string(),
            description: String::new(),
            variables: HashMap::new(),
            id: "bound-persona".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "bound-persona", 0, bound).unwrap();
        service
            .bind(
                &uid,
                "bound-persona",
                PersonaBinding {
                    character_id: "hero".to_string(),
                    session_id: None,
                },
            )
            .unwrap();

        let mut req = base_request_with_user(Some("alice"));
        req.scene_id = Some("scene-1".to_string());
        req.character_id = Some(CharacterId::new("hero").unwrap());
        let persona = resolve_request_persona(&req, tmp.path()).unwrap().unwrap();
        assert_eq!(
            persona.id, "default",
            "scene mode must skip find_for_character"
        );
        assert_ne!(persona.name, "ShouldNotActivate");
    }
}

/// 端到端 `prepare_pipeline` 集成测试：验证 A1b 把 persona 的 `variables` 真正
/// 注入到最终 system_prompt（通过 `{{key}}` 替换）。
#[cfg(test)]
mod tests_a1b_pipeline_e2e {
    use super::*;
    use crate::adapter::{BackendEngine, Provider};
    use crate::config::VolumeConfig;
    use crate::daemon::{MutableConfig, UserProfile};
    use crate::domain::{Persona, PersonaService};
    use crate::scene::{LorebookMerge, SceneConfig};
    use crate::types::SceneId;
    use crate::types::{CharacterId, UserId};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            settings_update: Default::default(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        })
    }

    /// 极简 chara_card_v2 JSON，description 含 `{{tone}}` 占位符。
    fn inline_card_with_tone_placeholder() -> String {
        r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"T","description":"{{tone}} writer","personality":"","scenario":"","first_mes":"Hi","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#
            .to_string()
    }

    fn base_chat_request(user_id: Option<&str>, persona_id: Option<&str>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            character_id: None,
            character_card_id: Some(inline_card_with_tone_placeholder()),
            lorebook_path: None,
            user_profile: UserProfile {
                name: "Client".to_string(),
                variables: HashMap::new(),
            },
            message: "hi".to_string(),
            messages_history: Some(vec![]),
            regex_filters: None,
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider: None,
            endpoint: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            scene_id: None,
            user_id: user_id.map(str::to_string),
            persona_id: persona_id.map(str::to_string),
        }
    }

    #[test]
    fn prepare_pipeline_injects_explicit_persona_variables_into_prompt() {
        let tmp = tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());

        let uid = UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "Writer".to_string(),
            description: String::new(),
            variables: HashMap::from([("tone".to_string(), "concise".to_string())]),
            id: "writer".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "writer", 0, stored).unwrap();

        let req = base_chat_request(Some("alice"), Some("writer"));
        let pipeline = prepare_pipeline(&req, &state).expect("pipeline should build");
        assert!(
            pipeline.system_prompt.contains("concise writer"),
            "persona variable must be substituted into prompt; got: {}",
            pipeline.system_prompt
        );
        assert!(
            !pipeline.system_prompt.contains("{{tone}}"),
            "placeholder must be replaced; got: {}",
            pipeline.system_prompt
        );
    }

    #[test]
    fn prepare_pipeline_injects_bound_persona_variables_into_prompt() {
        let tmp = tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());

        let uid = UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "Adventurer".to_string(),
            description: String::new(),
            variables: HashMap::from([("tone".to_string(), "brave".to_string())]),
            id: "adventurer".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "adventurer", 0, stored).unwrap();
        service
            .bind(
                &uid,
                "adventurer",
                crate::domain::PersonaBinding {
                    character_id: "hero".to_string(),
                    session_id: None,
                },
            )
            .unwrap();

        let mut req = base_chat_request(Some("alice"), None);
        // character_id triggers find_for_character; even with inline card the
        // bound persona resolves via the user's persona store.
        req.character_id = Some(CharacterId::new("hero").unwrap());
        let pipeline = prepare_pipeline(&req, &state).expect("pipeline should build");
        assert!(
            pipeline.system_prompt.contains("brave writer"),
            "bound persona variable must be substituted; got: {}",
            pipeline.system_prompt
        );
    }

    #[test]
    fn prepare_pipeline_returns_not_found_for_nonexistent_persona_id() {
        let tmp = tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());

        let mut req = base_chat_request(Some("alice"), Some("ghost"));
        req.character_id = Some(CharacterId::new("hero").unwrap());
        let result = prepare_pipeline(&req, &state);
        match result {
            Err(crate::error::AirpError::NotFound(_)) => {}
            other => panic!(
                "explicit nonexistent persona_id must fail closed with NotFound, got {:?}",
                other.map(|_| "Ok(..)")
            ),
        }
        let effective_root = tmp.path().join("users").join("alice");
        assert!(
            !effective_root.join("characters").join("hero").exists(),
            "rejected Persona selection must not create character chat or timeline state"
        );
    }

    #[test]
    fn prepare_pipeline_request_variables_override_persona_variables() {
        let tmp = tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());

        let uid = UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona {
            schema: Persona::SCHEMA,
            revision: 0,
            updated_at: String::new(),
            name: "Writer".to_string(),
            description: String::new(),
            variables: HashMap::from([("tone".to_string(), "concise".to_string())]),
            id: "writer".to_string(),
            bindings: Vec::new(),
        };
        service.save(&uid, "writer", 0, stored).unwrap();

        let mut req = base_chat_request(Some("alice"), Some("writer"));
        req.user_profile
            .variables
            .insert("tone".to_string(), "verbose".to_string());
        let pipeline = prepare_pipeline(&req, &state).expect("pipeline should build");
        assert!(
            pipeline.system_prompt.contains("verbose writer"),
            "request-side variable must override persona default; got: {}",
            pipeline.system_prompt
        );
        assert!(
            !pipeline.system_prompt.contains("concise writer"),
            "persona default must not survive when request overrides; got: {}",
            pipeline.system_prompt
        );
    }

    #[test]
    fn prepare_scene_pipeline_substitutes_persona_variables_in_prompt() {
        let tmp = tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let uid = UserId::new("alice").unwrap();
        PersonaService::new(tmp.path())
            .save(
                &uid,
                "writer",
                0,
                Persona {
                    schema: Persona::SCHEMA,
                    revision: 0,
                    updated_at: String::new(),
                    name: "Writer".to_string(),
                    description: String::new(),
                    variables: HashMap::from([("tone".to_string(), "concise".to_string())]),
                    id: "writer".to_string(),
                    bindings: Vec::new(),
                },
            )
            .unwrap();
        let effective_root =
            crate::data_dir::resolve_effective_root(tmp.path(), Some("alice")).unwrap();
        SceneConfig {
            scene_id: SceneId::new("writers-room").unwrap(),
            description: "A {{tone}} scene".to_string(),
            characters: Vec::new(),
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        }
        .save(&effective_root)
        .unwrap();

        let mut req = base_chat_request(Some("alice"), Some("writer"));
        req.character_card_id = None;
        req.scene_id = Some("writers-room".to_string());
        let pipeline = prepare_pipeline(&req, &state).unwrap();
        assert!(pipeline.system_prompt.contains("A concise scene"));
        assert!(!pipeline.system_prompt.contains("{{tone}}"));
    }
}
