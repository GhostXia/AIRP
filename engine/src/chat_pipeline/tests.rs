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
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
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
        }
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
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
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
            config: std::sync::RwLock::new(MutableConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                volume_config: VolumeConfig::default(),
                access_api_key: None,
                engine: BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
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
