# #577 PR 10f-2 — Authenticated Workspace migration exercise

Date: 2026-08-28

Roadmap: #577 PR 10

Status: implementation candidate

## Scope

This slice exposes the PR 10f-1 Engine-domain transaction through a bounded,
bearer-protected HTTP contract. It accepts only the legacy Blueprint-v1 shape
and does not add arbitrary Workspace JSON import or a Vue recovery UI.

- `POST /v1/ui/workspace/migrations/blueprint-v1/dry-run` parses one strict
  Blueprint-v1 source and returns the deterministic migration plan. It performs
  no Workspace or backup write.
- `POST /v1/ui/workspace/migrations/blueprint-v1/apply` requires a decimal
  string Workspace revision and all three reviewed identities: source hash,
  candidate hash, and converter version. The domain layer recomputes and binds
  them before creating a verified backup or committing.
- `POST /v1/ui/workspace/migrations/rollback` requires a decimal string current
  revision and a migration backup ID. The backup is resolved only below the
  selected effective user root and restored as another forward revision.
- All three routes use the existing daemon bearer middleware, effective-root
  user scope, `no-store` responses, and a 256 KiB whole-request limit. The
  apply envelope also carries reviewed identities, so its source payload is
  necessarily slightly smaller than that outer limit.

The HTTP adapter does not accept numeric revisions, unknown request fields,
unknown Blueprint fields, JSON Patch, caller-selected paths, caller-selected
backup scope, or a client-provided candidate Workspace. Structured domain
errors retain the PR 10f-1 distinction between definite commit failure and
outcome unknown, exposing only the retained backup ID and recovery category.

## Request contracts

Dry-run:

```json
{
  "source": {
    "version": "legacy-demo",
    "layout": { "type": "dock", "areas": [] },
    "widgets": []
  }
}
```

Apply:

```json
{
  "expected_revision": "0",
  "source": {
    "version": "legacy-demo",
    "layout": { "type": "dock", "areas": [] },
    "widgets": []
  },
  "planned_source_sha256": "<dry-run value>",
  "planned_candidate_sha256": "<dry-run value>",
  "planned_converter_version": "<dry-run value>"
}
```

Backup rollback:

```json
{
  "expected_revision": "1",
  "backup_id": "<apply value>"
}
```

## Safety boundaries

- A stale CAS, malformed request, unknown source field, reviewed-identity
  mismatch, unsupported current Workspace major, or cross-root backup ID fails
  before mutating the selected Workspace.
- Dry-run and rejected apply requests do not leave backup objects.
- Apply returns the committed Workspace, retained recovery backup ID, and
  reviewed normalized typed-source hash. This is not a hash of the original
  uploaded bytes: insignificant JSON formatting and field order are removed by
  typed parsing and serialization. The client must not infer success from a
  dropped HTTP connection; it refreshes Workspace authority before choosing
  recovery.
- Rollback never invokes generic destructive restore and never moves
  `current_revision` backward.
- Revision-zero recovery remains explicit: rollback of the migration backup
  commits the deterministic default layout as a higher revision.

## Evidence target

- all routes require configured and valid bearer authentication;
- strict request shape and decimal-string revisions;
- 256 KiB route body limit;
- dry-run has no Workspace or backup side effects;
- reviewed apply returns a higher revision and a verifiable Workspace backup;
- stale CAS and identity mismatch leave revision and backup count unchanged;
- revision-zero apply and forward rollback exercise;
- effective-root isolation rejects another user's backup ID;
- tampered backup rollback fails without changing Workspace authority;
- future-major Workspace permits only the pre-existing raw export path;
- history records `workspace_migration_blueprint_v1` and
  `workspace_backup_rollback` provenance.

## Deferred

- arbitrary Workspace-v1 JSON import;
- Vue migration/recovery controls;
- internal recovery-backup retention and lineage policy (#622);
- multiple named Workspaces;
- untrusted external data-root writers (#618);
- Windows directory-metadata power-loss proof (#596).
