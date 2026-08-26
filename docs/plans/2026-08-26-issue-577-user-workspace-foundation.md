# #577 PR 10a — user workspace persistence foundation

Date: 2026-08-26  
Roadmap: #577 PR 10  
Status: implementation candidate

## Boundary

This first PR 10 slice establishes the machine contract and durable domain
asset before exposing workspace mutation through HTTP or Vue.

- Workspace v1 stores only layout nodes and Widget `id` / `type` declarations.
- Surface Widget props, Chat, Memory, Character State, Activity, bearer tokens,
  filesystem paths and executable UI content have no persistence field.
- Split ratios use bounded basis points; existing Surface limits remain the
  authority for identifiers, references, duplicates, depth, nodes, children
  and Widget count.
- Workspace revisions are canonical decimal strings on JSON boundaries, so
  browser clients cannot round a `u64` CAS token. Workspace v1 accepts only the
  explicit first-party Widget type allowlist rather than an open string channel
  that could persist paths or token-shaped payloads. Registry-backed extension
  types are deferred until Engine installation authority is wired in.
- The fixed `default` workspace lives below the already resolved effective
  root at `ui/workspaces/default`; requests cannot supply filesystem paths.
- Mutations use `expected_revision` CAS and immutable revisions with
  `current_revision` as the commit point. A rollback validates an old committed
  revision and writes its layout as a new forward revision.
- Unknown future schema majors remain losslessly raw-exportable as the exact
  manifest-approved UTF-8 JSON plus its SHA-256, but cannot be read,
  overwritten, reset or automatically migrated by this implementation.
- Legacy Blueprint migration is dry-run only and drops props/state/capability
  data rather than copying a second domain truth into the workspace.

## Explicitly deferred

- bearer-protected HTTP endpoints;
- `core.workspace` intent identity and command reducer;
- Surface projection consumption and Vue editing controls;
- import/apply migration;
- standalone `BackupScope::Workspace` objects or destructive restore;
- revision compaction and retention deletion.

## Verification target

- strict unknown-field and unknown-major rejection;
- Surface limit, reference, duplicate and split-ratio failures;
- same-revision concurrent CAS permits exactly one writer;
- corrupt/tampered revisions fail closed;
- orphan revision numbers are skipped and excluded from committed lineage;
- rollback creates a higher revision and can reach committed ancestors older
  than the 256-entry history response cap;
- export contains no Surface props or credentials;
- migration planning performs no writes and cannot retain Widget props.

Windows uses the repository's existing atomic revision primitive. File bytes
and pointer replacement are synchronized, but this slice does not claim
directory-metadata durability across Windows power loss because `sync_dir`
remains a documented no-op there.
