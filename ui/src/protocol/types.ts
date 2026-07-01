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
  intents?: string[];
  entry?: WidgetEntry;
  author?: string;
  homepage?: string;
  license?: string;
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
