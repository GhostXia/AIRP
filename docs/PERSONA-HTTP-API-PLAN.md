# Multi-Persona HTTP API

> Status: A1a (PR #151), A1b (PR #152), A2a (PR #154), and the A2b/A2c WebUI closure (PR #180) delivered.
> This document records the stable HTTP/pipeline contract and the remaining
> product boundary; it is not an execution log. Last checked: 2026-07-16 at `main@13d07d7`.

## Scope

The plural `/v1/users/:user_id/personas` surface exposes the existing
`PersonaService` CRUD and binding operations. The legacy singular
`/v1/users/:user_id/persona` GET/PUT surface remains supported for the default
persona.

A2a provides WebUI CRUD. PR #180 adds automatic/explicit selection, character/session
bind/unbind controls, an observable effective-persona contract, and chat payload wiring.
Advanced Persona lifecycle work remains outside this stable HTTP/pipeline slice.

## Endpoints

| Method | Path | Input | Success |
| --- | --- | --- | --- |
| `GET` | `/v1/users/:user_id/personas` | none | `200` persona ID array, including `default` |
| `POST` | `/v1/users/:user_id/personas` | `CreatePersonaRequest` | `200` created `Persona` |
| `GET` | `/v1/users/:user_id/personas/:persona_id` | none | `200` `Persona` |
| `PUT` | `/v1/users/:user_id/personas/:persona_id` | `UpdateMultiPersonaRequest` | `200` updated `Persona` |
| `DELETE` | `/v1/users/:user_id/personas/:persona_id` | none | `204` |
| `POST` | `/v1/users/:user_id/personas/:persona_id/bindings` | `BindPersonaRequest` | `200` updated `Persona` |
| `DELETE` | `/v1/users/:user_id/personas/:persona_id/bindings` | `character_id` and optional `session_id` query | `200` updated `Persona` |
| `GET` | `/v1/users/:user_id/persona/effective` | `character_id` and optional `session_id` query | `200` effective `persona`, `source`, and `bindings.character_persona_id` / `bindings.session_persona_id` |

Create request:

```json
{
  "persona_id": "writer",
  "name": "Writer",
  "description": "optional",
  "variables": { "tone": "concise" }
}
```

Update request:

```json
{
  "expected_revision": 1,
  "name": "Writer",
  "description": "updated",
  "variables": { "tone": "precise" }
}
```

Binding request:

```json
{
  "character_id": "character-a",
  "session_id": "11111111-1111-4111-8111-111111111111"
}
```

## Invariants and errors

- IDs pass the existing path-segment validators. Path traversal is rejected.
- `default` is a case-insensitive reserved storage name. Create rejects every
  case variant; other service operations canonicalize variants to `default` so
  behavior is consistent on case-insensitive filesystems.
- A historical non-canonical file such as `Default.json` fails closed instead
  of being hidden or overwritten; an operator must resolve any conflict and
  rename it to `default.json`.
- Create uses expected revision `0`; an existing ID therefore produces a
  revision conflict instead of overwriting data.
- Update preserves bindings. Binding changes only through bind/unbind.
- Binding and unbinding are idempotent and do not bump revision on a no-op.
- A binding scope has at most one Persona owner. The save boundary checks all
  owners while holding the per-user lock, so concurrent binds cannot persist an
  ambiguous state. Effective resolution reads the complete owner set from one
  per-user snapshot; legacy/corrupt multi-owner data fails closed with `400`.
- Deleting `default` is rejected. Deleting a missing non-default persona is an
  idempotent `204`.
- Non-default GET/PUT/bind/unbind targets that do not exist return `404`.
- Client errors use the shared JSON envelope:
  `{"error":{"code":"bad_request","message":"..."}}`.

## Chat pipeline activation (A1b)

`POST /v1/chat/completions` accepts a new optional `persona_id` field. When
the request also carries `user_id`, the pipeline resolves a `Persona` and
merges its `name` / `variables` with the request `user_profile` before
prompt assembly. Resolution follows a documented precedence contract:

1. **Explicit `persona_id`** in the request body. The named persona must
   exist; otherwise the request fails with `404` (same as the plural GET
   contract). `default` is accepted and canonicalized case-insensitively.
2. **Bound persona** via `PersonaService::find_for_character`, using the
   request `character_id` and optional `session_id`. Exact session-scoped
   bindings win over generic per-character bindings. Skipped when the
   request uses `scene_id` (multi-character scenes do not have a single
   binding target; explicit `persona_id` is the only opt-in there).
3. **Default persona** via `PersonaService::get_default`. Returns the
   stored persona, or an in-memory `Persona::initial` snapshot when no
   file exists yet (no implicit disk write).

When `user_id` is absent, persona resolution is skipped entirely and the
request `user_profile` is used as-is — preserving the legacy single-user
contract bit-for-bit.

### Merge contract

| Field | Source |
| --- | --- |
| `name` (→ `{{user}}`) | Request `user_profile.name` if non-empty; otherwise persona `name`. |
| `variables` | Persona `variables` as defaults; request `user_profile.variables` overrides same-name keys. |

If multiple Personas match at the same binding precedence tier, resolution
fails with `400 bad_request` instead of silently selecting by filename order.
The merged variables are substituted in both single-character and scene system
prompts.

Rationale: the request body is the most recent client intent and must win
on explicit fields; the persona provides durable defaults (tone, persona
variables, etc.) without forcing the client to resend them every turn.
Clients that want the persona `name` to drive `{{user}}` must send an
empty `user_profile.name`; the WebUI uses this convention.

### Effective endpoint and WebUI selection

`GET /v1/users/:user_id/persona/effective` resolves binding → default and
returns `source=session_binding|character_binding|default` plus independent
`bindings.character_persona_id` and `bindings.session_persona_id` owners. The
two nested owner fields, not only the winning Persona, drive the corresponding
bind/unbind controls.

WebUI selection uses an empty internal value for "automatic": it always sends
the current `user_id` and omits `persona_id`, allowing the pipeline to resolve
bindings/default. A concrete selection sends `persona_id` explicitly. Automatic
mode is read-only for Persona save/delete, persists across refresh, and discards
stale effective responses after character/session changes.

### Scene-mode scope

`scene_id` requests resolve only precedence tiers 1 and 3 (explicit
`persona_id` or default). `find_for_character` is skipped because a scene
has multiple characters and no single binding target. Multi-character
persona binding semantics are deferred.

## Remaining work

- Define cross-session binding precedence for scene mode as part of any future
  multi-character Persona binding work.
- Add base lock, drift/history/rollback, avatar metadata, import/export and
  backup/restore under explicit versioned contracts.
- Build a unified effective configuration summary spanning Persona, Preset,
  provider/model and provenance.
