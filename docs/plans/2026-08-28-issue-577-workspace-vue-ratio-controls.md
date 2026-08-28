# #577 PR 10e — accepted Workspace controls and responsive split ratios

Date: 2026-08-28

Roadmap: #577 PR 10

Status: implementation candidate

## Client authority boundary

The production Vue desktop loads the validated `default` Workspace for the
resolved user scope before connecting its session Surface. Client Workspace
state is accepted-only:

- one complete Engine response replaces the accepted document;
- a command uses the accepted decimal-string revision exactly once;
- no command changes visible state before an Engine response;
- a conflict, failure, or unknown result triggers a read of the latest
  Workspace but never automatically replays the mutation;
- a stale response from an earlier Bus attempt cannot enter the current scope.
- a replacement Bus and resolved user scope become command-visible together
  only after Workspace, session list, and Surface connection are ready;
- manual reads and commands on the published Bus share an operation epoch;
  initialization reads use candidate/attempt identity, and an older revision
  can never replace a newer accepted document.

Surface remains the accepted structure and Widget-props authority. Vue does
not rewrite the accepted Surface blueprint. Workspace contributes only the
saved split-ratio map that Surface v2 cannot yet express.

## Visible behavior

- Saved `ratioBasisPoints` controls wide-screen split tracks.
- A compact ridge at the split boundary changes the leading pane in bounded
  five-percent steps and disables while a command is pending.
- Tab activation uses the Workspace command endpoint and waits for the Engine
  Surface poll to publish the new active tab.
- Below 760 CSS pixels, horizontal splits stack vertically and the ridge is
  hidden; the saved desktop ratio remains intact for larger viewports.
- Reduced-motion users receive no decorative transition.

The real-Engine browser smoke proves the default 65/35 Workspace, a CAS-bound
change to 70/30, the rendered CSS ratio, the following tab command using the
new Workspace revision, exact non-duplicated command order, the 760/761 CSS-px
stacking boundary, and the existing Memory, Character State, and Chat vertical
slices.

## Explicitly deferred

- open, close, move, and reset controls;
- drag-to-resize and arbitrary pixel persistence;
- extension-backed Workspace Widget types;
- migration apply/import, backup/rollback, and multiple named Workspaces;
- adding ratio fields to Surface v2.
