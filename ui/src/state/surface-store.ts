import { reactive, readonly, shallowReactive } from "vue";
import {
  SurfaceStore as AtomicSurfaceStore,
  type SurfaceApplyResult,
  type SurfaceResyncRequest,
} from "../protocol/surface-v2";
import type {
  SurfaceLayoutNode,
  SurfaceMessage,
  SurfaceSnapshot,
} from "../protocol/types";

export interface SurfaceViewState {
  acceptedSnapshot: SurfaceSnapshot | null;
  pendingUpdate: unknown | null;
  lastResync: SurfaceResyncRequest | null;
  lastError: Extract<SurfaceApplyResult, { status: "resync" }>["error"] | null;
  activeTabByNodeId: Record<string, string>;
  focusedWidgetInstanceId: string | null;
}

interface TabsState {
  active: string;
  children: Set<string>;
  widgetsByChild: Map<string, Set<string>>;
}

function collectWidgetIds(node: SurfaceLayoutNode, ids: Set<string>): void {
  if (node.type === "widget") {
    ids.add(node.instanceId);
    return;
  }
  for (const child of node.children) collectWidgetIds(child, ids);
}

function collectTabs(node: SurfaceLayoutNode, tabs: Map<string, TabsState>): void {
  if (node.type === "widget") return;
  if (node.type === "tabs") {
    tabs.set(node.id, {
      active: node.active,
      children: new Set(node.children.map((child) => child.id)),
      widgetsByChild: new Map(node.children.map((child) => {
        const ids = new Set<string>();
        collectWidgetIds(child, ids);
        return [child.id, ids];
      })),
    });
  }
  for (const child of node.children) collectTabs(child, tabs);
}

function tabsIn(snapshot: SurfaceSnapshot | null): Map<string, TabsState> {
  const tabs = new Map<string, TabsState>();
  if (snapshot !== null) collectTabs(snapshot.blueprint.root, tabs);
  return tabs;
}

/** Vue-reactive UI state layered over the protocol's atomic Surface store. */
export class SurfaceStateStore {
  private readonly atomic = new AtomicSurfaceStore();
  private readonly mutable = shallowReactive<SurfaceViewState>({
    acceptedSnapshot: null,
    pendingUpdate: null,
    lastResync: null,
    lastError: null,
    activeTabByNodeId: reactive<Record<string, string>>({}),
    focusedWidgetInstanceId: null,
  });

  readonly state = readonly(this.mutable as object) as Readonly<SurfaceViewState>;

  get acceptedSnapshot(): SurfaceSnapshot | null {
    return this.mutable.acceptedSnapshot === null
      ? null
      : structuredClone(this.mutable.acceptedSnapshot);
  }

  get pendingUpdate(): unknown | null {
    return this.mutable.pendingUpdate;
  }

  get lastResync(): SurfaceResyncRequest | null {
    return this.mutable.lastResync;
  }

  get lastError(): SurfaceViewState["lastError"] {
    return this.mutable.lastError;
  }

  get activeTabByNodeId(): Readonly<Record<string, string>> {
    return this.state.activeTabByNodeId;
  }

  get focusedWidgetInstanceId(): string | null {
    return this.mutable.focusedWidgetInstanceId;
  }

  applySnapshot(value: unknown): SurfaceApplyResult {
    return this.applyUpdate(value, () => this.atomic.applySnapshot(value));
  }

  applyPatch(value: unknown): SurfaceApplyResult {
    return this.applyUpdate(value, () => this.atomic.applyPatch(value));
  }

  apply(value: SurfaceMessage | unknown): SurfaceApplyResult {
    return this.applyUpdate(value, () => this.atomic.apply(value));
  }

  activateTab(tabsNodeId: string, childNodeId: string): boolean {
    const tabs = tabsIn(this.mutable.acceptedSnapshot).get(tabsNodeId);
    if (tabs === undefined || !tabs.children.has(childNodeId)) return false;
    this.mutable.activeTabByNodeId[tabsNodeId] = childNodeId;
    const focused = this.mutable.focusedWidgetInstanceId;
    if (focused !== null) {
      const allTabWidgets = new Set([...tabs.widgetsByChild.values()].flatMap((ids) => [...ids]));
      if (allTabWidgets.has(focused) && !tabs.widgetsByChild.get(childNodeId)?.has(focused)) {
        this.mutable.focusedWidgetInstanceId = null;
      }
    }
    return true;
  }

  focusWidget(instanceId: string | null): boolean {
    if (instanceId === null) {
      this.mutable.focusedWidgetInstanceId = null;
      return true;
    }
    const widgets = this.mutable.acceptedSnapshot?.blueprint.widgets;
    if (widgets === undefined || !widgets.some((widget) => widget.id === instanceId)) return false;
    this.mutable.focusedWidgetInstanceId = instanceId;
    return true;
  }

  private applyUpdate(value: unknown, apply: () => SurfaceApplyResult): SurfaceApplyResult {
    this.mutable.pendingUpdate = value;
    try {
      const result = apply();
      if (result.status === "resync") {
        this.mutable.lastResync = result.request;
        this.mutable.lastError = result.error;
        return result;
      }

      const previous = this.mutable.acceptedSnapshot;
      this.mutable.acceptedSnapshot = result.snapshot;
      this.mutable.lastResync = null;
      this.mutable.lastError = null;
      this.reconcileEphemeral(previous, result.snapshot);
      return result;
    } finally {
      this.mutable.pendingUpdate = null;
    }
  }

  private reconcileEphemeral(previous: SurfaceSnapshot | null, accepted: SurfaceSnapshot): void {
    const oldTabs = tabsIn(previous);
    const newTabs = tabsIn(accepted);
    const sameSurface = previous?.surfaceId === accepted.surfaceId;

    for (const tabsNodeId of Object.keys(this.mutable.activeTabByNodeId)) {
      if (!newTabs.has(tabsNodeId)) delete this.mutable.activeTabByNodeId[tabsNodeId];
    }

    for (const [tabsNodeId, tabs] of newTabs) {
      const selected = this.mutable.activeTabByNodeId[tabsNodeId];
      const authoritativeChanged = !sameSurface || oldTabs.get(tabsNodeId)?.active !== tabs.active;
      if (selected === undefined || authoritativeChanged || !tabs.children.has(selected)) {
        this.mutable.activeTabByNodeId[tabsNodeId] = tabs.active;
      }
    }

    const focused = this.mutable.focusedWidgetInstanceId;
    if (focused !== null && !accepted.blueprint.widgets.some((widget) => widget.id === focused)) {
      this.mutable.focusedWidgetInstanceId = null;
    }
  }
}
