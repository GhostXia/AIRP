export const MAX_INVENTORY_ITEMS = 128;

const MAX_EMOTION_LABEL_CHARS = 128;
const MAX_INVENTORY_ID_CHARS = 128;
const MAX_INVENTORY_NAME_CHARS = 256;
const MAX_INVENTORY_ICON_CHARS = 16;
const MAX_INVENTORY_QUANTITY = 1_000_000_000;

export interface ProjectionMetadata {
  revision: number;
  timestamp: string | null;
  source: {
    kind: "character_state";
    scope: "character";
    characterId: string;
  };
}

export type ProjectionView<T> =
  | { status: "available"; metadata: ProjectionMetadata; value: T }
  | { status: "unconfigured"; metadata: ProjectionMetadata }
  | { status: "unavailable"; metadata: ProjectionMetadata | null };

export interface EmotionValue {
  emotion: number;
  label?: string;
}

export interface InventoryItem {
  id: string;
  name: string;
  qty?: number;
  icon?: string;
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function safeRevision(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function boundedString(value: unknown, maximum: number): value is string {
  return typeof value === "string" && value.length > 0 && [...value].length <= maximum;
}

function metadataFrom(value: Record<string, unknown>): ProjectionMetadata | null {
  const source = record(value.source);
  if (
    !safeRevision(value.revision)
    || !(value.timestamp === null || typeof value.timestamp === "string")
    || source?.kind !== "character_state"
    || source.scope !== "character"
    || !boundedString(source.character_id, MAX_INVENTORY_ID_CHARS)
  ) return null;

  return {
    revision: value.revision,
    timestamp: value.timestamp,
    source: {
      kind: "character_state",
      scope: "character",
      characterId: source.character_id,
    },
  };
}

export function emotionView(state: unknown): ProjectionView<EmotionValue> {
  const value = record(state);
  if (!value) return { status: "unavailable", metadata: null };
  const metadata = metadataFrom(value);
  if (!metadata) return { status: "unavailable", metadata: null };
  if (value.available === false) {
    return value.emotion === undefined && value.reason === "missing"
      ? { status: "unconfigured", metadata }
      : { status: "unavailable", metadata };
  }
  if (
    value.available !== true
    || !Number.isInteger(value.emotion)
    || (value.emotion as number) < 0
    || (value.emotion as number) > 100
    || (value.label !== undefined
      && (typeof value.label !== "string" || [...value.label].length > MAX_EMOTION_LABEL_CHARS))
  ) return { status: "unavailable", metadata };

  return {
    status: "available",
    metadata,
    value: {
      emotion: value.emotion as number,
      ...(typeof value.label === "string" ? { label: value.label } : {}),
    },
  };
}

function inventoryItem(value: unknown): InventoryItem | null {
  const item = record(value);
  if (
    !item
    || !boundedString(item.id, MAX_INVENTORY_ID_CHARS)
    || !boundedString(item.name, MAX_INVENTORY_NAME_CHARS)
    || (item.qty !== undefined && (
      !Number.isSafeInteger(item.qty)
      || (item.qty as number) < 0
      || (item.qty as number) > MAX_INVENTORY_QUANTITY
    ))
    || (item.icon !== undefined && !boundedString(item.icon, MAX_INVENTORY_ICON_CHARS))
  ) return null;

  return {
    id: item.id,
    name: item.name,
    ...(typeof item.qty === "number" ? { qty: item.qty } : {}),
    ...(typeof item.icon === "string" ? { icon: item.icon } : {}),
  };
}

export function inventoryView(state: unknown): ProjectionView<InventoryItem[]> {
  const value = record(state);
  if (!value) return { status: "unavailable", metadata: null };
  const metadata = metadataFrom(value);
  if (!metadata) return { status: "unavailable", metadata: null };
  if (value.available === false) {
    return value.items === undefined && value.reason === "missing"
      ? { status: "unconfigured", metadata }
      : { status: "unavailable", metadata };
  }
  if (
    value.available !== true
    || !Array.isArray(value.items)
    || value.items.length > MAX_INVENTORY_ITEMS
  ) return { status: "unavailable", metadata };

  const items = value.items.map(inventoryItem);
  if (items.some((item) => item === null)) return { status: "unavailable", metadata };
  const validItems = items as InventoryItem[];
  if (new Set(validItems.map((item) => item.id)).size !== validItems.length) {
    return { status: "unavailable", metadata };
  }
  return { status: "available", metadata, value: validItems };
}

export function projectionTimestamp(metadata: ProjectionMetadata | null): string {
  if (!metadata) return "—";
  if (metadata.timestamp === null) return "尚无更新时间";
  const timestamp = new Date(metadata.timestamp);
  return Number.isNaN(timestamp.getTime()) ? metadata.timestamp : timestamp.toLocaleString();
}
