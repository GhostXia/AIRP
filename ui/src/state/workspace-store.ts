import { readonly, shallowReactive } from "vue";
import type {
  WorkspaceCommand,
  WorkspaceDocument,
  WorkspaceNode,
} from "../protocol/http-engine-bus";

export interface WorkspaceViewState {
  acceptedDocument: WorkspaceDocument | null;
  pendingCommand: WorkspaceCommand["type"] | null;
  lastError: string | null;
}

/** Orders asynchronous Workspace reads and writes within one published Bus scope. */
export class WorkspaceRequestGate {
  private epoch = 0;

  begin(): number {
    this.epoch += 1;
    return this.epoch;
  }

  invalidate(): void {
    this.epoch += 1;
  }

  isCurrent(epoch: number): boolean {
    return epoch === this.epoch;
  }
}

function collectSplitRatios(node: WorkspaceNode, ratios: Record<string, number>): void {
  if (node.type === "widget") return;
  if (node.type === "split") ratios[node.id] = node.ratioBasisPoints;
  for (const child of node.children) collectSplitRatios(child, ratios);
}

/** Accepted-only client view of the Engine-authoritative Workspace asset. */
export class WorkspaceStateStore {
  private readonly mutable = shallowReactive<WorkspaceViewState>({
    acceptedDocument: null,
    pendingCommand: null,
    lastError: null,
  });

  readonly state = readonly(this.mutable as object) as Readonly<WorkspaceViewState>;

  get acceptedDocument(): WorkspaceDocument | null {
    return this.mutable.acceptedDocument === null
      ? null
      : structuredClone(this.mutable.acceptedDocument);
  }

  get revision(): string | null {
    return this.mutable.acceptedDocument?.revision ?? null;
  }

  get pendingCommand(): WorkspaceCommand["type"] | null {
    return this.mutable.pendingCommand;
  }

  get lastError(): string | null {
    return this.mutable.lastError;
  }

  get splitRatioByNodeId(): Readonly<Record<string, number>> {
    const ratios: Record<string, number> = {};
    if (this.mutable.acceptedDocument !== null) {
      collectSplitRatios(this.mutable.acceptedDocument.layout.root, ratios);
    }
    return ratios;
  }

  accept(document: WorkspaceDocument): boolean {
    const currentRevision = this.mutable.acceptedDocument?.revision;
    if (currentRevision !== undefined && BigInt(document.revision) < BigInt(currentRevision)) return false;
    this.mutable.acceptedDocument = structuredClone(document);
    this.mutable.lastError = null;
    return true;
  }

  clear(): void {
    this.mutable.acceptedDocument = null;
    this.mutable.pendingCommand = null;
    this.mutable.lastError = null;
  }

  begin(command: WorkspaceCommand): boolean {
    if (this.mutable.pendingCommand !== null || this.mutable.acceptedDocument === null) return false;
    this.mutable.pendingCommand = command.type;
    this.mutable.lastError = null;
    return true;
  }

  finish(document: WorkspaceDocument): void {
    this.accept(document);
    this.mutable.pendingCommand = null;
  }

  fail(message: string): void {
    this.mutable.pendingCommand = null;
    this.mutable.lastError = message;
  }
}
