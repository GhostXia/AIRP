# #577 PR 10b — authenticated workspace HTTP and first command

Date: 2026-08-26  
Roadmap: #577 PR 10  
Status: implementation candidate

## Delivered boundary

- Every Workspace endpoint fails closed when daemon bearer authentication is
  not configured. Existing access-key and desktop-session bearer validation
  remains authoritative.
- Routes expose read, bounded committed history, lossless raw export, forward
  rollback, and one Engine-owned `resize_split` command.
- Mutations accept canonical decimal-string revisions. The server never accepts
  a replacement Workspace document, client JSON Patch, path, workspace ID, or
  client-asserted Widget identity.
- The command reducer resolves the split inside the current validated layout,
  applies one bounded ratio change, validates the complete candidate, and then
  uses the existing immutable-revision CAS commit.
- Revision conflict and future-major responses have stable machine codes and
  bounded fields. Internal manifest, path, parse, and I/O details remain hidden.
- Export returns the exact manifest-approved JSON text with schema/hash headers,
  fixed filename, and `Cache-Control: no-store`.
- `HttpEngineBus` uses the same bearer/renewal path as Surface traffic, preserves
  export text without parsing, and never automatically replays a 409 mutation.

`user_id` selects the existing validated effective-root namespace. The daemon
bearer is not user-bound, so this is storage isolation rather than multi-tenant
authorization.

## Explicitly deferred

- Vue Workspace store, editor controls, and loading the saved layout into a
  session Surface;
- a trusted desktop-shell actor in the general `/v1/ui/intents` identity model;
- `open_widget`, `close_widget`, `move_widget`, `activate_tab`, and
  `reset_layout` command reducers;
- migration apply/import, preset switching, revision retention deletion, and a
  standalone backup object.

No hidden `core.workspace` Widget is persisted and no existing session Widget
may impersonate Workspace authority. Later intent integration must either keep
the dedicated authenticated command route or formally model a trusted shell
actor before using the general Surface intent endpoint.
