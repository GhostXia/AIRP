# #577 PR 10c — closed Workspace layout commands

Date: 2026-08-26

Roadmap: #577 PR 10

Status: implementation candidate

## Command contract

The authenticated Workspace command endpoint accepts exactly six operations:

- `open_widget`: declare one allowlisted first-party Widget and insert its
  Engine-derived placement node into a tabs or stack container;
- `close_widget`: remove one declared Widget and its unique placement;
- `move_widget`: preserve the placement node identity while moving it to a tabs
  or stack container;
- `resize_split`: replace one bounded split ratio;
- `activate_tab`: select one direct child of a tabs container;
- `reset_layout`: commit the deterministic default layout as a new revision.

Every request carries an expected decimal-string revision. Commands execute on
the currently validated layout copy under the Workspace lock. The complete
candidate then passes Workspace v1 validation before immutable revision commit;
invalid targets, duplicates, disallowed Widget types, empty containers, stale
revisions, invalid references, and resource-limit failures publish nothing.

For open/move, an omitted index appends. A supplied index is interpreted against
the target children after any source placement has been removed. The client
does not submit a node ID for open: the Engine reuses `instance_id` as the
placement node ID and rejects any collision.

## Explicitly deferred

- Vue controls and optimistic-draft reconciliation;
- consuming the saved Workspace as the layout source for a session Surface;
- a general `/v1/ui/intents` desktop-shell actor;
- extension-backed Widget types;
- migration apply/import, backup objects, and revision retention deletion.
