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
    use std::collections::{BTreeMap, HashMap};
    use std::time::SystemTime;
    use tempfile::tempdir;

    /// 构造一份最小可用的 `DaemonState`，data_root 指向临时目录。
    fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            fts: Default::default(),
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
            swipe_candidates: Vec::new(),
            branch_from: None,
        }
    }

    fn snapshot_tree(root: &std::path::Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
        fn visit(
            root: &std::path::Path,
            dir: &std::path::Path,
            out: &mut BTreeMap<String, (Vec<u8>, SystemTime)>,
        ) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let metadata = entry.metadata().unwrap();
                if metadata.is_dir() {
                    out.insert(
                        format!("{relative}/"),
                        (Vec::new(), std::time::SystemTime::UNIX_EPOCH),
                    );
                    visit(root, &path, out);
                } else {
                    out.insert(
                        relative,
                        (std::fs::read(&path).unwrap(), metadata.modified().unwrap()),
                    );
                }
            }
        }
        let mut snapshot = BTreeMap::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    #[test]
    fn preview_pipeline_is_write_free_and_traces_actual_payload() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let character = tmp.path().join("characters/alice");
        std::fs::create_dir_all(character.join("history")).unwrap();
        std::fs::create_dir_all(character.join("gating")).unwrap();
        std::fs::create_dir_all(character.join("memory")).unwrap();
        std::fs::write(
            character.join("history/chat_log.jsonl"),
            "{\"role\":\"assistant\",\"content\":\"Earlier reply\"}\n",
        )
        .unwrap();
        std::fs::write(
            character.join("history/chat_log_meta.json"),
            "{\"sentinel\":true}",
        )
        .unwrap();
        std::fs::write(character.join("gating/timeline.md"), "- 累计消耗时槽: 4\n").unwrap();
        std::fs::write(
            character.join("gating/checkpoints.md"),
            "- 当前关卡: CP-1\n- 进度百分比: 40%\n",
        )
        .unwrap();
        std::fs::write(character.join("known.md"), "threshold clue").unwrap();
        std::fs::write(character.join("memory/current.md"), "existing context").unwrap();
        let before = snapshot_tree(tmp.path());
        let mut req = base_request();
        req.character_id = Some(CharacterId::new("alice").unwrap());
        req.character_card_id = Some(
            r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"Alice","description":"A careful archivist","personality":"observant","scenario":"A quiet library","first_mes":"","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#.to_string(),
        );
        req.endpoint = Some("https://example.test/v1/chat/completions?token=secret".to_string());
        req.api_key = Some("never-serialize-me".to_string());

        let pipeline = preview_pipeline(&req, &state).unwrap();

        assert!(character.exists());
        assert_eq!(
            snapshot_tree(tmp.path()),
            before,
            "preview changed persisted state"
        );
        let kinds: Vec<_> = pipeline
            .prompt_trace
            .segments
            .iter()
            .map(|segment| segment.source_kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            ["card", "known", "card", "memory", "history", "user"]
        );
        assert!(pipeline.system_prompt.contains("Current CP: CP-2"));
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

    /// #214 ISSUE-1: 未定义的 `{{lowercase_var}}` 残留在 system prompt 中时，
    /// `build_prompt_trace` 必须推送 `undefined_variable_placeholder` 诊断。
    /// 正则只匹配小写字母+下划线，不会误报 `{{getvar::x}}` / `{{lastUserMessage}}`。
    #[test]
    fn prompt_trace_flags_undefined_variable_placeholder() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let character = tmp.path().join("characters/alice");
        std::fs::create_dir_all(character.join("history")).unwrap();
        std::fs::create_dir_all(character.join("gating")).unwrap();
        std::fs::create_dir_all(character.join("memory")).unwrap();
        std::fs::write(
            character.join("history/chat_log.jsonl"),
            "{\"role\":\"assistant\",\"content\":\"Earlier reply\"}\n",
        )
        .unwrap();
        std::fs::write(
            character.join("history/chat_log_meta.json"),
            "{\"sentinel\":true}",
        )
        .unwrap();
        std::fs::write(character.join("memory/current.md"), "existing context").unwrap();

        let mut req = base_request();
        req.character_id = Some(CharacterId::new("alice").unwrap());
        // 描述中故意包含未定义的 {{weapon}} 和 {{armor_level}}；{{char}} 会被
        // final_vars 替换，{{getvar::x}} 不应被匹配（含 `::`），{{lastUserMessage}}
        // 不应被匹配（含大写字母）。
        req.character_card_id = Some(
            r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"Alice","description":"A careful archivist wielding {{weapon}} with {{armor_level}}. Macro test: {{getvar::x}} and {{lastUserMessage}} and {{char}}.","personality":"observant","scenario":"A quiet library","first_mes":"","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#
                .to_string(),
        );

        let pipeline = preview_pipeline(&req, &state).unwrap();

        let undefined_diag = pipeline
            .prompt_trace
            .diagnostics
            .iter()
            .find(|d| d.kind == "undefined_variable_placeholder");
        assert!(
            undefined_diag.is_some(),
            "expected undefined_variable_placeholder diagnostic; got diagnostics: {:?}",
            pipeline
                .prompt_trace
                .diagnostics
                .iter()
                .map(|d| &d.kind)
                .collect::<Vec<_>>()
        );
        let message = &undefined_diag.unwrap().message;
        assert!(
            message.contains("{{weapon}}"),
            "diagnostic should list {{weapon}}: {message}"
        );
        assert!(
            message.contains("{{armor_level}}"),
            "diagnostic should list {{armor_level}}: {message}"
        );
        // `{{getvar::x}}` 含 `::` 不应被匹配；`{{lastUserMessage}}` 含大写字母不应被匹配；
        // `{{char}}` 会被 final_vars 替换掉，也不应出现在诊断中。
        assert!(
            !message.contains("{{getvar::x}}"),
            "diagnostic should not flag getvar macro: {message}"
        );
        assert!(
            !message.contains("{{lastUserMessage}}"),
            "diagnostic should not flag lastUserMessage macro: {message}"
        );
        assert!(
            !message.contains("{{char}}"),
            "diagnostic should not flag {{char}} (substituted by final_vars): {message}"
        );
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

    // ── Phase 2h: trace 完整性收口 ───────────────────────────────────────────

    #[test]
    fn test_phase_2h_trace_fills_all_six_revisions_on_new_data() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        // effective_root（Chat 模式下 = data_root/users/{uid}）
        let uid = crate::types::UserId::new("phase2h-user").unwrap();
        let effective_root = state.data_root.join("users").join(uid.as_str());
        fs::create_dir_all(effective_root.join("characters")).unwrap();
        fs::create_dir_all(effective_root.join("presets")).unwrap();

        // 角色：写入 card.json + 手动创建 character revision 指针（绕过 import 路径）
        let cid = CharacterId::new("phase2h-char").unwrap();
        let char_dir = effective_root.join("characters").join(cid.as_str());
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("card.json"), r#"{"name":"Phase2h","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}"#).unwrap();
        fs::write(char_dir.join("current_revision"), "3").unwrap();
        // 角色 world（lorebook）revision
        let world_dir = char_dir.join("world");
        fs::create_dir_all(&world_dir).unwrap();
        fs::write(world_dir.join("current_revision"), "5").unwrap();
        // 角色 state revision
        let state_dir = char_dir.join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("current_revision"), "42").unwrap();
        // 角色 memory revision（已升级路径，CF-3 后 memory/ 为权威）
        let memory_dir = char_dir.join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("current_revision"), "9").unwrap();

        // Preset revision
        let preset_dir = effective_root.join("presets").join("phase2h-preset");
        fs::create_dir_all(&preset_dir).unwrap();
        fs::write(preset_dir.join("current_revision"), "2").unwrap();
        // Preset 必须存在 current 指针才能被加载（preset.rs::load 要求）
        fs::write(preset_dir.join("current"), "gen-phase2h").unwrap();
        fs::create_dir_all(preset_dir.join("versions").join("gen-phase2h")).unwrap();
        fs::write(
            preset_dir
                .join("versions")
                .join("gen-phase2h")
                .join("preset.json"),
            serde_json::json!({
                "schema_version": 1,
                "name": "phase2h-preset",
                "prompt_order": [],
                "prompts": [],
                "parameters": {}
            })
            .to_string(),
        )
        .unwrap();

        // Persona revision（双源读取：优先 current_revision）
        // personas 目录在 effective_root 下（effective_root 已含 users/{uid}）
        let persona_asset_dir = effective_root.join("personas").join("default");
        fs::create_dir_all(&persona_asset_dir).unwrap();
        fs::write(persona_asset_dir.join("current_revision"), "7").unwrap();
        // 同步写入工作副本 personas/default.json，使 PersonaService::load 能读到
        let persona_path = persona_asset_dir.with_file_name("default.json");
        fs::write(
            &persona_path,
            serde_json::json!({
                "schema": 2,
                "id": "default",
                "revision": 7,
                "updated_at": "2026-07-17T00:00:00Z",
                "name": "Phase2h Persona",
                "description": "phase2h test persona",
                "variables": {},
                "bindings": []
            })
            .to_string(),
        )
        .unwrap();

        let mut req = base_request();
        req.character_id = Some(cid.clone());
        req.preset_id = Some(crate::types::PresetId::new("phase2h-preset").unwrap());
        req.user_id = Some(uid.as_str().to_string());
        req.persona_id = Some(crate::types::PersonaId::new("default").unwrap());

        let pipeline = prepare_pipeline(&req, &state).unwrap();

        let eff = &pipeline.prompt_trace.effective;
        assert_eq!(
            eff.character_revision,
            Some(3),
            "character_revision 应填充 3"
        );
        assert_eq!(eff.lorebook_revision, Some(5), "lorebook_revision 应填充 5");
        assert_eq!(eff.state_revision, Some(42), "state_revision 应填充 42");
        assert_eq!(eff.memory_revision, Some(9), "memory_revision 应填充 9");
        assert_eq!(eff.preset_revision, Some(2), "preset_revision 应填充 2");
        assert_eq!(
            eff.persona_revision,
            Some(7),
            "persona_revision 应填充 7（双源优先读 current_revision）"
        );

        // 新数据上不应推送任何 *_revision_unavailable 诊断
        let unavailable: Vec<_> = pipeline
            .prompt_trace
            .diagnostics
            .iter()
            .filter(|d| d.kind.ends_with("_revision_unavailable"))
            .collect();
        assert!(
            unavailable.is_empty(),
            "新数据上不应推送 *_revision_unavailable，实际: {:?}",
            unavailable
        );
    }

    #[test]
    fn test_phase_2h_trace_pushes_all_six_unavailable_diagnostics_on_legacy_data() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        // 旧数据场景：仅创建角色目录与 card.json，无任何 current_revision 指针；
        // 也不创建 world/state/memory 目录、preset 目录、persona asset 目录。
        let uid = crate::types::UserId::new("phase2h-legacy").unwrap();
        let effective_root = state.data_root.join("users").join(uid.as_str());
        fs::create_dir_all(effective_root.join("characters")).unwrap();

        let cid = CharacterId::new("phase2h-legacy-char").unwrap();
        let char_dir = effective_root.join("characters").join(cid.as_str());
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(
            char_dir.join("card.json"),
            r#"{"name":"Phase2h Legacy","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}"#,
        )
        .unwrap();
        // 故意不写 char_dir/current_revision
        // 故意不创建 world/、state/、memory/ 子目录
        // 故意不创建 presets/phase2h-legacy-preset/ 目录
        // 故意不创建 personas/default/ 目录（PersonaService::get_default 会回退到 Persona::initial，
        //   revision = 0，触发我们新加的 "Persona.revision == 0 视作 unavailable" 分支）

        let mut req = base_request();
        req.character_id = Some(cid.clone());
        req.preset_id = Some(crate::types::PresetId::new("phase2h-legacy-preset").unwrap());
        req.user_id = Some(uid.as_str().to_string());
        // 不设置 req.persona_id —— 让 PersonaService 走 get_default 兜底

        let pipeline = prepare_pipeline(&req, &state).unwrap();

        let mut kinds: Vec<String> = pipeline
            .prompt_trace
            .diagnostics
            .iter()
            .map(|d| d.kind.clone())
            .collect();
        kinds.sort();
        let expected = vec![
            "character_revision_unavailable",
            "lorebook_revision_unavailable",
            "memory_revision_unavailable",
            "persona_revision_unavailable",
            "preset_revision_unavailable",
            "state_revision_unavailable",
        ];
        assert_eq!(
            kinds, expected,
            "6 个 *_revision_unavailable 诊断都应推送，实际: {:?}",
            kinds
        );

        // 全部 revision 字段应为 None
        let eff = &pipeline.prompt_trace.effective;
        assert!(
            eff.character_revision.is_none(),
            "character_revision 应 None"
        );
        assert!(eff.lorebook_revision.is_none(), "lorebook_revision 应 None");
        assert!(eff.state_revision.is_none(), "state_revision 应 None");
        assert!(eff.memory_revision.is_none(), "memory_revision 应 None");
        assert!(eff.preset_revision.is_none(), "preset_revision 应 None");
        assert!(eff.persona_revision.is_none(), "persona_revision 应 None");
    }

    /// Phase 2h：scene 模式下 character_revision 字段语义不适用（多角色无单一 revision），
    /// 应留 `None` 且**不**推送 `character_revision_unavailable` 诊断。
    /// 与 `test_ms6_scene_pipeline_builds_multi_char_prompt`（line ~748）互补：
    /// 那条测试断言 character_id 为 None；这条断言 character_revision 为 None。
    ///
    /// CodeRabbit 审计修复（nitpick）：扩展断言覆盖 lorebook / state / memory 三个字段，
    /// 它们在 scene 模式下同样不适用，都应留 None 且不推送对应 *_revision_unavailable 诊断。
    #[test]
    fn test_phase_2h_scene_mode_leaves_character_revision_none_without_diagnostic() {
        use crate::scene::{CharacterEntry, CharacterRole, LorebookMerge, SceneConfig};
        use crate::types::SceneId;

        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let scene = SceneConfig {
            scene_id: SceneId::new("phase2h_scene").unwrap(),
            description: "phase2h scene".to_string(),
            characters: vec![CharacterEntry {
                character_id: "phase2h_scene_char".to_string(),
                role: CharacterRole::Primary,
                intro: String::new(),
            }],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(tmp.path()).unwrap();

        let mut req = base_request();
        req.scene_id = Some(SceneId::new("phase2h_scene").unwrap().to_string());
        // 同时提供 character_id，应被 scene 模式忽略（参见 chat_pipeline/tests.rs:774-794 现有 scene 测试）
        req.character_id = Some(CharacterId::new("ignored-character").unwrap());

        let pipeline = prepare_pipeline(&req, &state).unwrap();
        let eff = &pipeline.prompt_trace.effective;
        let diag_kinds: Vec<_> = pipeline
            .prompt_trace
            .diagnostics
            .iter()
            .map(|d| d.kind.as_str())
            .collect();

        // character / lorebook / state / memory 在 scene 模式下都应留 None
        assert!(
            eff.character_revision.is_none(),
            "scene: character_revision 应 None"
        );
        assert!(
            eff.lorebook_revision.is_none(),
            "scene: lorebook_revision 应 None"
        );
        assert!(
            eff.state_revision.is_none(),
            "scene: state_revision 应 None"
        );
        assert!(
            eff.memory_revision.is_none(),
            "scene: memory_revision 应 None"
        );
        assert!(eff.character_id.is_none(), "scene: character_id 应 None");

        // scene 模式不应推送任何 character/lorebook/state/memory 相关的 *_revision_unavailable
        for kind in [
            "character_revision_unavailable",
            "lorebook_revision_unavailable",
            "state_revision_unavailable",
            "memory_revision_unavailable",
        ] {
            assert!(
                !diag_kinds.contains(&kind),
                "scene 模式不应推送 {kind}，实际诊断: {:?}",
                diag_kinds
            );
        }
    }

    /// CodeRabbit 审计阻塞修复：当 `character_card_id` 或 `lorebook_path` 显式提供
    /// 外部 card / lorebook 源时，不读取 `characters/{cid}/` 下的 canonical revision
    /// 指针——实际 prompt 内容不来自该目录，读取会产生误导性 revision。
    /// 应留 None 且不推送对应 *_revision_unavailable 诊断。
    #[test]
    fn test_phase_2h_external_card_and_lorebook_skip_canonical_revision() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let uid = crate::types::UserId::new("phase2h-ext").unwrap();
        let effective_root = state.data_root.join("users").join(uid.as_str());
        fs::create_dir_all(effective_root.join("characters")).unwrap();

        let cid = CharacterId::new("phase2h-ext-char").unwrap();
        let char_dir = effective_root.join("characters").join(cid.as_str());
        fs::create_dir_all(&char_dir).unwrap();
        // canonical 目录下有 current_revision，但因为使用了外部 card，不应读取它
        fs::write(char_dir.join("current_revision"), "99").unwrap();
        // world/ 也放一个 current_revision，但因 lorebook_path 被指定，不应读取
        let world_dir = char_dir.join("world");
        fs::create_dir_all(&world_dir).unwrap();
        fs::write(world_dir.join("current_revision"), "88").unwrap();

        let mut req = base_request();
        req.character_id = Some(cid.clone());
        req.user_id = Some(uid.as_str().to_string());
        // 外部内联 card（JSON 字符串）—— character_card_id 非 None，跳过 canonical revision
        req.character_card_id = Some(
            r#"{"name":"External","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}"#.to_string(),
        );
        // 外部内联 lorebook（JSON 字符串）—— lorebook_path 非 None，跳过 canonical revision
        req.lorebook_path = Some(r#"{"entries":[]}"#.to_string());

        let pipeline = prepare_pipeline(&req, &state).unwrap();
        let eff = &pipeline.prompt_trace.effective;
        let diag_kinds: Vec<_> = pipeline
            .prompt_trace
            .diagnostics
            .iter()
            .map(|d| d.kind.as_str())
            .collect();

        assert!(
            eff.character_revision.is_none(),
            "外部 card 时 character_revision 应 None（不读 canonical 99）"
        );
        assert!(
            eff.lorebook_revision.is_none(),
            "外部 lorebook 时 lorebook_revision 应 None（不读 canonical 88）"
        );
        assert!(
            !diag_kinds.contains(&"character_revision_unavailable"),
            "外部 card 不应推送 character_revision_unavailable"
        );
        assert!(
            !diag_kinds.contains(&"lorebook_revision_unavailable"),
            "外部 lorebook 不应推送 lorebook_revision_unavailable"
        );
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
        persist_live_state(tmp.path(), "bob", &state).await.unwrap();

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

        persist_live_state(tmp.path(), "carol", &s1).await.unwrap();
        persist_live_state(tmp.path(), "carol", &s2).await.unwrap();
        persist_live_state(tmp.path(), "carol", &s3).await.unwrap();

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

        persist_live_state(tmp.path(), "dave", &s1).await.unwrap();
        persist_live_state(tmp.path(), "dave", &s2).await.unwrap();

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
        persist_live_state(tmp.path(), "eve", &state).await.unwrap();

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
        persist_live_state(tmp.path(), "frank", &state)
            .await
            .unwrap();

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
            fts: Default::default(),
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
            swipe_candidates: Vec::new(),
            branch_from: None,
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

        let mut req = scene_request("forest_scene");
        // Scene mode ignores a simultaneously supplied character_id; the trace must not report
        // that ignored identity as effective configuration.
        req.character_id = Some(crate::types::CharacterId::new("ignored-character").unwrap());
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
        assert!(pipeline.prompt_trace.effective.character_id.is_none());
        assert!(!pipeline
            .prompt_trace
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "character_revision_unavailable"));
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
    fn scene_lorebook_trace_preserves_character_and_scene_entry_provenance() {
        let tmp = tempdir().unwrap();
        let state = make_state(tmp.path().to_path_buf());
        crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();

        let scene_id = SceneId::new("forest_scene").unwrap();
        SceneConfig {
            scene_id: scene_id.clone(),
            description: "A forest".to_string(),
            characters: vec![CharacterEntry {
                character_id: "alice".to_string(),
                role: CharacterRole::Primary,
                intro: String::new(),
            }],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        }
        .save(tmp.path())
        .unwrap();

        let character_lorebook = crate::data_dir::char_world_lorebook_path(tmp.path(), "alice");
        std::fs::create_dir_all(character_lorebook.parent().unwrap()).unwrap();
        std::fs::write(
            &character_lorebook,
            r#"{"entries":[{"keys":["hello"],"content":"Alice lore","enabled":true,"priority":20}]}"#,
        )
        .unwrap();

        let scene_lorebook = crate::data_dir::scene_world_lorebook_path(tmp.path(), &scene_id);
        std::fs::create_dir_all(scene_lorebook.parent().unwrap()).unwrap();
        std::fs::write(
            &scene_lorebook,
            r#"{"entries":[{"keys":["scene"],"content":"Scene lore","enabled":true,"priority":10}]}"#,
        )
        .unwrap();

        let pipeline = prepare_pipeline(&scene_request("forest_scene"), &state).unwrap();
        let lorebook_segments: Vec<_> = pipeline
            .prompt_trace
            .segments
            .iter()
            .filter(|segment| segment.source_kind == "lorebook")
            .collect();

        assert_eq!(lorebook_segments.len(), 2);
        assert_eq!(
            lorebook_segments[0].source_id.as_deref(),
            Some("character:alice")
        );
        assert_eq!(lorebook_segments[0].item_id.as_deref(), Some("0"));
        assert_eq!(
            lorebook_segments[1].source_id.as_deref(),
            Some("scene:forest_scene")
        );
        assert_eq!(lorebook_segments[1].item_id.as_deref(), Some("0"));
        assert!(lorebook_segments[0].position < lorebook_segments[1].position);

        let expected_lore =
            "[世界书信息]\n\n[World Info/Lorebook Information]:\nAlice lore\nScene lore\n\n\n";
        assert!(pipeline.system_prompt.contains(expected_lore));
        let trace_json = serde_json::to_string(&pipeline.prompt_trace).unwrap();
        assert!(!trace_json.contains(&tmp.path().display().to_string()));
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
            fts: Default::default(),
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
            swipe_candidates: Vec::new(),
            branch_from: None,
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
            fts: Default::default(),
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
    fn p1_default_user_uses_global_session_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        assert_eq!(
            effective_root_for_mode(&root, Some("default"), PrepareMode::Chat).unwrap(),
            root
        );
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
            swipe_candidates: Vec::new(),
            branch_from: None,
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
            swipe_candidates: Vec::new(),
            branch_from: None,
        }
    }

    #[test]
    fn no_user_id_returns_none_and_is_backward_compatible() {
        let tmp = tempdir().unwrap();
        let req = base_request_with_user(None);
        let (persona, source) = resolve_request_persona(&req, tmp.path()).unwrap();
        assert!(persona.is_none(), "user_id absent → no persona resolution");
        assert_eq!(
            source,
            crate::orchestrator::trace::PersonaActivationSource::Absent
        );
    }

    #[test]
    fn default_persona_is_returned_when_no_explicit_id_and_no_bindings() {
        let tmp = tempdir().unwrap();
        let req = base_request_with_user(Some("alice"));
        let (persona, source) = resolve_request_persona(&req, tmp.path()).unwrap();
        let persona = persona.unwrap();
        assert_eq!(persona.id, "default");
        assert_eq!(
            persona.revision, 0,
            "fresh default is initial, no disk write"
        );
        assert_eq!(
            source,
            crate::orchestrator::trace::PersonaActivationSource::Default,
            "no explicit id + no binding → Default source"
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
        req.persona_id = Some(crate::types::PersonaId::new("writer").unwrap());
        let (persona, source) = resolve_request_persona(&req, tmp.path()).unwrap();
        let persona = persona.unwrap();
        assert_eq!(persona.id, "writer");
        assert_eq!(persona.name, "Writer");
        assert_eq!(persona.variables.get("tone").unwrap(), "concise");
        assert_eq!(
            source,
            crate::orchestrator::trace::PersonaActivationSource::Explicit,
            "explicit persona_id → Explicit source"
        );
    }

    #[test]
    fn explicit_persona_id_canonicalizes_default_case_insensitive() {
        let tmp = tempdir().unwrap();
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = PersonaService::new(tmp.path());
        let stored = Persona::initial("StoredDefault");
        service.save_default(&uid, 0, stored).unwrap();

        let mut req = base_request_with_user(Some("alice"));
        req.persona_id = Some(crate::types::PersonaId::new("DEFAULT").unwrap());
        let persona = resolve_request_persona(&req, tmp.path())
            .unwrap()
            .0
            .unwrap();
        assert_eq!(persona.id, "default");
    }

    #[test]
    fn explicit_nonexistent_persona_id_returns_not_found() {
        let tmp = tempdir().unwrap();
        let mut req = base_request_with_user(Some("alice"));
        req.persona_id = Some(crate::types::PersonaId::new("ghost").unwrap());
        let err = resolve_request_persona(&req, tmp.path()).unwrap_err();
        assert!(
            matches!(err, crate::error::AirpError::NotFound(_)),
            "expected NotFound, got {:?}",
            err
        );
    }

    /// #153 E1: 路径遍历拒绝现在前移到反序列化阶段（PersonaId::new 内调
    /// validate_id_segment）。原 `explicit_persona_id_rejects_path_traversal_at_pipeline_boundary`
    /// 测试覆盖的场景（pipeline 边界拒绝）不再可能——serde 阶段就拒绝，无法
    /// 构造出 Option<PersonaId> 含非法字符串的 ChatCompletionRequest。
    ///
    /// CodeRabbit #288 review: 本测试同时验证两条路径：
    /// 1. `PersonaId::new` 直接构造拒绝（覆盖内部 Rust API 调用路径）
    /// 2. `serde_json::from_str::<PersonaId>` 反序列化拒绝（覆盖 axum extractor
    ///    实际路径，确保 Deserialize impl 把错误转成 serde::de::Error）
    #[test]
    fn explicit_persona_id_rejects_path_traversal_at_deserialize_boundary() {
        // 路径 1: PersonaId::new 直接拒绝
        let bad = crate::types::PersonaId::new("../escape");
        assert!(
            matches!(bad, Err(crate::error::AirpError::BadRequest(_))),
            "expected BadRequest for path traversal, got {:?}",
            bad
        );

        // 其他非法字符同样拒绝
        let bad_nul = crate::types::PersonaId::new("a\0b");
        assert!(matches!(
            bad_nul,
            Err(crate::error::AirpError::BadRequest(_))
        ));
        let bad_slash = crate::types::PersonaId::new("a/b");
        assert!(matches!(
            bad_slash,
            Err(crate::error::AirpError::BadRequest(_))
        ));

        // 合法 id 仍然通过
        let ok = crate::types::PersonaId::new("writer").unwrap();
        assert_eq!(ok.as_str(), "writer");

        // 路径 2: serde_json 反序列化拒绝（axum extractor 实际路径）
        let bad_serde: Result<crate::types::PersonaId, _> = serde_json::from_str(r#""../escape""#);
        assert!(
            bad_serde.is_err(),
            "serde should reject path traversal, got {:?}",
            bad_serde
        );
        let bad_nul_serde: Result<crate::types::PersonaId, _> = serde_json::from_str(r#""a\0b""#);
        assert!(bad_nul_serde.is_err(), "serde should reject null byte");
        let bad_slash_serde: Result<crate::types::PersonaId, _> = serde_json::from_str(r#""a/b""#);
        assert!(bad_slash_serde.is_err(), "serde should reject slash");

        // 合法 id 反序列化通过
        let ok_serde: crate::types::PersonaId = serde_json::from_str(r#""writer""#).unwrap();
        assert_eq!(ok_serde.as_str(), "writer");

        // ChatCompletionRequest 反序列化路径也拒绝（端到端覆盖）
        let bad_req: Result<ChatCompletionRequest, _> =
            serde_json::from_str(r#"{"message":"hi","user_id":"alice","persona_id":"../escape"}"#);
        assert!(
            bad_req.is_err(),
            "ChatCompletionRequest serde should reject path traversal persona_id, got {:?}",
            bad_req
        );
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
        let (persona, source) = resolve_request_persona(&req, tmp.path()).unwrap();
        let persona = persona.unwrap();
        assert_eq!(persona.id, "adventurer");
        assert_eq!(persona.name, "Adventurer");
        assert_eq!(
            source,
            crate::orchestrator::trace::PersonaActivationSource::CharacterBinding,
            "character-scoped generic binding → CharacterBinding source"
        );
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
        let (persona, source) = resolve_request_persona(&req, tmp.path()).unwrap();
        let persona = persona.unwrap();
        assert_eq!(
            persona.id, "scoped-hero",
            "session-scoped binding must win over generic"
        );
        assert_eq!(
            source,
            crate::orchestrator::trace::PersonaActivationSource::SessionBinding,
            "session-scoped binding → SessionBinding source"
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
        let (persona, source) = resolve_request_persona(&req, tmp.path()).unwrap();
        let persona = persona.unwrap();
        assert_eq!(
            persona.id, "default",
            "scene mode must skip find_for_character"
        );
        assert_ne!(persona.name, "ShouldNotActivate");
        assert_eq!(
            source,
            crate::orchestrator::trace::PersonaActivationSource::Default,
            "scene mode skips binding → Default source"
        );
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
            fts: Default::default(),
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
            // CodeRabbit #288 review: 用 unwrap 而非 ok()，让 fixture 中的非法
            // persona_id 立即 panic，避免静默回退到 None 掩盖测试编写错误。
            persona_id: persona_id.map(|s| crate::types::PersonaId::new(s).unwrap()),
            swipe_candidates: Vec::new(),
            branch_from: None,
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
    fn failed_user_message_persistence_does_not_advance_timeline() {
        let tmp = tempdir().unwrap();
        crate::data_dir::ensure_data_dirs(tmp.path()).unwrap();
        let state = make_state(tmp.path().to_path_buf());
        let character = tmp.path().join("characters/hero");
        std::fs::create_dir_all(character.join("gating")).unwrap();
        let timeline = character.join("gating/timeline.md");
        std::fs::write(&timeline, "- 累计消耗时槽: 7\n").unwrap();
        std::fs::write(character.join("history"), b"blocks history directory").unwrap();

        let mut req = base_chat_request(None, None);
        req.character_id = Some(CharacterId::new("hero").unwrap());
        assert!(prepare_pipeline(&req, &state).is_err());
        assert_eq!(
            std::fs::read_to_string(timeline).unwrap(),
            "- 累计消耗时槽: 7\n"
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

    /// #114 e2e：验证 `prepare_pipeline` → `build_prompt_trace` → `EffectiveIds`
    /// 正确填充 #114 新增的 8 个字段（`persona_activation_source`, `persona_name`,
    /// `provider_source`, `model_source`, `temperature`, `temperature_source`,
    /// `max_tokens`, `max_tokens_source`）。单元测试 `tests_effective_config_summary`
    /// 只覆盖 `resolve_param_sources` 函数本身；此测试覆盖端到端传递。
    #[test]
    fn prepare_pipeline_populates_effective_config_summary_fields() {
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
                    variables: HashMap::new(),
                    id: "writer".to_string(),
                    bindings: Vec::new(),
                },
            )
            .unwrap();

        // 请求带 persona_id="writer" → activation_source=Explicit；
        // 所有 gen 参数 None → 从 snapshot 取 model/provider，temperature/max_tokens 无来源。
        let req = base_chat_request(Some("alice"), Some("writer"));
        let pipeline = prepare_pipeline(&req, &state).expect("pipeline should build");
        let eff = &pipeline.prompt_trace.effective;

        // persona 来源与名称
        assert_eq!(
            eff.persona_activation_source.as_deref(),
            Some("explicit"),
            "persona_id 在请求中显式指定 → Explicit"
        );
        assert_eq!(
            eff.persona_name.as_deref(),
            Some("Writer"),
            "persona_name 应为 persona 的显示名"
        );

        // provider/model 从 snapshot 取（请求未带）
        assert_eq!(
            eff.provider_source.as_deref(),
            Some("snapshot"),
            "请求未带 provider → snapshot"
        );
        assert_eq!(
            eff.model_source.as_deref(),
            Some("snapshot"),
            "请求未带 model 且无 preset → snapshot"
        );

        // temperature/max_tokens 无来源（请求与 preset 均未提供）
        assert!(
            eff.temperature_source.is_none(),
            "请求与 preset 均未提供 temperature → None"
        );
        assert!(
            eff.max_tokens_source.is_none(),
            "请求与 preset 均未提供 max_tokens → None"
        );
        assert!(eff.temperature.is_none());
        assert!(eff.max_tokens.is_none());
    }
}

// ── #114 effective config summary：参数来源标签 ──────────────────────────────
#[cfg(test)]
mod tests_effective_config_summary {
    use super::*;
    use crate::adapter::Provider;
    use crate::daemon::UserProfile;
    use crate::orchestrator::TavernPreset;
    use std::collections::HashMap;

    fn req_with(
        provider: Option<Provider>,
        model: Option<&str>,
        temp: Option<f32>,
        mt: Option<u32>,
    ) -> ChatCompletionRequest {
        ChatCompletionRequest {
            character_id: None,
            character_card_id: None,
            lorebook_path: None,
            user_profile: UserProfile {
                name: "U".to_string(),
                variables: HashMap::new(),
            },
            message: "hi".to_string(),
            messages_history: None,
            regex_filters: None,
            preset_id: None,
            enabled_presets: None,
            session_id: None,
            provider,
            endpoint: None,
            api_key: None,
            model: model.map(str::to_string),
            temperature: temp,
            max_tokens: mt,
            scene_id: None,
            user_id: None,
            persona_id: None,
            swipe_candidates: Vec::new(),
            branch_from: None,
        }
    }

    fn preset(temp: Option<f32>, mt: Option<u32>, model: Option<&str>) -> TavernPreset {
        TavernPreset {
            prompts: None,
            temperature: temp,
            max_tokens: mt,
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn provider_source_is_request_when_payload_overrides() {
        let req = req_with(Some(Provider::OpenAI), None, None, None);
        let sources = resolve_param_sources(&req, None);
        assert_eq!(sources.provider_source, Some("request"));
    }

    #[test]
    fn provider_source_is_snapshot_when_payload_omits() {
        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, None);
        assert_eq!(sources.provider_source, Some("snapshot"));
    }

    #[test]
    fn model_source_resolves_request_preset_snapshot_priority() {
        // request 显式 → request
        let req = req_with(None, Some("req-model"), None, None);
        let sources = resolve_param_sources(&req, Some(&preset(None, None, Some("preset-model"))));
        assert_eq!(sources.model_source, Some("request"));

        // request 缺，preset 有 → preset
        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, Some(&preset(None, None, Some("preset-model"))));
        assert_eq!(sources.model_source, Some("preset"));

        // 都缺 → snapshot
        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, None);
        assert_eq!(sources.model_source, Some("snapshot"));
    }

    #[test]
    fn temperature_source_resolves_request_over_preset() {
        let req = req_with(None, None, Some(0.5), None);
        let sources = resolve_param_sources(&req, Some(&preset(Some(0.9), None, None)));
        assert_eq!(sources.temperature, Some(0.5));
        assert_eq!(sources.temperature_source, Some("request"));

        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, Some(&preset(Some(0.9), None, None)));
        assert_eq!(sources.temperature, Some(0.9));
        assert_eq!(sources.temperature_source, Some("preset"));

        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, None);
        assert_eq!(sources.temperature, None);
        assert_eq!(sources.temperature_source, None);
    }

    #[test]
    fn max_tokens_source_resolves_request_over_preset() {
        let req = req_with(None, None, None, Some(123));
        let sources = resolve_param_sources(&req, Some(&preset(None, Some(999), None)));
        assert_eq!(sources.max_tokens, Some(123));
        assert_eq!(sources.max_tokens_source, Some("request"));

        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, Some(&preset(None, Some(999), None)));
        assert_eq!(sources.max_tokens, Some(999));
        assert_eq!(sources.max_tokens_source, Some("preset"));

        let req = req_with(None, None, None, None);
        let sources = resolve_param_sources(&req, None);
        assert_eq!(sources.max_tokens, None);
        assert_eq!(sources.max_tokens_source, None);
    }
}

/// #252 §2.B3：finalize 层端到端测试。
///
/// 验证 `run_finalize` 在 `stripped` 为空（模型只输出 `<state>` 块或纯空白）
/// 且 `swipe_candidates` 非空时，会回灌旧候选而非丢失用户资产（§2.B1 回归）。
/// 同时验证三条分支的完整契约：
///   1. stripped 空 + candidates 非空 → 原样回灌旧候选
///   2. stripped 非空 + candidates 非空 → 旧候选 + 新 stripped
///   3. stripped 空 + candidates 空 → 不创建 assistant 消息
///
/// `session_dir = None` 跳过卷副作用（封卷 / 维护 / 记忆抽取），让测试聚焦于
/// ChatLog 持久化分支。`provider_config` / `gen_params` / `http_client` 仍需构造
/// （FinalizerCtx 字段非 Option），但不会实际发起 HTTP 调用。
#[cfg(test)]
mod tests_b1_finalize_empty_stripped {
    use super::finalize::run_finalize;
    use super::*;
    use crate::adapter::{MessageRole, Provider};
    use crate::types::CharacterId;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// 构造一份最小可用的 `FinalizerCtx`，`session_dir = None` 跳过卷副作用。
    fn make_finalizer_ctx(
        data_root: PathBuf,
        character_id: Option<CharacterId>,
        swipe_candidates: Vec<String>,
    ) -> FinalizerCtx {
        FinalizerCtx {
            character_id,
            session_id: None,
            user_id: None,
            data_root,
            session_dir: None,
            provider_config: Arc::new(ProviderConfig {
                provider: Provider::OpenAI,
                endpoint: "https://example.test/v1/chat/completions".to_string(),
                api_key: Some("test-key".to_string()),
            }),
            gen_params: GenerationParams {
                model: "test-model".to_string(),
                temperature: None,
                max_tokens: None,
            },
            volume_config: VolumeConfig::default(),
            http_client: reqwest::Client::new(),
            continue_mode: false,
            swipe_candidates,
        }
    }

    /// 准备一个临时数据根 + 一个角色，写入 1 条 user 消息。
    /// 返回 (tempdir, data_root, character_id)——tempdir 必须存活到测试结束。
    fn setup_character_with_user_msg() -> (tempfile::TempDir, PathBuf, CharacterId) {
        let tmp = tempdir().unwrap();
        let data_root = tmp.path().to_path_buf();
        let character = CharacterId::new("finalize-char").unwrap();
        ChatService::new(&data_root)
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "hello".into(),
                },
            )
            .unwrap();
        (tmp, data_root, character)
    }

    /// §2.B1 回归核心：stripped 空 + swipe_candidates 非空 → 旧候选原样回灌。
    ///
    /// 场景：regen 时 `delete_last_n(1)` 已删除旧 assistant 消息 + 候选，
    /// 旧候选被捕获到 `swipe_candidates`。模型再生失败（只输出 `<state>` 块，
    /// stripped 后为空）。finalize 必须把旧候选写回，避免永久丢失。
    #[tokio::test]
    async fn finalize_empty_stripped_restores_old_candidates() {
        let (_tmp, data_root, character) = setup_character_with_user_msg();
        let ctx = make_finalizer_ctx(
            data_root.clone(),
            Some(character.clone()),
            vec!["old-reply-a".to_string(), "old-reply-b".to_string()],
        );
        // raw_acc / cleaned_acc 只含 <state> 块；extract_state_content 后 stripped 为空。
        let raw_acc = r#"<state>{"hp":100}</state>"#.to_string();
        let cleaned_acc = raw_acc.clone();

        run_finalize(ctx, raw_acc, cleaned_acc).await.unwrap();

        // 验证：chat log 有 1 条 user + 1 条 assistant，assistant 候选 = 旧候选原样回灌。
        let log = ChatService::new(&data_root)
            .history(&character, None)
            .unwrap();
        assert_eq!(log.messages.len(), 2, "should have user + assistant");
        assert_eq!(log.messages[1].role, MessageRole::Assistant);
        assert_eq!(
            log.message_candidates[1],
            vec!["old-reply-a".to_string(), "old-reply-b".to_string()],
            "old candidates must be restored verbatim, not lost"
        );
        assert_eq!(
            log.message_swipe_index[1], 1,
            "swipe_index should point to last restored candidate"
        );
        assert_eq!(
            log.messages[1].content, "old-reply-b",
            "content must match active candidate"
        );
    }

    /// 正向路径：stripped 非空 + swipe_candidates 非空 → 旧候选 + 新 stripped。
    ///
    /// 场景：regen 模型成功生成新回复，旧候选 + 新回复组成新候选列表。
    #[tokio::test]
    async fn finalize_non_empty_stripped_appends_to_candidates() {
        let (_tmp, data_root, character) = setup_character_with_user_msg();
        let ctx = make_finalizer_ctx(
            data_root.clone(),
            Some(character.clone()),
            vec!["old-reply-a".to_string(), "old-reply-b".to_string()],
        );
        let raw_acc = "new generated reply".to_string();
        let cleaned_acc = raw_acc.clone();

        run_finalize(ctx, raw_acc, cleaned_acc).await.unwrap();

        let log = ChatService::new(&data_root)
            .history(&character, None)
            .unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(
            log.message_candidates[1],
            vec![
                "old-reply-a".to_string(),
                "old-reply-b".to_string(),
                "new generated reply".to_string()
            ],
            "new stripped should be appended as last candidate"
        );
        assert_eq!(
            log.message_swipe_index[1], 2,
            "swipe_index should point to newly generated candidate"
        );
        assert_eq!(
            log.messages[1].content, "new generated reply",
            "content must match newly generated candidate"
        );
    }

    /// 防御性：stripped 空 + swipe_candidates 空 → 不创建 assistant 消息。
    ///
    /// 场景：普通 chat（非 regen），模型只输出 state 块，无旧候选可回灌。
    /// finalize 不应创建空 assistant 消息。
    #[tokio::test]
    async fn finalize_empty_stripped_no_candidates_no_message() {
        let (_tmp, data_root, character) = setup_character_with_user_msg();
        let ctx = make_finalizer_ctx(
            data_root.clone(),
            Some(character.clone()),
            Vec::new(), // 无旧候选
        );
        let raw_acc = r#"<state>{"hp":100}</state>"#.to_string();
        let cleaned_acc = raw_acc.clone();

        run_finalize(ctx, raw_acc, cleaned_acc).await.unwrap();

        let log = ChatService::new(&data_root)
            .history(&character, None)
            .unwrap();
        assert_eq!(
            log.messages.len(),
            1,
            "no assistant message should be created when stripped is empty and no candidates"
        );
    }

    /// 防御性：stripped 是纯空白（whitespace-only）+ swipe_candidates 非空
    /// → 应等同 stripped 空，走旧候选回灌分支。
    ///
    /// 场景：模型输出只含空白字符（"\n  \t"），extract_state_content 后 stripped 非空
    /// 但 trim 后为空。finalize.rs 用 `stripped.trim().is_empty()` 判断，应走回灌分支。
    #[tokio::test]
    async fn finalize_whitespace_stripped_restores_old_candidates() {
        let (_tmp, data_root, character) = setup_character_with_user_msg();
        let ctx = make_finalizer_ctx(
            data_root.clone(),
            Some(character.clone()),
            vec!["old-reply".to_string()],
        );
        // cleaned_acc 是纯空白，extract_state_content 不剥离任何内容，
        // 但 finalize.rs 的 `stripped.trim().is_empty()` 会判其为空。
        let raw_acc = "   \n\t  ".to_string();
        let cleaned_acc = raw_acc.clone();

        run_finalize(ctx, raw_acc, cleaned_acc).await.unwrap();

        let log = ChatService::new(&data_root)
            .history(&character, None)
            .unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(
            log.message_candidates[1],
            vec!["old-reply".to_string()],
            "whitespace-only stripped should restore old candidates, not create empty message"
        );
        assert_eq!(log.messages[1].content, "old-reply");
    }
}
