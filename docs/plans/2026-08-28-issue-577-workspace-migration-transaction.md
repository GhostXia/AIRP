# #577 PR 10f-1 — Workspace migration transaction and verified backup

Date: 2026-08-28

Roadmap: #577 PR 10

Status: implementation candidate

## Scope

This slice closes the Engine-domain safety boundary required before exposing
Blueprint-v1 migration apply over HTTP. It does not add an import endpoint or
accept arbitrary Workspace JSON.

- `BackupSource::PreMigration` records migration provenance.
- Internal `BackupScope::Workspace { revision }` always resolves to
  `ui/workspaces/default`; callers cannot choose a path and the public backup
  create API cannot request this scope.
- Migration apply recomputes the dry-run candidate, source hash, candidate hash,
  and converter version; checks all reviewed identities plus current Workspace
  CAS; validates any current asset; creates and verifies a Workspace-only
  backup; then commits the candidate as a higher immutable revision.
- A migration backup is rolled back by reading its verified, manifest-approved
  layout and committing another higher Workspace revision. Generic destructive
  backup restore rejects Workspace scope. Full restore rejects a target manifest
  containing Workspace assets and its swap algorithm always preserves the live
  `ui/workspaces/` subtree, including one created concurrently after preflight.
- Revision zero is explicit: the pre-migration backup represents no committed
  Workspace, and rollback commits the deterministic default layout forward.

Lock order is `WORKSPACE_LOCK -> BACKUP_LOCK -> revision COMMIT_LOCK`. One
`BACKUP_LOCK` lifetime spans migration backup creation, verification, and the
forward commit, preventing public backup deletion from opening a TOCTOU window.
A stale CAS or reviewed identity mismatch creates neither a backup nor a
revision. If commit reports an error after backup verification, apply re-reads
the authoritative pointer while both locks remain held. A verified matching
published candidate is returned as success only after renewing the revision,
pointer-file, and every ancestor-directory durability barrier up to the stable
effective data root; an unchanged pointer
returns a structured definite-failure error; unreadable, contradictory, or
barrier-failing authority returns a distinct outcome-unknown error that requires
refresh before recovery. Migration-backup rollback uses the same coordinator.

Before rename, backup publication enumerates the complete staging directory tree
and syncs every directory deepest-first; after rename it syncs both `backups/`
and its data-root parent. This covers nested copied paths and the first-backup
directory entry on Unix before migration proceeds.

Generic restore uses the same deepest-first barrier for its staging tree. Full
restore syncs changed `ui/` entries and then the data root after top-level swap;
scoped restore syncs the destination's direct parent through the canonical data
root bottom-up after rename.

The manifest schema remains v1. New readers accept old v1 manifests. A previous
engine does not understand the new `pre_migration` source or `workspace` scope,
so downgrade readers fail closed or omit those backups rather than restoring
them. Workspace recovery therefore requires the same or a newer engine.

## Evidence

Focused tests cover:

- fixed-scope manifest roundtrip and transparent list/get summary;
- Workspace-only file capture and full backup verification;
- generic restore rejection before rollback backup or swap;
- reviewed source/candidate/converter identity and stale-CAS no-write behavior;
- verified backup before migration forward commit;
- atomic commit failpoints at the real post-revision/pre-pointer and
  post-pointer/pre-final-sync boundaries, covering definite failure and renewed
  durability reconciliation for both apply and backup rollback;
- an unreadable-pointer reconciliation test producing a structured
  outcome-unknown response;
- ordered ancestor-sync evidence plus injected renewed-barrier failure producing
  outcome unknown;
- deepest-first backup staging-directory barrier ordering;
- deepest-first restore staging plus scoped destination-parent sync ordering;
- Full restore rejection for target Workspace assets plus path-level preservation
  of a live Workspace subtree;
- revision-zero rollback to the deterministic default;
- committed revision backup rollback restoring the exact old layout as a
  higher revision;
- post-verification file hash/length recheck before accepting backup bytes.

## Deferred

- bearer-protected dry-run/apply/backup-rollback HTTP endpoints;
- strict raw-byte legacy Blueprint parsing and request-size contract;
- user-scope negative HTTP tests and tampered-backup endpoint tests;
- Vue recovery/import UI;
- arbitrary Workspace-v1 JSON import;
- multiple named Workspaces.
