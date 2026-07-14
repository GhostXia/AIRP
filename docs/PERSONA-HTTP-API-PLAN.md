# Multi-Persona HTTP API

> Status: A1a delivered by PR #151. This document records the stable HTTP
> contract and remaining product boundary; it is not an execution log.

## Scope

The plural `/v1/users/:user_id/personas` surface exposes the existing
`PersonaService` CRUD and binding operations. The legacy singular
`/v1/users/:user_id/persona` GET/PUT surface remains supported for the default
persona.

This slice does not activate a persona during generation and does not add a
WebUI management surface. Those remain A1b and A2 respectively.

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
  "session_id": "session-a"
}
```

## Invariants and errors

- IDs pass the existing path-segment validators. Path traversal is rejected.
- `default` is a case-insensitive reserved storage name. Create rejects every
  case variant; other service operations canonicalize variants to `default` so
  behavior is consistent on case-insensitive filesystems.
- Create uses expected revision `0`; an existing ID therefore produces a
  revision conflict instead of overwriting data.
- Update preserves bindings. Binding changes only through bind/unbind.
- Binding and unbinding are idempotent and do not bump revision on a no-op.
- Deleting `default` is rejected. Deleting a missing non-default persona is an
  idempotent `204`.
- Non-default GET/PUT/bind/unbind targets that do not exist return `404`.
- Client errors use the shared JSON envelope:
  `{"error":{"code":"bad_request","message":"..."}}`.

## Remaining work

- A1b: resolve an explicit or bound persona in `chat_pipeline`, with a documented
  precedence contract.
- A2: add WebUI list/create/edit/delete/bind/unbind/switch flows after A1b is
  stable.
- Define cross-session binding precedence as part of the activation contract.
