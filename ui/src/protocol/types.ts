/**
 * State Protocol types for the UI.
 *
 * Single source of truth is the Rust wire types in `protocol/src/lib.rs`
 * (crate `airp-state-protocol`). This file is the hand-mirrored TypeScript
 * binding for the UI; keep it in sync when the Rust types change. The
 * `serde(tag = "kind", rename_all = "snake_case")` / `rename` annotations on
 * the Rust side are reflected here as discriminated unions and string literals.
 */

export type Json = string | number | boolean | null | Json[] | { [k: string]: Json };

export const PROTOCOL_VERSION = 1 as const;

export interface Envelope {
  v: typeof PROTOCOL_VERSION;
  id: string;
  ts: number;
  src: string;
  body: Body;
}

export type Body =
  | BlueprintBody
  | StateBody
  | ManifestBody
  | EventBody
  | ErrorBody
  | IntentBody
  | SubscribeBody
  | UnsubscribeBody
  | HelloBody
  | AckBody;

export interface BlueprintBody {
  kind: "blueprint";
  op: "set" | "patch";
  blueprint?: Blueprint;
  patch?: JsonPatch;
}

export interface StateBody {
  kind: "state";
  scope: string;
  op: "set" | "patch";
  state?: Json;
  patch?: JsonPatch;
}

export interface ManifestBody {
  kind: "manifest";
  op: "set" | "patch";
  manifests: WidgetDef[];
}

export interface EventBody {
  kind: "event";
  topic: string;
  data?: Json;
}

export interface ErrorBody {
  kind: "error";
  code: string;
  message: string;
  detail?: Json;
}

export interface IntentBody {
  kind: "intent";
  name: string;
  source?: string;
  params?: Json;
}

export interface SubscribeBody {
  kind: "subscribe";
  scopes: string[];
}

export interface UnsubscribeBody {
  kind: "unsubscribe";
  scopes: string[];
}

export interface HelloBody {
  kind: "hello";
  client: string;
  version: string;
  accept?: string[];
}

export interface AckBody {
  kind: "ack";
  /** The acknowledged envelope id (serde rename of `ref_`). */
  ref: string;
}

export interface Blueprint {
  version: string;
  profile?: string;
  theme?: Theme;
  layout: Layout;
  widgets: WidgetInstance[];
}

export interface Theme {
  name: string;
  tokens?: Record<string, string>;
}

export interface Layout {
  type: "dock" | "grid" | "stack" | "tabs";
  areas: Area[];
}

export interface Area {
  id: string;
  widgets: string[];
  props?: Json;
}

export interface WidgetInstance {
  id: string;
  /** Registry key, e.g. "core.chat" (serde rename of `kind`). */
  type: string;
  props?: Json;
  state?: string;
  capabilities?: Capability[];
}

export interface WidgetDef {
  /** Namespaced id, e.g. "core.chat" (serde rename of `kind`). */
  type: string;
  version: string;
  title: string;
  description?: string;
  /** JSON Schema for props (serde `propsSchema`). */
  propsSchema?: Json;
  /** JSON Schema for state (serde `stateSchema`). */
  stateSchema?: Json;
  capabilities?: Capability[];
  /** Required WidgetContext host contract; omitted means major 1. */
  host_api?: string;
  /** Optional trusted local plugin dependencies; missing entries degrade visibly. */
  trusted_plugins?: TrustedPluginDependency[];
  intents?: string[];
  entry?: WidgetEntry;
  author?: string;
  homepage?: string;
  license?: string;
}

export interface TrustedPluginDependency {
  id: string;
  min_host_api?: string;
}

export interface WidgetEntry {
  kind: "builtin" | "esm";
  source?: string;
  sandbox?: boolean;
}

export type Capability =
  | "read:memory"
  | "write:memory"
  | "read:worldbook"
  | "read:state"
  | "write:state"
  | "call:tool";

export type JsonPatch = PatchOp[];

export interface PatchOp {
  op: PatchOpKind;
  path: string;
  value?: Json;
  from?: string;
}

export type PatchOpKind = "add" | "remove" | "replace" | "move" | "copy" | "test";

export type SetOrPatch = "set" | "patch";

// ---------------------------------------------------------------------------
// Surface Protocol v2
// ---------------------------------------------------------------------------
//
// v2 is intentionally additive to this module.  The existing v1 Envelope and
// its Blueprint remain available to the demo and are not silently retyped.

export const SURFACE_PROTOCOL_MAJOR = 2 as const;
export const SURFACE_PROTOCOL_MINOR = 0 as const;

/** Decimal unsigned-64 revision; never convert this value to a JS number. */
export type SurfaceRevision = string;

export interface SurfaceProtocolVersion {
  major: number;
  minor: number;
}

export type SurfaceLayoutNode =
  | SurfaceSplitNode
  | SurfaceTabsNode
  | SurfaceStackNode
  | SurfaceWidgetNode;

export interface SurfaceSplitNode {
  type: "split";
  id: string;
  orientation: "horizontal" | "vertical";
  children: [SurfaceLayoutNode, SurfaceLayoutNode];
}

export interface SurfaceTabsNode {
  type: "tabs";
  id: string;
  /** Stable id of one direct child in `children`. */
  active: string;
  children: SurfaceLayoutNode[];
}

export interface SurfaceStackNode {
  type: "stack";
  id: string;
  children: SurfaceLayoutNode[];
}

export interface SurfaceWidgetNode {
  type: "widget";
  id: string;
  /** Stable reference into `BlueprintV2.widgets`. */
  instanceId: string;
}

export interface SurfaceWidgetInstance {
  id: string;
  type: string;
  props?: Json;
}

export interface BlueprintV2 {
  version: 2;
  root: SurfaceLayoutNode;
  widgets: SurfaceWidgetInstance[];
}

export interface SurfaceSnapshot {
  kind: "snapshot";
  protocol: SurfaceProtocolVersion;
  surfaceId: string;
  revision: SurfaceRevision;
  blueprint: BlueprintV2;
}

export type SurfaceSnapshotV2 = SurfaceSnapshot;

export type SurfacePatchOperation = "add" | "remove" | "replace" | "move" | "copy" | "test";

export interface SurfacePatchOp {
  op: SurfacePatchOperation;
  /** RFC 6901 pointer into the full snapshot document. */
  path: string;
  value?: Json;
  from?: string;
}

export interface SurfacePatchEvent {
  kind: "patch";
  protocol: SurfaceProtocolVersion;
  surfaceId: string;
  baseRevision: SurfaceRevision;
  revision: SurfaceRevision;
  patch: SurfacePatchOp[];
}

export type SurfacePatchEventV2 = SurfacePatchEvent;

export type SurfaceMessage = SurfaceSnapshot | SurfacePatchEvent;

export type SurfaceErrorCode =
  | "unsupported_major"
  | "invalid_version"
  | "invalid_revision"
  | "revision_mismatch"
  | "revision_gap"
  | "invalid_blueprint"
  | "duplicate_instance_id"
  | "invalid_reference"
  | "invalid_patch"
  | "resource_limit"
  | "document_too_large"
  | "forbidden_executable_field"
  | "resync_required";

export interface SurfaceError {
  kind: "error";
  protocol: SurfaceProtocolVersion;
  surfaceId?: string;
  code: SurfaceErrorCode;
  message: string;
  resync: true;
}
