//! Shared domain services used by HTTP, Tauri-facing pipelines, and Agent tools.
//!
//! Transport adapters must not implement their own persistence locking or
//! rollback semantics.  `ChatService` is the single boundary for chat/session
//! mutations and character deletion.

mod analysis;
mod chat;
pub(crate) mod lock_order;
mod locks;
mod lorebook;
mod persona;
mod plot;
mod state;
mod world_event;

pub use analysis::AnalysisService;
pub(crate) use chat::RegenSnapshot;
pub use chat::{ChatService, HistoryWindow, SwipeResponse, SWIPE_CANDIDATES_CAP};
pub(crate) use locks::{character_lock, session_lock, state_lock};
pub use lorebook::LorebookService;
pub use persona::{
    EffectivePersonaResolution, EffectivePersonaSource, Persona, PersonaBinding,
    PersonaRevisionConflict, PersonaService,
};
pub use plot::{PlotArc, PlotPhase, PlotService};
pub use state::{StateService, StateSnapshot};
pub use world_event::{WorldClock, WorldEvent, WorldEventService};

// ── ChatService 已提取至 `domain/chat.rs`（E-P1-1 slice 5）────────────────────
// 通过 `pub use chat::{...};` 重新导出，保持公共 API 不变。
// `RegenSnapshot` 为 `pub(crate)`，通过 `pub(crate) use chat::RegenSnapshot;` 重新导出。

// ── StateService 已提取至 `domain/state.rs`（E-P1-1 slice 3）──────────────────
// 通过 `pub use state::{StateService, StateSnapshot};` 重新导出，保持公共 API 不变。

// ── PersonaService 已提取至 `domain/persona.rs`（E-P1-1 slice 4）──────────────
// 通过 `pub use persona::{...};` 重新导出，保持公共 API 不变。

// ── WorldEventService 已提取至 `domain/world_event.rs`（E-P1-3 slice 1）────────
// 通过 `pub use world_event::{...};` 重新导出，保持公共 API 不变。
// agent/tools/world_event.rs 不再直接 replace_file/fs::write，写路径收口至 Service。

// ── PlotService 已提取至 `domain/plot.rs`（E-P1-3 slice 2）────────────────────
// 通过 `pub use plot::{PlotArc, PlotPhase, PlotService};` 重新导出，保持公共 API 不变。
// daemon/handlers/plot.rs 不再直接 replace_file/fs::write，写路径收口至 Service。
// 边界：PlotService 只管 plot_arc.json；live.json 的 plot_history 仍由 StateService::mutate 管。

// ── AnalysisService 已提取至 `domain/analysis.rs`（E-P1-3 P0）────────────────
// 通过 `pub use analysis::AnalysisService;` 重新导出，保持公共 API 不变。
// agent/tools/analysis.rs + daemon/decompose_handlers.rs 不再直接 tokio::fs::write，
// 写路径收口至 Service（原子 replace_file + character_lock 串行化）。
// 边界（审计 PR #431 Point 4）：Service 内部同步 std::fs，调用方在 async context
// 必须用 `tokio::task::spawn_blocking` 包装，避免相对原 tokio::fs::write 的性能回归。
// 已知 gap（审计 PR #431 Point 1）：character_lock 不检测语义冲突，last-write-wins
// 静默丢失风险由未来 revision 合同/CAS 解决。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{ChatMessage, MessageRole};
    use crate::data_dir;
    use crate::error::AirpError;
    use crate::revision::atomic::read_current_revision;
    use crate::types::{CharacterId, SessionId, UserId};
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    #[test]
    fn lorebook_read_migrates_v3_selective_without_losing_explicit_false() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("lore-v3").unwrap();
        let world_dir = data_dir::char_world_dir(tmp.path(), character.as_str()).unwrap();
        let path = world_dir.join("lorebook.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "entries": [
                    {"keys": ["a"], "content": "absent"},
                    {"keys": ["b"], "content": "legacy", "extensions": {"selective": true, "position": "before_char"}},
                    {"keys": ["c"], "content": "explicit", "selective": false, "extensions": {"selective": true}}
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let lorebook = LorebookService::new(tmp.path()).read(&character).unwrap();
        assert!(!lorebook.entries[0].selective);
        assert!(lorebook.entries[1].selective);
        assert!(!lorebook.entries[2].selective);
        assert!(lorebook.entries[2].extensions.is_none());
        assert_eq!(
            lorebook.entries[1]
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("position")),
            Some(&serde_json::json!("before_char"))
        );
        assert!(lorebook.entries.iter().all(|entry| entry
            .extensions
            .as_ref()
            .is_none_or(|extensions| !extensions.contains_key("selective"))));
    }

    #[test]
    fn append_and_rollback_share_one_session_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();

        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "one".into(),
                },
            )
            .unwrap();
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::Assistant,
                    content: "two".into(),
                },
            )
            .unwrap();

        let (log, dropped) = service.rollback(&character, None, 0).unwrap();
        assert_eq!(dropped, 1);
        assert_eq!(log.messages.len(), 1);
    }

    #[test]
    fn concurrent_appends_do_not_lose_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let service = Arc::new(ChatService::new(tmp.path()));
        let character = CharacterId::new("concurrent").unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let mut workers = Vec::new();

        for index in 0..8 {
            let service = service.clone();
            let character = character.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                service
                    .append(
                        &character,
                        None,
                        ChatMessage {
                            role: MessageRole::User,
                            content: format!("message-{index}"),
                        },
                    )
                    .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let log = service.history(&character, None).unwrap();
        assert_eq!(log.messages.len(), 8);
        let unique: std::collections::HashSet<_> = log
            .messages
            .iter()
            .map(|message| &message.content)
            .collect();
        assert_eq!(unique.len(), 8);
    }

    #[test]
    fn state_service_validates_schema_and_assigns_revisions() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("stateful").unwrap();
        let state_dir = data_dir::char_state_dir(tmp.path(), character.as_str());
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(
            state_dir.join("schema.json"),
            serde_json::to_vec(&serde_json::json!({
                "type": "object",
                "required": ["hp"],
                "additionalProperties": false,
                "properties": {"hp": {"type": "integer", "minimum": 0, "maximum": 100}}
            }))
            .unwrap(),
        )
        .unwrap();
        let service = StateService::new(tmp.path());

        let first = service
            .write(&character, &serde_json::json!({"hp": 80}))
            .unwrap();
        let second = service
            .write(&character, &serde_json::json!({"hp": 60}))
            .unwrap();
        assert_eq!((first.revision, second.revision), (1, 2));
        assert!(service
            .write(&character, &serde_json::json!({"hp": 101}))
            .is_err());
        let live: serde_json::Value =
            serde_json::from_slice(&fs::read(state_dir.join("live.json")).unwrap()).unwrap();
        assert_eq!(live["hp"], 60);
    }

    #[test]
    fn state_schema_without_properties_rejects_all_additional_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("closed").unwrap();
        let state_dir = data_dir::char_state_dir(tmp.path(), character.as_str());
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(
            state_dir.join("schema.json"),
            serde_json::to_vec(&serde_json::json!({
                "type": "object",
                "additionalProperties": false
            }))
            .unwrap(),
        )
        .unwrap();

        let error = StateService::new(tmp.path())
            .write(&character, &serde_json::json!({"unexpected": true}))
            .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
    }

    // `latest_revision_skips_a_large_invalid_trailing_line` 已随 `latest_revision`
    // 一同迁移至 `domain/state.rs` 的内嵌 `tests` 模块（E-P1-1 slice 3）。

    // ── PersonaService（#114）─────────────────────────────────────────────────────

    #[test]
    fn persona_get_returns_initial_when_not_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let persona = service.get_default(&uid, "User").unwrap();
        assert_eq!(
            persona.revision, 0,
            "non-existent persona returns revision 0"
        );
        assert_eq!(persona.name, "User", "default name fallback");
        assert!(persona.variables.is_empty());
        // 不写盘：persona.json 不应存在
        assert!(!crate::data_dir::user_persona_path(tmp.path(), &uid).exists());
    }

    #[test]
    fn persona_save_bumps_revision_and_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        let persona = Persona {
            schema: Persona::SCHEMA,
            revision: 0, // save 内 bump
            updated_at: String::new(),
            name: "Alice".to_string(),
            description: "a curious librarian".to_string(),
            variables: HashMap::from([("mood".to_string(), "curious".to_string())]),
            id: "default".to_string(),
            bindings: Vec::new(),
        };
        let saved = service.save_default(&uid, 0, persona).unwrap();
        assert_eq!(saved.revision, 1, "first save bumps 0 -> 1");
        assert_eq!(saved.name, "Alice");
        assert_eq!(saved.variables.get("mood").unwrap(), "curious");

        // 持久化：重新 get 应读回同一份
        let reread = service.get_default(&uid, "User").unwrap();
        assert_eq!(reread.revision, 1);
        assert_eq!(reread.name, "Alice");
        assert_eq!(reread.description, "a curious librarian");
    }

    #[test]
    fn persona_save_rejects_revision_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        let p1 = Persona::initial("Alice");
        service.save_default(&uid, 0, p1).unwrap(); // revision -> 1

        // 客户端仍持有 revision=0，服务端已 1 → 必须拒绝
        let p2 = Persona::initial("Alice-updated");
        let err = service.save_default(&uid, 0, p2).unwrap_err();
        let conflict: PersonaRevisionConflict = serde_json::from_str(match &err {
            AirpError::BadRequest(s) => s,
            _ => panic!("expected BadRequest with PersonaRevisionConflict JSON, got {err:?}"),
        })
        .unwrap();
        assert_eq!(
            conflict.current_revision, 1,
            "conflict payload must report server-side revision"
        );
    }

    #[test]
    fn persona_save_rejects_unsupported_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        // 手动写一份 schema=999 的 persona.json
        let dir = crate::data_dir::user_dir(tmp.path(), &uid);
        fs::create_dir_all(&dir).unwrap();
        let bad = serde_json::json!({
            "schema": 999,
            "revision": 5,
            "updated_at": "2026-07-11T00:00:00Z",
            "name": "bad",
            "description": "",
            "variables": {}
        });
        fs::write(
            crate::data_dir::user_persona_path(tmp.path(), &uid),
            serde_json::to_vec_pretty(&bad).unwrap(),
        )
        .unwrap();

        let err = service.get_default(&uid, "User").unwrap_err();
        assert!(
            matches!(err, AirpError::Internal(_)),
            "unsupported schema must be Internal, got {err:?}"
        );
    }

    #[test]
    fn persona_save_does_not_overwrite_corrupt_existing_data() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let path = crate::data_dir::user_persona_multi_path(tmp.path(), &uid, "default").unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not-json").unwrap();

        assert!(service
            .save_default(&uid, 0, Persona::initial("Alice"))
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not-json");
    }

    #[test]
    fn persona_multi_storage_rejects_traversal_and_preserves_legacy_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();

        let mut legacy = Persona::initial("Legacy");
        legacy.schema = 1;
        legacy.revision = 7;
        let legacy_path = crate::data_dir::user_persona_path(tmp.path(), &uid);
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let canonical =
            crate::data_dir::user_persona_multi_path(tmp.path(), &uid, "default").unwrap();
        assert!(!canonical.exists());

        let migrated = service.save(&uid, "default", 7, legacy).unwrap();
        assert_eq!(migrated.revision, 8);
        let canonical_persona: Persona =
            serde_json::from_slice(&fs::read(canonical).unwrap()).unwrap();
        let legacy_persona: Persona =
            serde_json::from_slice(&fs::read(legacy_path).unwrap()).unwrap();
        assert_eq!(canonical_persona.revision, 8);
        assert_eq!(legacy_persona.revision, 8);
        assert!(service.get(&uid, "../escape", "User").is_err());
        assert!(service
            .save(&uid, "..\\escape", 0, Persona::initial("Bad"))
            .is_err());
    }

    #[test]
    fn persona_list_always_contains_default_and_custom_get_requires_existing_data() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        service
            .save(&uid, "custom", 0, Persona::initial("Custom"))
            .unwrap();

        assert_eq!(service.list(&uid).unwrap(), vec!["custom", "default"]);
        assert!(matches!(
            service.get(&uid, "missing", "User"),
            Err(AirpError::NotFound(_))
        ));
    }

    #[test]
    fn persona_default_read_normalizes_stored_id() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let path = crate::data_dir::user_persona_path(tmp.path(), &uid);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut legacy = Persona::initial("Legacy");
        legacy.id = "Default".to_string();
        fs::write(path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        assert_eq!(service.get_default(&uid, "User").unwrap().id, "default");
    }

    #[test]
    fn persona_case_variant_default_file_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let dir = crate::data_dir::user_personas_dir(tmp.path(), &uid);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Default.json"), b"{}").unwrap();

        assert!(matches!(service.list(&uid), Err(AirpError::BadRequest(_))));
        assert!(matches!(
            service.get_default(&uid, "User"),
            Err(AirpError::BadRequest(_))
        ));
        assert!(matches!(
            service.save_default(&uid, 0, Persona::initial("Alice")),
            Err(AirpError::BadRequest(_))
        ));
    }

    #[test]
    fn persona_default_mirror_failure_rolls_back_canonical_write() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let canonical =
            crate::data_dir::user_persona_multi_path(tmp.path(), &uid, "default").unwrap();
        fs::create_dir_all(canonical.parent().unwrap()).unwrap();
        fs::write(
            &canonical,
            serde_json::to_vec_pretty(&Persona::initial("Before")).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(crate::data_dir::user_persona_path(tmp.path(), &uid)).unwrap();

        assert!(service
            .save_default(&uid, 0, Persona::initial("After"))
            .is_err());
        let persisted: Persona = serde_json::from_slice(&fs::read(canonical).unwrap()).unwrap();
        assert_eq!(persisted.revision, 0);
        assert_eq!(persisted.name, "Before");
    }

    #[test]
    fn persona_default_uses_newer_legacy_revision_and_resynchronizes_on_save() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let canonical = service
            .save_default(&uid, 0, Persona::initial("Canonical"))
            .unwrap();
        assert_eq!(canonical.revision, 1);

        let legacy_path = crate::data_dir::user_persona_path(tmp.path(), &uid);
        let mut legacy = Persona::initial("Legacy edit");
        legacy.revision = 2;
        fs::write(&legacy_path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let selected = service.get_default(&uid, "User").unwrap();
        assert_eq!(selected.revision, 2);
        assert_eq!(selected.name, "Legacy edit");
        let saved = service.save_default(&uid, 2, selected).unwrap();
        assert_eq!(saved.revision, 3);
        let canonical_path =
            crate::data_dir::user_persona_multi_path(tmp.path(), &uid, "default").unwrap();
        let canonical_after: Persona =
            serde_json::from_slice(&fs::read(canonical_path).unwrap()).unwrap();
        let legacy_after: Persona =
            serde_json::from_slice(&fs::read(legacy_path).unwrap()).unwrap();
        assert_eq!(canonical_after.revision, 3);
        assert_eq!(legacy_after.revision, 3);
    }

    #[test]
    fn persona_binding_prefers_session_and_idempotent_bind_does_not_bump_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let session = SessionId::new().to_string();

        service
            .save(&uid, "generic", 0, Persona::initial("Generic"))
            .unwrap();
        service
            .save(&uid, "specific", 0, Persona::initial("Specific"))
            .unwrap();
        let generic = service
            .bind(
                &uid,
                "generic",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: None,
                },
            )
            .unwrap();
        let unchanged = service
            .bind(
                &uid,
                "generic",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: None,
                },
            )
            .unwrap();
        assert_eq!(unchanged.revision, generic.revision);

        service
            .bind(
                &uid,
                "specific",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: Some(session.clone()),
                },
            )
            .unwrap();
        assert_eq!(
            service
                .find_for_character(&uid, "char-a", Some(&session))
                .unwrap(),
            Some("specific".to_string())
        );
        assert_eq!(
            service.find_for_character(&uid, "char-a", None).unwrap(),
            Some("generic".to_string())
        );
    }

    #[test]
    fn persona_binding_scope_has_one_atomic_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        for id in ["one", "two"] {
            service.save(&uid, id, 0, Persona::initial(id)).unwrap();
        }
        let binding = PersonaBinding {
            character_id: "char-a".to_string(),
            session_id: None,
        };
        service.bind(&uid, "one", binding.clone()).unwrap();

        let error = service.bind(&uid, "two", binding).unwrap_err();
        assert!(matches!(
            error,
            AirpError::BadRequest(message)
                if message.contains("already owned by one")
        ));
        assert!(service
            .get(&uid, "two", "User")
            .unwrap()
            .bindings
            .is_empty());
    }

    #[test]
    fn resolved_persona_deleted_before_read_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        service
            .save(&uid, "writer", 0, Persona::initial("Writer"))
            .unwrap();
        service
            .bind(
                &uid,
                "writer",
                PersonaBinding {
                    character_id: "char-a".to_string(),
                    session_id: None,
                },
            )
            .unwrap();

        let resolution = service
            .resolve_effective_persona(&uid, "char-a", None)
            .unwrap();
        let resolved_id = resolution.effective_persona_id.unwrap();
        service.delete(&uid, &resolved_id).unwrap();
        assert!(matches!(
            service.get(&uid, &resolved_id, "User"),
            Err(AirpError::NotFound(_))
        ));
    }

    #[test]
    fn persona_binding_ambiguity_fails_closed_at_each_precedence_tier() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("alice").unwrap();
        let session = SessionId::new().to_string();
        for id in ["one", "two"] {
            service.save(&uid, id, 0, Persona::initial(id)).unwrap();
        }
        service
            .bind(
                &uid,
                "one",
                PersonaBinding {
                    character_id: "generic-char".to_string(),
                    session_id: None,
                },
            )
            .unwrap();
        service
            .bind(
                &uid,
                "one",
                PersonaBinding {
                    character_id: "session-char".to_string(),
                    session_id: Some(session.clone()),
                },
            )
            .unwrap();

        // Seed legacy/corrupt persisted ambiguity directly. New saves reject
        // this state, while the resolver must still fail closed when reading it.
        let mut two = service.get(&uid, "two", "User").unwrap();
        two.bindings = vec![
            PersonaBinding {
                character_id: "generic-char".to_string(),
                session_id: None,
            },
            PersonaBinding {
                character_id: "session-char".to_string(),
                session_id: Some(session.clone()),
            },
        ];
        let two_path = data_dir::user_persona_multi_path(tmp.path(), &uid, "two").unwrap();
        fs::write(two_path, serde_json::to_vec_pretty(&two).unwrap()).unwrap();

        assert!(matches!(
            service.find_for_character(&uid, "generic-char", None),
            Err(AirpError::BadRequest(_))
        ));
        assert!(matches!(
            service.find_for_character(&uid, "session-char", Some(&session)),
            Err(AirpError::BadRequest(_))
        ));
    }

    // ── delete_session + session-scoped lifecycle（#35/#37）──────────────────────

    #[test]
    fn delete_session_removes_directory_and_is_not_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let sid = service.create_session(&character).unwrap();

        // append 一条消息到命名会话，确认目录非空
        service
            .append(
                &character,
                Some(&sid),
                ChatMessage {
                    role: MessageRole::User,
                    content: "hi".to_string(),
                },
            )
            .unwrap();
        let sessions_dir = tmp
            .path()
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(sid.to_string());
        assert!(
            sessions_dir.is_dir(),
            "session dir must exist before delete"
        );

        service.delete_session(&character, &sid, true).unwrap();
        assert!(
            !sessions_dir.exists(),
            "session dir must be gone after delete"
        );
        let listed = service.list_sessions(&character).unwrap();
        assert!(
            !listed.contains(&sid),
            "deleted session must not appear in list_sessions"
        );
    }

    #[test]
    fn delete_session_returns_not_found_for_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let unknown = SessionId::new();
        let err = service
            .delete_session(&character, &unknown, true)
            .unwrap_err();
        assert!(
            matches!(err, AirpError::NotFound(_)),
            "unknown session delete must be NotFound, got {err:?}"
        );
    }

    #[test]
    fn delete_session_retries_cleanup_after_tombstone_was_written() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let sid = service.create_session(&character).unwrap();
        let marker = tmp
            .path()
            .join("characters/alice/deleted_sessions")
            .join(sid.to_string());
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, []).unwrap();

        service.delete_session(&character, &sid, true).unwrap();

        assert!(marker.is_file());
        assert!(!tmp
            .path()
            .join("characters/alice/sessions")
            .join(sid.to_string())
            .exists());
    }

    /// #35/#37：命名会话与默认会话隔离——append 到命名会话不污染默认会话 history，
    /// 删除命名会话不影响默认会话。这是 WEBUI-MVP-PLAN §3.2"切换后不串流、串历史"
    /// 的最小可自动验收子集。
    #[test]
    fn named_session_isolated_from_default_and_delete_does_not_leak() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();

        // default session：2 条
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "default-1".to_string(),
                },
            )
            .unwrap();
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "default-2".to_string(),
                },
            )
            .unwrap();

        // named session A：3 条
        let sid_a = service.create_session(&character).unwrap();
        for content in ["a-1", "a-2", "a-3"] {
            service
                .append(
                    &character,
                    Some(&sid_a),
                    ChatMessage {
                        role: MessageRole::User,
                        content: content.to_string(),
                    },
                )
                .unwrap();
        }

        // 隔离断言：default history 不含 named 的消息
        let default_log = service.history(&character, None).unwrap();
        assert_eq!(
            default_log.messages.len(),
            2,
            "default session must keep its own 2 messages"
        );
        assert!(
            default_log
                .messages
                .iter()
                .all(|m| m.content.starts_with("default-")),
            "default session must not leak named session messages"
        );

        let named_log = service.history(&character, Some(&sid_a)).unwrap();
        assert_eq!(
            named_log.messages.len(),
            3,
            "named session A must keep its own 3 messages"
        );
        assert!(
            named_log
                .messages
                .iter()
                .all(|m| m.content.starts_with("a-")),
            "named session A must not leak default session messages"
        );

        // delete named A → default 不受影响
        service.delete_session(&character, &sid_a, true).unwrap();
        let default_log_after = service.history(&character, None).unwrap();
        assert_eq!(
            default_log_after.messages.len(),
            2,
            "default session must survive named session delete"
        );
        assert!(
            !service.list_sessions(&character).unwrap().contains(&sid_a),
            "deleted named session A must not appear in list_sessions"
        );
    }

    /// #35：delete_session 与 append 同时起跑。共享 session lock 必须保证每个 append
    /// 要么完整落盘，要么在 delete 的 tombstone 后返回 NotFound，不能半写或复活目录。
    #[test]
    fn delete_session_serializes_with_concurrent_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let service = Arc::new(ChatService::new(tmp.path()));
        let character = CharacterId::new("concurrent").unwrap();
        let sid = service.create_session(&character).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let mut workers = Vec::new();

        for index in 0..8 {
            let service = service.clone();
            let character = character.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                service.append(
                    &character,
                    Some(&sid),
                    ChatMessage {
                        role: MessageRole::User,
                        content: format!("message-{index}"),
                    },
                )
            }));
        }
        let delete_service = service.clone();
        let delete_character = character.clone();
        let delete_barrier = barrier.clone();
        let delete_worker = std::thread::spawn(move || {
            delete_barrier.wait();
            delete_service.delete_session(&delete_character, &sid, true)
        });
        for worker in workers {
            let result = worker.join().unwrap();
            assert!(
                result.is_ok() || matches!(result, Err(AirpError::NotFound(_))),
                "append racing delete must either commit or return NotFound, got {result:?}"
            );
        }
        delete_worker.join().unwrap().unwrap();
        assert!(
            !service.list_sessions(&character).unwrap().contains(&sid),
            "deleted concurrent session must not appear in list_sessions"
        );
        // delete 后再 append 到同一命名会话 → NotFound（目录被删，load_or_create 不复活命名会话）
        let err = service
            .append(
                &character,
                Some(&sid),
                ChatMessage {
                    role: MessageRole::User,
                    content: "post-delete".to_string(),
                },
            )
            .unwrap_err();
        assert!(
            matches!(err, AirpError::NotFound(_)),
            "append to deleted named session must be NotFound, got {err:?}"
        );
    }

    #[test]
    fn deleting_unknown_session_does_not_create_character() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("missing-character").unwrap();
        let sid = SessionId::new();

        let err = service.delete_session(&character, &sid, true).unwrap_err();
        assert!(matches!(err, AirpError::NotFound(_)));
        assert!(
            !tmp.path().join("characters/missing-character").exists(),
            "a failed delete must not create an empty character"
        );
    }

    // ── #342 E-P2-1：pre-delete backup ───────────────────────────────────────

    /// `delete_character(force=false)` 应创建 `PreDelete` + `Character` scoped backup。
    #[test]
    fn delete_character_creates_pre_delete_backup_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        // 创建角色目录 + card.json
        let char_dir = tmp.path().join("characters").join("alice");
        std::fs::create_dir_all(&char_dir).unwrap();
        std::fs::write(char_dir.join("card.json"), r#"{"name":"alice"}"#).unwrap();

        let backup_id = service.delete_character(&character, false).unwrap();
        assert!(backup_id.is_some(), "force=false 应创建 pre-delete backup");

        // 角色目录已被删
        assert!(!char_dir.exists());

        // backup 存在 + manifest 正确
        let manifest =
            crate::backup::read_backup_manifest(tmp.path(), backup_id.as_ref().unwrap()).unwrap();
        assert_eq!(manifest.source, crate::backup::BackupSource::PreDelete);
        assert_eq!(
            manifest.scope,
            crate::backup::BackupScope::Character {
                character_id: "alice".to_string()
            }
        );
        // backup files 应包含 card.json
        let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"characters/alice/card.json"));
    }

    /// `delete_character(force=true)` 跳过 pre-delete backup。
    #[test]
    fn delete_character_force_skips_pre_delete_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let char_dir = tmp.path().join("characters").join("alice");
        std::fs::create_dir_all(&char_dir).unwrap();
        std::fs::write(char_dir.join("card.json"), "{}").unwrap();

        let backup_id = service.delete_character(&character, true).unwrap();
        assert!(backup_id.is_none(), "force=true 应跳过 pre-delete backup");
        assert!(!char_dir.exists());

        // 不应存在任何 backup
        let listed = crate::backup::list_backups(tmp.path()).unwrap();
        assert!(listed.is_empty(), "force=true 不应留下 backup");
    }

    /// `delete_character(force=false)` 创建的 pre-delete backup 可被 restore 恢复。
    #[test]
    fn delete_character_pre_delete_backup_can_be_restored() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let char_dir = tmp.path().join("characters").join("alice");
        std::fs::create_dir_all(&char_dir).unwrap();
        std::fs::write(char_dir.join("card.json"), r#"{"name":"alice"}"#).unwrap();

        let backup_id = service
            .delete_character(&character, false)
            .unwrap()
            .unwrap();

        // restore 这个 pre-delete backup
        let (restored_from, _rollback_id) =
            crate::backup::restore_backup(tmp.path(), &backup_id).unwrap();
        assert_eq!(restored_from, backup_id);

        // 角色目录应被恢复
        assert!(char_dir.exists());
        let restored_card = std::fs::read_to_string(char_dir.join("card.json")).unwrap();
        assert_eq!(restored_card, r#"{"name":"alice"}"#);
    }

    /// `delete_session(force=false)` 应创建 `PreDelete` + `Session` scoped backup。
    #[test]
    fn delete_session_creates_pre_delete_backup_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let sid = service.create_session(&character).unwrap();

        // 写一条消息让 session 目录非空
        service
            .append(
                &character,
                Some(&sid),
                ChatMessage {
                    role: MessageRole::User,
                    content: "hello".into(),
                },
            )
            .unwrap();

        let backup_id = service.delete_session(&character, &sid, false).unwrap();
        assert!(backup_id.is_some(), "force=false 应创建 pre-delete backup");

        // session 目录已被删
        let session_dir = tmp
            .path()
            .join("characters/alice/sessions")
            .join(sid.to_string());
        assert!(!session_dir.exists());

        // backup 存在 + manifest 正确
        let manifest =
            crate::backup::read_backup_manifest(tmp.path(), backup_id.as_ref().unwrap()).unwrap();
        assert_eq!(manifest.source, crate::backup::BackupSource::PreDelete);
        match manifest.scope {
            crate::backup::BackupScope::Session {
                character_id,
                session_id,
            } => {
                assert_eq!(character_id, "alice");
                assert_eq!(session_id, sid.to_string());
            }
            other => panic!("expected Session scope, got {other:?}"),
        }
    }

    /// `delete_session(force=true)` 跳过 pre-delete backup。
    #[test]
    fn delete_session_force_skips_pre_delete_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("alice").unwrap();
        let sid = service.create_session(&character).unwrap();

        let backup_id = service.delete_session(&character, &sid, true).unwrap();
        assert!(backup_id.is_none(), "force=true 应跳过 pre-delete backup");

        let listed = crate::backup::list_backups(tmp.path()).unwrap();
        assert!(listed.is_empty(), "force=true 不应留下 backup");
    }

    // ── #37 durable message-id contract：cursor / rollback-by-ID 不变式 ──────

    fn seed_session_with_n(
        root: &Path,
        cid: &str,
        sid: Option<SessionId>,
        n: usize,
    ) -> (ChatService, CharacterId, Option<SessionId>) {
        let character = CharacterId::new(cid).unwrap();
        let session_id = sid;
        let service = ChatService::new(root);
        for i in 0..n {
            service
                .append(
                    &character,
                    session_id.as_ref(),
                    ChatMessage {
                        role: if i % 2 == 0 {
                            crate::adapter::MessageRole::User
                        } else {
                            crate::adapter::MessageRole::Assistant
                        },
                        content: format!("msg-{i}"),
                    },
                )
                .unwrap();
        }
        (service, character, session_id)
    }

    fn parse_sid(s: &str) -> SessionId {
        // 用固定 UUID 字符串做测试 sid，避免 SessionId::new() 的非确定性。
        SessionId::parse(s).unwrap()
    }

    #[test]
    fn history_window_limit_returns_tail_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "win_char", None, 10);

        // 取最近 4 条 → 应是 msg-6..msg-9，时间正序。
        let win = service
            .history_window(&character, session_id.as_ref(), Some(4), None)
            .unwrap();
        assert_eq!(win.messages.len(), 4);
        assert_eq!(win.messages[0].content, "msg-6");
        assert_eq!(win.messages[3].content, "msg-9");
        assert_eq!(win.total, 10);
        assert!(
            win.has_more,
            "loading tail of 10 with limit 4 must have more"
        );
        assert!(win.oldest_id.is_some());
    }

    #[test]
    fn history_window_before_cursor_returns_strictly_earlier() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "cursor_char", None, 10);

        // 取最近 4 条拿到 oldest_id 当 cursor。
        let tail = service
            .history_window(&character, session_id.as_ref(), Some(4), None)
            .unwrap();
        let cursor = tail.oldest_id.unwrap().to_ascii_lowercase();

        // before=cursor → 返回 cursor 严格之前（更早）的消息，limit 3。
        let earlier = service
            .history_window(&character, session_id.as_ref(), Some(3), Some(&cursor))
            .unwrap();
        assert_eq!(earlier.messages.len(), 3);
        // cursor 是 msg-6，更早 3 条 = msg-3..msg-5。
        assert_eq!(earlier.messages[0].content, "msg-3");
        assert_eq!(earlier.messages[2].content, "msg-5");
        assert!(earlier.has_more, "there are still earlier messages");
    }

    #[test]
    fn cursor_rejects_id_from_other_session() {
        let tmp = tempfile::tempdir().unwrap();
        // session A 拿一个真实 ID。
        let (svc_a, char_a, sess_a) = seed_session_with_n(
            tmp.path(),
            "cross_a",
            Some(parse_sid("550e8400-e29b-41d4-a716-446655440001")),
            3,
        );
        let log_a = svc_a.history(&char_a, sess_a.as_ref()).unwrap();
        let id_a = log_a.message_ids[0].clone();

        // session B 用 A 的 ID 当 cursor → BadRequest（cursor 不能跨 session）。
        let (svc_b, char_b, sess_b) = seed_session_with_n(
            tmp.path(),
            "cross_b",
            Some(parse_sid("550e8400-e29b-41d4-a716-446655440002")),
            3,
        );
        let err = svc_b
            .history_window(&char_b, sess_b.as_ref(), Some(2), Some(&id_a))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref msg) if msg.contains("not in this session")),
            "cross-session cursor must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn cursor_rejects_malformed_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) = seed_session_with_n(tmp.path(), "mal_char", None, 3);
        let err = service
            .history_window(&character, session_id.as_ref(), Some(2), Some("not-a-ulid"))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not a valid durable message id")),
            "malformed cursor must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn rollback_by_id_equivalent_to_by_index() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "rbid_char", None, 5);

        // index 2 的 ID → rollback_to_id(id_at_2) 应等价 rollback(2)：保留 0..=2 = 3 条。
        let log = service.history(&character, session_id.as_ref()).unwrap();
        let id_at_2 = log.message_ids[2].clone();

        let (log_after, dropped) = service
            .rollback_to_id(&character, session_id.as_ref(), &id_at_2)
            .unwrap();
        assert_eq!(dropped, 2, "rollback to index 2 drops 2 (total 5, kept 3)");
        assert_eq!(log_after.messages.len(), 3);
        assert_eq!(log_after.messages[2].content, "msg-2");

        // 不变量 6：同位置等价。
        let log_check = service.history(&character, session_id.as_ref()).unwrap();
        assert_eq!(log_check.messages.len(), 3);
    }

    #[test]
    fn rollback_by_id_rejects_unknown_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) =
            seed_session_with_n(tmp.path(), "rbid_unknown", None, 3);
        // 合形但不在 session 的 ID（派生一个不命中的）。
        let fake = crate::ulid::derive_legacy_id("some-other-scope", 99);
        let err = service
            .rollback_to_id(&character, session_id.as_ref(), &fake)
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not in this session")),
            "unknown message_id must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn rollback_by_id_rejects_malformed_id() {
        let tmp = tempfile::tempdir().unwrap();
        let (service, character, session_id) = seed_session_with_n(tmp.path(), "rbid_mal", None, 3);
        let err = service
            .rollback_to_id(&character, session_id.as_ref(), "not-a-ulid")
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not a valid durable message id")),
            "malformed message_id must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn rollback_validation_rejects_both_and_neither() {
        // 不变量 7 的 HTTP 入口校验：RollbackRequest.validate_rollback_target。
        use crate::daemon::RollbackRequest;
        use crate::types::CharacterId;
        let cid = CharacterId::new("vchar").unwrap();

        let both = RollbackRequest {
            character_id: cid.clone(),
            message_index: Some(2),
            message_id: Some("m0abc".to_string()),
            session_id: None,
        };
        assert!(both.validate_rollback_target().is_err());

        let neither = RollbackRequest {
            character_id: cid,
            message_index: None,
            message_id: None,
            session_id: None,
        };
        assert!(neither.validate_rollback_target().is_err());

        let ok_id = RollbackRequest {
            character_id: CharacterId::new("v2").unwrap(),
            message_index: None,
            message_id: Some("m0abc".to_string()),
            session_id: None,
        };
        assert!(ok_id.validate_rollback_target().is_ok());

        let ok_idx = RollbackRequest {
            character_id: CharacterId::new("v3").unwrap(),
            message_index: Some(2),
            message_id: None,
            session_id: None,
        };
        assert!(ok_idx.validate_rollback_target().is_ok());
    }

    #[test]
    fn concurrent_append_and_rollback_no_half_state() {
        // 不变量 7：with_session 串行化 → 并发 append/rollback 不产生半态。
        let tmp = tempfile::tempdir().unwrap();
        let cid = CharacterId::new("conc_char").unwrap();
        let sid = parse_sid("550e8400-e29b-41d4-a716-446655440010");
        let svc = ChatService::new(tmp.path());
        // 先种 5 条。
        for _ in 0..5 {
            svc.append(
                &cid,
                Some(&sid),
                ChatMessage {
                    role: crate::adapter::MessageRole::User,
                    content: "seed".to_string(),
                },
            )
            .unwrap();
        }

        let svc_arc = std::sync::Arc::new(svc);
        let mut handles = Vec::new();
        for i in 0..10 {
            let s = svc_arc.clone();
            let cidc = cid.clone();
            let sidc = sid;
            handles.push(std::thread::spawn(move || {
                if i % 2 == 0 {
                    s.append(
                        &cidc,
                        Some(&sidc),
                        ChatMessage {
                            role: crate::adapter::MessageRole::Assistant,
                            content: format!("concurrent-{i}"),
                        },
                    )
                } else {
                    // rollback 到 index 2（保留前 3）。
                    s.rollback(&cidc, Some(&sidc), 2)
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        // 不变量：最终态自洽——messages/ids/timestamps 等长，无半态。
        let final_log = svc_arc.history(&cid, Some(&sid)).unwrap();
        assert_eq!(
            final_log.messages.len(),
            final_log.message_ids.len(),
            "concurrent mutations must keep messages/ids equal length"
        );
        assert_eq!(
            final_log.messages.len(),
            final_log.message_timestamps.len(),
            "concurrent mutations must keep messages/timestamps equal length"
        );
    }

    // ── #115 Phase 2d/2e/2g：revision 合同接入测试 ──────────────────────────────

    /// Phase 2d：LorebookService::write 接入 revision 合同。
    /// 验证首次写入创建 revision 1 目录 + current_revision 文件，
    /// 第二次写入 bump 到 revision 2，旧 revision 目录保留不可变。
    ///
    /// CodeRabbit nitpick：v1/v2 必须用不同 entries，否则无法验证
    /// revision 1 的 lorebook.json 在 revision 2 写入后内容不变（不可变性）。
    #[test]
    fn lorebook_write_creates_revision_dir_and_bumps_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let service = LorebookService::new(tmp.path());
        let character = CharacterId::new("lore-rev").unwrap();
        let world_dir = data_dir::char_world_dir(tmp.path(), character.as_str()).unwrap();

        let lb_v1 = crate::orchestrator::Lorebook {
            entries: vec![crate::orchestrator::LorebookEntry {
                keys: vec!["剑".to_string()],
                content: "古老的铁剑".to_string(),
                enabled: Some(true),
                priority: Some(10),
                constant: None,
                comment: Some("v1".to_string()),
                secondary_keys: vec![],
                selective: false,
                case_sensitive: None,
                extensions: None,
            }],
        };
        service.write(&character, &lb_v1).unwrap();

        // 首次写入 → revision 1
        let revision_dir_v1 = world_dir.join("revisions").join("1");
        assert!(revision_dir_v1.is_dir(), "revision 1 目录应存在");
        assert!(revision_dir_v1.join("lorebook.json").is_file());
        assert!(revision_dir_v1.join("manifest.json").is_file());
        assert_eq!(
            read_current_revision(&world_dir).unwrap(),
            Some(1),
            "current_revision 应指向 1"
        );

        // 记录 revision 1 的 lorebook.json 字节，用于后续不可变性校验
        let v1_bytes = fs::read(revision_dir_v1.join("lorebook.json")).unwrap();

        // 第二次写入（不同 entries）→ revision 2
        let lb_v2 = crate::orchestrator::Lorebook {
            entries: vec![crate::orchestrator::LorebookEntry {
                keys: vec!["盾".to_string()],
                content: "镶金的圆盾".to_string(),
                enabled: Some(true),
                priority: Some(5),
                constant: None,
                comment: Some("v2".to_string()),
                secondary_keys: vec![],
                selective: false,
                case_sensitive: None,
                extensions: None,
            }],
        };
        service.write(&character, &lb_v2).unwrap();
        let revision_dir_v2 = world_dir.join("revisions").join("2");
        assert!(revision_dir_v2.is_dir(), "revision 2 目录应存在");
        assert!(revision_dir_v2.join("lorebook.json").is_file());
        assert!(revision_dir_v2.join("manifest.json").is_file());
        assert_eq!(
            read_current_revision(&world_dir).unwrap(),
            Some(2),
            "current_revision 应 bump 到 2"
        );

        // 旧 revision 目录保留不可变：revision 1 的 lorebook.json 内容不应被
        // revision 2 的写入覆盖。
        assert!(revision_dir_v1.is_dir(), "旧 revision 1 目录应保留不可变");
        let v1_bytes_after_v2 = fs::read(revision_dir_v1.join("lorebook.json")).unwrap();
        assert_eq!(
            v1_bytes, v1_bytes_after_v2,
            "revision 1 的 lorebook.json 在 revision 2 写入后应保持不变"
        );

        // legacy 工作副本仍存在（内容应等于 v2）
        let legacy_path = data_dir::char_world_lorebook_path(tmp.path(), character.as_str());
        assert!(legacy_path.is_file(), "legacy lorebook.json 工作副本应保留");
        let legacy_bytes = fs::read(&legacy_path).unwrap();
        let v2_bytes = fs::read(revision_dir_v2.join("lorebook.json")).unwrap();
        assert_eq!(
            legacy_bytes, v2_bytes,
            "legacy lorebook.json 应与最新 revision 2 内容一致"
        );
    }

    /// Phase 2e：StateService::write 接入 revision 合同。
    /// 验证首次写入创建 revision 1 目录 + current_revision 文件，
    /// 第二次写入 bump 到 revision 2，批准文件 state.json 内容与 live.json 对齐。
    #[test]
    fn state_write_creates_revision_dir_and_bumps_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let service = StateService::new(tmp.path());
        let character = CharacterId::new("state-rev").unwrap();
        let state_dir = data_dir::char_state_dir(tmp.path(), character.as_str());

        let first = service
            .write(&character, &serde_json::json!({"hp": 80}))
            .unwrap();
        assert_eq!(first.revision, 1);

        let revision_dir_v1 = state_dir.join("revisions").join("1");
        assert!(revision_dir_v1.is_dir(), "revision 1 目录应存在");
        assert!(revision_dir_v1.join("state.json").is_file());
        assert!(revision_dir_v1.join("manifest.json").is_file());
        assert_eq!(
            read_current_revision(&state_dir).unwrap(),
            Some(1),
            "current_revision 应指向 1"
        );

        // 批准文件 state.json 内容应与 live.json 对齐（只含 state 字段）
        let state_json_bytes = fs::read(revision_dir_v1.join("state.json")).unwrap();
        let live_json_bytes = fs::read(state_dir.join("live.json")).unwrap();
        assert_eq!(
            state_json_bytes, live_json_bytes,
            "state.json 应与 live.json 内容一致"
        );

        // 第二次写入 → revision 2
        let second = service
            .write(&character, &serde_json::json!({"hp": 60}))
            .unwrap();
        assert_eq!(second.revision, 2);

        let revision_dir_v2 = state_dir.join("revisions").join("2");
        assert!(revision_dir_v2.is_dir(), "revision 2 目录应存在");
        assert!(revision_dir_v2.join("state.json").is_file());
        assert_eq!(
            read_current_revision(&state_dir).unwrap(),
            Some(2),
            "current_revision 应 bump 到 2"
        );

        // 旧 revision 目录保留不可变
        assert!(revision_dir_v1.is_dir(), "旧 revision 1 目录应保留不可变");
    }

    /// Phase 2g：PersonaService::save 接入 revision 合同。
    /// 验证首次保存创建 revision 1 目录 + current_revision 文件，
    /// 第二次保存 bump 到 revision 2，legacy 工作副本 `personas/{pid}.json` 保留。
    #[test]
    fn persona_save_creates_revision_dir_and_bumps_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("paul").unwrap();
        let persona_asset_dir = data_dir::user_personas_dir(tmp.path(), &uid).join("default");

        let saved_v1 = service
            .save_default(&uid, 0, Persona::initial("Paul v1"))
            .unwrap();
        assert_eq!(saved_v1.revision, 1);

        // 首次保存 → revision 1
        let revision_dir_v1 = persona_asset_dir.join("revisions").join("1");
        assert!(revision_dir_v1.is_dir(), "revision 1 目录应存在");
        assert!(revision_dir_v1.join("persona.json").is_file());
        assert!(revision_dir_v1.join("manifest.json").is_file());
        assert_eq!(
            read_current_revision(&persona_asset_dir).unwrap(),
            Some(1),
            "current_revision 应指向 1"
        );

        // 第二次保存 → revision 2
        let saved_v2 = service
            .save_default(&uid, 1, Persona::initial("Paul v2"))
            .unwrap();
        assert_eq!(saved_v2.revision, 2);

        let revision_dir_v2 = persona_asset_dir.join("revisions").join("2");
        assert!(revision_dir_v2.is_dir(), "revision 2 目录应存在");
        assert!(revision_dir_v2.join("persona.json").is_file());
        assert_eq!(
            read_current_revision(&persona_asset_dir).unwrap(),
            Some(2),
            "current_revision 应 bump 到 2"
        );

        // 旧 revision 目录保留不可变
        assert!(revision_dir_v1.is_dir(), "旧 revision 1 目录应保留不可变");

        // legacy 工作副本 personas/default.json 仍存在
        let legacy_path = data_dir::user_persona_multi_path(tmp.path(), &uid, "default").unwrap();
        assert!(
            legacy_path.is_file(),
            "legacy personas/default.json 工作副本应保留"
        );

        // 批准文件 persona.json 内容应与 legacy 工作副本一致
        let revision_persona_bytes = fs::read(revision_dir_v2.join("persona.json")).unwrap();
        let legacy_persona_bytes = fs::read(&legacy_path).unwrap();
        assert_eq!(
            revision_persona_bytes, legacy_persona_bytes,
            "revision 内 persona.json 应与 legacy 工作副本内容一致"
        );
    }

    /// Phase 2g + Gemini #2：`PersonaService::delete` 应同时删除工作副本
    /// `personas/{pid}.json` 和 revision 目录 `users/{uid}/personas/{pid}/`，
    /// 避免后续以同 id 重建 Persona 时 `commit_revision` 因 `revisions/1`
    /// 已存在而失败。
    #[test]
    fn persona_delete_removes_revision_dir_and_allows_recreate() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("paul").unwrap();
        let pid = "p1";
        let persona_asset_dir = data_dir::user_personas_dir(tmp.path(), &uid).join(pid);
        let legacy_path = data_dir::user_persona_multi_path(tmp.path(), &uid, pid).unwrap();

        // 首次保存 → 创建 revision 1
        let saved_v1 = service
            .save(&uid, pid, 0, Persona::initial("Paul v1"))
            .unwrap();
        assert_eq!(saved_v1.revision, 1);
        assert!(legacy_path.is_file(), "工作副本应在保存后存在");
        assert!(persona_asset_dir.is_dir(), "revision 目录应在保存后存在");
        assert!(
            persona_asset_dir.join("revisions").join("1").is_dir(),
            "revision 1 目录应存在"
        );

        // 删除 → 工作副本与 revision 目录都应消失
        service.delete(&uid, pid).unwrap();
        assert!(!legacy_path.exists(), "工作副本应在 delete 后不存在");
        assert!(
            !persona_asset_dir.exists(),
            "revision 目录应在 delete 后被清理（Gemini #2）"
        );

        // 重新以同 id 保存 → 应能成功从 revision 1 重新开始（不冲突）
        let saved_v2 = service
            .save(&uid, pid, 0, Persona::initial("Paul v2"))
            .unwrap();
        assert_eq!(
            saved_v2.revision, 1,
            "重新创建后 revision 应从 1 开始（revision 目录被清理）"
        );
        assert!(
            persona_asset_dir.join("revisions").join("1").is_dir(),
            "重新创建后 revision 1 目录应再次存在"
        );
    }

    #[test]
    fn persona_delete_keeps_working_copy_when_revision_cleanup_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("paul").unwrap();
        let pid = "blocked";
        let legacy_path = data_dir::user_persona_multi_path(tmp.path(), &uid, pid).unwrap();
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, b"user asset").unwrap();

        let persona_asset_dir = data_dir::user_personas_dir(tmp.path(), &uid).join(pid);
        fs::create_dir_all(persona_asset_dir.parent().unwrap()).unwrap();
        fs::write(&persona_asset_dir, b"not a directory").unwrap();

        assert!(service.delete(&uid, pid).is_err());
        assert_eq!(fs::read(&legacy_path).unwrap(), b"user asset");
    }

    #[test]
    fn persona_delete_rejects_traversal_before_touching_user_data() {
        let tmp = tempfile::tempdir().unwrap();
        let service = PersonaService::new(tmp.path());
        let uid = UserId::new("paul").unwrap();
        let user_dir = data_dir::user_dir(tmp.path(), &uid);
        fs::create_dir_all(&user_dir).unwrap();
        let unrelated = user_dir.join("unrelated.txt");
        fs::write(&unrelated, b"keep me").unwrap();

        assert!(service.delete(&uid, "..").is_err());
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep me");
    }

    /// Lorebook orphan revision_dir 恢复测试。
    ///
    /// 模拟 `commit_revision` 第 5 步成功后崩溃（revision_dir 已 rename 但
    /// current_revision 指针未更新）：预先创建 orphan `revisions/2/` 空目录，
    /// 下次 `LorebookService::write` 应通过 `next_content_revision` 跳过 orphan，
    /// 使用 revision 3 而非与 orphan 冲突的 revision 2。
    #[test]
    fn lorebook_write_recovers_from_orphan_revision_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let service = LorebookService::new(tmp.path());
        let character = CharacterId::new("lore-orphan").unwrap();
        let world_dir = data_dir::char_world_dir(tmp.path(), character.as_str()).unwrap();

        // 第一次写入 → revision 1
        let lb_v1 = crate::orchestrator::Lorebook {
            entries: vec![crate::orchestrator::LorebookEntry {
                keys: vec!["剑".to_string()],
                content: "古老的铁剑".to_string(),
                enabled: Some(true),
                priority: Some(10),
                constant: None,
                comment: None,
                secondary_keys: vec![],
                selective: false,
                case_sensitive: None,
                extensions: None,
            }],
        };
        service.write(&character, &lb_v1).unwrap();
        assert_eq!(read_current_revision(&world_dir).unwrap(), Some(1));

        // 模拟 orphan：手动创建 revisions/2/ 空目录（current_revision 仍指向 1）
        std::fs::create_dir_all(world_dir.join("revisions").join("2")).unwrap();

        // 第二次写入应跳过 orphan 2，使用 revision 3
        let lb_v2 = crate::orchestrator::Lorebook {
            entries: vec![crate::orchestrator::LorebookEntry {
                keys: vec!["盾".to_string()],
                content: "镶金的圆盾".to_string(),
                enabled: Some(true),
                priority: Some(5),
                constant: None,
                comment: None,
                secondary_keys: vec![],
                selective: false,
                case_sensitive: None,
                extensions: None,
            }],
        };
        let result = service.write(&character, &lb_v2);
        assert!(
            result.is_ok(),
            "write 应跳过 orphan revisions/2/ 并使用 revision 3，实际: {:?}",
            result.err()
        );
        assert_eq!(
            read_current_revision(&world_dir).unwrap(),
            Some(3),
            "current_revision 应为 3（跳过 orphan 2）"
        );
        assert!(
            world_dir.join("revisions").join("3").is_dir(),
            "revision 3 目录应存在"
        );
        // orphan 目录应保留（不可变快照原则）
        assert!(
            world_dir.join("revisions").join("2").is_dir(),
            "orphan revisions/2/ 应保留不删除"
        );
    }

    // ── #249 Swipe 测试（审计 B3 修复）─────────────────────────────────────

    /// 辅助：创建带 1 条 user 消息的 session，返回 service。
    fn make_swipe_service() -> (tempfile::TempDir, ChatService, CharacterId) {
        let tmp = tempfile::tempdir().unwrap();
        let service = ChatService::new(tmp.path());
        let character = CharacterId::new("swipe-char").unwrap();
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "hello".into(),
                },
            )
            .unwrap();
        (tmp, service, character)
    }

    #[test]
    fn append_with_candidates_basic() {
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(
                &character,
                None,
                vec!["reply-a".to_string(), "reply-b".to_string()],
            )
            .unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(log.messages[1].content, "reply-b");
        assert_eq!(log.message_candidates[1], vec!["reply-a", "reply-b"]);
        assert_eq!(log.message_swipe_index[1], 1);
    }

    #[test]
    fn append_with_candidates_empty_rejected() {
        let (_tmp, service, character) = make_swipe_service();
        let err = service
            .append_with_candidates(&character, None, vec![])
            .err();
        assert!(err.is_some(), "empty candidates should be rejected");
    }

    #[test]
    fn append_with_candidates_all_whitespace_rejected() {
        // #252 D2: 全部空白候选（whitespace-only）应被拒绝。
        let (_tmp, service, character) = make_swipe_service();
        let err = service
            .append_with_candidates(
                &character,
                None,
                vec!["   ".to_string(), "\t\n".to_string(), "".to_string()],
            )
            .err();
        assert!(
            err.is_some(),
            "all-whitespace candidates should be rejected"
        );
        let msg = format!("{}", err.unwrap());
        assert!(
            msg.contains("all whitespace"),
            "error message should mention 'all whitespace', got: {msg}"
        );
    }

    #[test]
    fn append_with_candidates_partial_whitespace_filtered() {
        // #252 D2: 部分空白候选应被过滤，保留有效候选。
        // 场景：历史数据中旧候选含空白（理论上不应出现，但防御性处理）。
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(
                &character,
                None,
                vec![
                    "valid-a".to_string(),
                    "   ".to_string(),
                    "valid-b".to_string(),
                    "".to_string(),
                    "\t\n".to_string(),
                ],
            )
            .unwrap();
        // 过滤后应保留 2 个有效候选
        assert_eq!(
            log.message_candidates[1],
            vec!["valid-a".to_string(), "valid-b".to_string()],
            "whitespace candidates should be filtered out"
        );
        // swipe_index 指向最后一个有效候选（valid-b，索引 1）
        assert_eq!(log.message_swipe_index[1], 1);
        assert_eq!(log.messages[1].content, "valid-b");
    }

    #[test]
    fn append_with_candidates_single_whitespace_rejected() {
        // #252 D2: 单个空白候选应被拒绝（过滤后等价于全空白）。
        let (_tmp, service, character) = make_swipe_service();
        let err = service
            .append_with_candidates(&character, None, vec!["   ".to_string()])
            .err();
        assert!(
            err.is_some(),
            "single whitespace candidate should be rejected"
        );
    }

    #[test]
    fn append_with_candidates_preserves_non_trimmed_content() {
        // #252 D2: 候选内容前后空格应被保留（只过滤 trim 后为空的）。
        // 这是预期行为：候选可以含前后空格，只要 trim 后非空。
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(&character, None, vec!["  padded content  ".to_string()])
            .unwrap();
        assert_eq!(log.messages[1].content, "  padded content  ");
        assert_eq!(log.message_candidates[1][0], "  padded content  ");
        assert_eq!(log.message_swipe_index[1], 0);
    }

    #[test]
    fn append_with_candidates_single_works() {
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(&character, None, vec!["only".to_string()])
            .unwrap();
        assert_eq!(log.messages[1].content, "only");
        assert_eq!(log.message_candidates[1], vec!["only"]);
        assert_eq!(log.message_swipe_index[1], 0);
    }

    #[test]
    fn append_with_candidates_cap_drops_oldest() {
        let (_tmp, service, character) = make_swipe_service();
        let mut cands: Vec<String> = (0..(SWIPE_CANDIDATES_CAP + 5))
            .map(|i| format!("reply-{i}"))
            .collect();
        let expected_last = cands.last().unwrap().clone();
        let expected_cands: Vec<String> = cands.drain(5..).collect();
        let log = service
            .append_with_candidates(
                &character,
                None,
                (0..(SWIPE_CANDIDATES_CAP + 5))
                    .map(|i| format!("reply-{i}"))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(log.message_candidates[1].len(), SWIPE_CANDIDATES_CAP);
        assert_eq!(log.message_candidates[1], expected_cands);
        assert_eq!(log.messages[1].content, expected_last);
        assert_eq!(log.message_swipe_index[1], SWIPE_CANDIDATES_CAP - 1);
    }

    #[test]
    fn switch_swipe_updates_content_and_index() {
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(
                &character,
                None,
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
            )
            .unwrap();
        let msg_id = log.message_ids[1].clone();
        // #252 D3：switch_swipe 返回 SwipeResponse 增量响应。
        let switched = service.switch_swipe(&character, None, &msg_id, 0).unwrap();
        assert_eq!(switched.content, "a");
        assert_eq!(switched.index, 0);
        assert_eq!(switched.candidates_count, 3);
        assert_eq!(switched.message_id, msg_id);
        let switched2 = service.switch_swipe(&character, None, &msg_id, 2).unwrap();
        assert_eq!(switched2.content, "c");
        assert_eq!(switched2.index, 2);
        assert_eq!(switched2.candidates_count, 3);
    }

    #[test]
    fn switch_swipe_invalid_id_rejected() {
        let (_tmp, service, character) = make_swipe_service();
        service
            .append_with_candidates(&character, None, vec!["x".to_string()])
            .unwrap();
        let err = service
            .switch_swipe(&character, None, "not-a-valid-id", 0)
            .err();
        assert!(err.is_some(), "invalid message_id should be rejected");
    }

    #[test]
    fn switch_swipe_index_out_of_range_rejected() {
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(&character, None, vec!["a".to_string(), "b".to_string()])
            .unwrap();
        let msg_id = log.message_ids[1].clone();
        let err = service.switch_swipe(&character, None, &msg_id, 5).err();
        assert!(err.is_some(), "out-of-range index should be rejected");
    }

    #[test]
    fn switch_swipe_message_without_candidates_rejected() {
        let (_tmp, service, character) = make_swipe_service();
        // user 消息无候选
        let msg_id = service.history(&character, None).unwrap().message_ids[0].clone();
        let err = service.switch_swipe(&character, None, &msg_id, 0).err();
        assert!(err.is_some(), "switch on no-candidate message should fail");
    }

    #[test]
    fn regen_snapshot_preserves_durable_message_until_commit() {
        let (_tmp, service, character) = make_swipe_service();
        let log = service
            .append_with_candidates(
                &character,
                None,
                vec!["old-a".to_string(), "old-b".to_string()],
            )
            .unwrap();
        let snapshot = service
            .regen_snapshot(&character, None, "generation-1".to_string())
            .unwrap();
        let unchanged = service.history(&character, None).unwrap();
        assert_eq!(unchanged.message_ids[1], log.message_ids[1]);
        assert_eq!(unchanged.messages[1].content, "old-b");
        assert_eq!(snapshot.candidates, vec!["old-a", "old-b"]);

        let committed = service
            .commit_regen(&character, None, &snapshot, "new-c")
            .unwrap();
        assert_eq!(committed.messages.len(), 2);
        assert_eq!(committed.message_ids[1], log.message_ids[1]);
        assert_eq!(
            committed.message_candidates[1],
            vec!["old-a", "old-b", "new-c"]
        );
        assert_eq!(committed.messages[1].content, "new-c");
    }

    #[test]
    fn regen_commit_rejects_stale_snapshot_without_overwriting_history() {
        let (_tmp, service, character) = make_swipe_service();
        service
            .append_with_candidates(&character, None, vec!["old".to_string()])
            .unwrap();
        let snapshot = service
            .regen_snapshot(&character, None, "generation-1".to_string())
            .unwrap();
        service
            .append(
                &character,
                None,
                ChatMessage {
                    role: MessageRole::User,
                    content: "a concurrent edit".to_string(),
                },
            )
            .unwrap();

        assert!(matches!(
            service.commit_regen(&character, None, &snapshot, "new"),
            Err(AirpError::Conflict(_))
        ));
        let history = service.history(&character, None).unwrap();
        assert_eq!(
            history.messages.last().unwrap().content,
            "a concurrent edit"
        );
    }

    #[test]
    fn regen_commit_preserves_explicit_single_candidate_representation() {
        let (_tmp, service, character) = make_swipe_service();
        service
            .append_with_candidates(&character, None, vec!["old".to_string()])
            .unwrap();
        let snapshot = service
            .regen_snapshot(&character, None, "generation-1".to_string())
            .unwrap();
        assert_eq!(snapshot.stored_candidates, vec!["old"]);
        let committed = service
            .commit_regen(&character, None, &snapshot, "new")
            .unwrap();
        assert_eq!(committed.message_candidates[1], vec!["old", "new"]);
    }

    // ── PR #270 audit M2/M3: domain-level branch behavior ─────────────────

    #[test]
    fn append_with_branch_creates_branch_from_arbitrary_message() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let (service, character, session_id) = seed_session_with_n(root, "br_char", None, 3);
        // msg-0 (user), msg-1 (assistant), msg-2 (user). 主线 leaf = msg-2.

        // 从 msg-0 分叉一条新 user 消息。
        let log_before = service.history(&character, session_id.as_ref()).unwrap();
        let fork_id = log_before.message_ids[0].clone();
        service
            .append_with_branch(
                &character,
                session_id.as_ref(),
                ChatMessage {
                    role: MessageRole::User,
                    content: "branch-msg".into(),
                },
                Some(fork_id.clone()),
            )
            .unwrap();

        // 验证：内存态多了 1 条消息，且 parent = fork_id。
        let log_after = service.history(&character, session_id.as_ref()).unwrap();
        assert_eq!(log_after.messages.len(), 4);
        let branch_msg_idx = log_after
            .messages
            .iter()
            .position(|m| m.content == "branch-msg")
            .unwrap();
        assert_eq!(
            log_after.message_parents[branch_msg_idx].as_deref(),
            Some(fork_id.as_str()),
            "branch_from sets parent correctly"
        );
        assert_eq!(
            log_after.active_leaf.as_deref(),
            Some(log_after.message_ids[branch_msg_idx].as_str()),
            "active_leaf moved to new branch leaf"
        );
    }

    #[test]
    fn append_with_branch_rejects_unknown_branch_from() {
        // B6 修复：branch_from ID 不存在时必须 BadRequest。
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let (service, character, session_id) = seed_session_with_n(root, "br_unknown", None, 2);
        let fake = crate::ulid::derive_legacy_id("other-scope", 99);
        let err = service
            .append_with_branch(
                &character,
                session_id.as_ref(),
                ChatMessage {
                    role: MessageRole::User,
                    content: "x".into(),
                },
                Some(fake),
            )
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not found")),
            "unknown branch_from must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn history_window_filters_to_active_branch() {
        // B5 修复：history_window 必须按 active path 过滤，不能混入 sibling 分支消息。
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let (service, character, session_id) = seed_session_with_n(root, "hw_branch", None, 3);
        // 主线: msg-0, msg-1, msg-2. 现在从 msg-0 分叉。
        let log_before = service.history(&character, session_id.as_ref()).unwrap();
        let fork_id = log_before.message_ids[0].clone();
        service
            .append_with_branch(
                &character,
                session_id.as_ref(),
                ChatMessage {
                    role: MessageRole::User,
                    content: "branch-B".into(),
                },
                Some(fork_id),
            )
            .unwrap();

        // active_leaf 现在指向 branch-B（新分叉），active_path = [msg-0, branch-B]。
        // history_window(limit=10) 应只返回 2 条，不能包含 msg-1 / msg-2。
        let win = service
            .history_window(&character, session_id.as_ref(), Some(10), None)
            .unwrap();
        let contents: Vec<_> = win.messages.iter().map(|m| m.content.clone()).collect();
        assert_eq!(
            contents,
            vec!["msg-0", "branch-B"],
            "history_window must filter to active branch"
        );
        assert_eq!(
            win.total, 2,
            "total counts active path length, not physical"
        );
    }

    #[test]
    fn history_window_cursor_rejects_id_on_inactive_branch() {
        // B5 新 contract：cursor 在 session 中存在但不在 active path 上 → BadRequest。
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let (service, character, session_id) = seed_session_with_n(root, "hw_cursor", None, 3);
        // 主线: msg-0, msg-1, msg-2. 记下 msg-1 的 ID（之后会变成 inactive 分支）。
        let log_before = service.history(&character, session_id.as_ref()).unwrap();
        let msg1_id = log_before.message_ids[1].clone();
        let fork_id = log_before.message_ids[0].clone();
        // 从 msg-0 分叉，使 msg-1 / msg-2 变成 inactive 分支。
        service
            .append_with_branch(
                &character,
                session_id.as_ref(),
                ChatMessage {
                    role: MessageRole::User,
                    content: "branch-B".into(),
                },
                Some(fork_id),
            )
            .unwrap();

        // 用 msg1_id 当 cursor：它在 session 中存在，但不在 active path 上。
        let err = service
            .history_window(&character, session_id.as_ref(), Some(2), Some(&msg1_id))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not on active branch")),
            "cursor on inactive branch must be BadRequest, got {err:?}"
        );
    }

    #[test]
    fn history_window_cursor_rejects_id_from_other_session_still_works() {
        // B5 修复后，cross-session cursor 仍应被拒绝（保留原 contract）。
        // 这条是回归测试，覆盖 B5 修复时引入的 in_session 检查。
        let tmp = tempfile::tempdir().unwrap();
        let (svc_a, char_a, sess_a) = seed_session_with_n(
            tmp.path(),
            "cross_a_v2",
            Some(parse_sid("550e8400-e29b-41d4-a716-446655440001")),
            3,
        );
        let log_a = svc_a.history(&char_a, sess_a.as_ref()).unwrap();
        let id_a = log_a.message_ids[0].clone();

        let (svc_b, char_b, sess_b) = seed_session_with_n(
            tmp.path(),
            "cross_b_v2",
            Some(parse_sid("550e8400-e29b-41d4-a716-446655440002")),
            3,
        );
        let err = svc_b
            .history_window(&char_b, sess_b.as_ref(), Some(2), Some(&id_a))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::BadRequest(ref m) if m.contains("not in this session")),
            "cross-session cursor must be BadRequest with 'not in this session', got {err:?}"
        );
    }
}
