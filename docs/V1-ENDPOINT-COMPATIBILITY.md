# v1 endpoint compatibility retention

This is the repository-owned compatibility contract for `/v1` routes that are
currently not consumed by the WebUI.  `webui/tests/fixtures/v1-endpoints.json`
keeps the WebUI inventory and the retention decision together; this table keeps
the external-client promise reviewable instead of encoding it only in a prose
`reason`.

The owner is responsible for reviewing the route before the date in the table.
The route must not be removed or changed without updating this contract and the
client migration/compatibility plan.  The provenance for every row is the
route declaration in `engine/src/daemon/mod.rs` at the fixture's
`generatedFrom.ref` revision.

| Method | Path | Owner | Review after | Compatibility contract |
| --- | --- | --- | --- | --- |
| GET | `/v1/backups/:backup_id` | backup-service | 2026-11-08 | API clients may read a complete backup manifest. |
| GET | `/v1/characters/:character_id/avatar` | character-service | 2026-11-08 | Desktop and external clients may load the card avatar asset. |
| GET | `/v1/conversation-capabilities` | conversation-runtime | 2026-11-08 | Conversation clients discover the versioned execution contract. |
| GET | `/v1/conversation-migrations/:migration_id/export` | conversation-runtime | 2026-11-08 | Migration clients export a verified legacy source. |
| POST | `/v1/conversation-migrations/:migration_id/rollback` | conversation-runtime | 2026-11-08 | Migration clients may roll back an unchanged generated copy. |
| POST | `/v1/conversation-migrations/plan` | conversation-runtime | 2026-11-08 | Migration clients obtain a read-only migration plan. |
| POST | `/v1/conversation-migrations` | conversation-runtime | 2026-11-08 | Migration clients explicitly execute a verified plan. |
| GET | `/v1/conversation-policies` | conversation-runtime | 2026-11-08 | Conversation clients discover registered policy descriptors. |
| GET | `/v1/conversations/:conversation_id/events` | conversation-runtime | 2026-11-08 | Conversation clients read the append-only event stream. |
| POST | `/v1/conversations/:conversation_id/events` | conversation-runtime | 2026-11-08 | Conversation clients append an event. |
| POST | `/v1/conversations/:conversation_id/turns/:turn_id/cancel` | conversation-runtime | 2026-11-08 | Conversation clients request cooperative turn cancellation. |
| GET | `/v1/conversations/:conversation_id/turns/:turn_id/observability` | conversation-runtime | 2026-11-08 | Conversation clients read the redacted turn projection. |
| GET | `/v1/conversations/:conversation_id/turns/:turn_id` | conversation-runtime | 2026-11-08 | Conversation clients read a turn lifecycle projection. |
| POST | `/v1/conversations/:conversation_id/turns` | conversation-runtime | 2026-11-08 | Conversation clients execute and persist a turn. |
| GET | `/v1/conversations/:conversation_id` | conversation-runtime | 2026-11-08 | Conversation clients read a manifest. |
| GET | `/v1/conversations` | conversation-runtime | 2026-11-08 | Conversation clients list manifests. |
| POST | `/v1/conversations` | conversation-runtime | 2026-11-08 | Conversation clients create a manifest. |
| POST | `/v1/desktop-session` | desktop-shell | 2026-11-08 | The desktop shell exchanges its access key for a short-lived UI token. |
| GET | `/v1/extensions/:extension_id/grants` | extension-runtime | 2026-11-08 | API clients may query one extension's authoritative grant state. |
| POST | `/v1/presets/:preset_id/decompose` | preset-service | 2026-11-08 | API clients may generate a preset analysis sidecar. |
| GET | `/v1/provider-routing` | provider-routing | 2026-11-08 | API clients may read the persisted provider routing configuration. |
| POST | `/v1/scenes/:scene_id/conversations` | conversation-runtime | 2026-11-08 | Conversation clients create a scene snapshot adapter. |
| GET | `/v1/users/:user_id/persona/effective` | persona-service | 2026-11-08 | API clients may resolve the effective binding-to-default Persona. |
| GET | `/v1/users/:user_id/persona` | persona-service | 2026-11-08 | API clients retain the legacy default Persona read contract. |
| PUT | `/v1/users/:user_id/persona` | persona-service | 2026-11-08 | API clients retain the legacy default Persona write contract. |

The `/v1/conversations*` family is intentionally retained as the parallel
Conversation/legacy Chat contract.  WebUI zero-consumption is not permission
to delete or silently repurpose those routes.
