import type { Json, SurfaceSnapshot } from "./protocol/types";

export type ChatOperationBaseline = {
  messageIds: string[];
  lastAssistantId?: string;
  lastAssistantContent?: string;
  lastAssistantCandidateCount: number;
  lastAssistantCandidateIndex: number;
};

export type RecoverableChatOperation = {
  mode?: string;
  targetMessageId?: string;
  targetIndex?: number;
  baseline?: ChatOperationBaseline;
  retryName?: string;
  retryParams?: Json;
};

type ProjectedChat = {
  messages?: Array<{ role?: unknown; content?: unknown }>;
  message_ids?: string[];
  message_candidates?: unknown[][];
  message_swipe_index?: number[];
};

function chatProjection(snapshot: SurfaceSnapshot | null, instanceId: string): ProjectedChat | null {
  const widget = snapshot?.blueprint.widgets.find((candidate) => candidate.id === instanceId);
  return widget?.props && typeof widget.props === "object" ? widget.props as ProjectedChat : null;
}

function lastAssistant(chat: ProjectedChat | null): {
  id?: string;
  content?: string;
  candidateCount: number;
  candidateIndex: number;
} {
  if (!chat || !Array.isArray(chat.messages)) return { candidateCount: 0, candidateIndex: 0 };
  for (let index = chat.messages.length - 1; index >= 0; index -= 1) {
    const message = chat.messages[index];
    if (message?.role !== "assistant") continue;
    return {
      id: chat.message_ids?.[index],
      content: typeof message.content === "string" ? message.content : undefined,
      candidateCount: chat.message_candidates?.[index]?.length ?? 0,
      candidateIndex: chat.message_swipe_index?.[index] ?? 0,
    };
  }
  return { candidateCount: 0, candidateIndex: 0 };
}

export function captureChatBaseline(
  snapshot: SurfaceSnapshot | null,
  instanceId: string,
): ChatOperationBaseline {
  const chat = chatProjection(snapshot, instanceId);
  const assistant = lastAssistant(chat);
  return {
    messageIds: Array.isArray(chat?.message_ids) ? [...chat.message_ids] : [],
    lastAssistantId: assistant.id,
    lastAssistantContent: assistant.content,
    lastAssistantCandidateCount: assistant.candidateCount,
    lastAssistantCandidateIndex: assistant.candidateIndex,
  };
}

export function chatOperationIsCommitted(
  snapshot: SurfaceSnapshot | null,
  instanceId: string,
  operation: RecoverableChatOperation,
): boolean {
  const chat = chatProjection(snapshot, instanceId);
  const baseline = operation.baseline;
  if (!chat || !baseline) return false;
  if (operation.mode === "swipe" && operation.targetMessageId && Number.isInteger(operation.targetIndex)) {
    const index = chat.message_ids?.indexOf(operation.targetMessageId) ?? -1;
    return index >= 0 && chat.message_swipe_index?.[index] === operation.targetIndex;
  }
  const assistant = lastAssistant(chat);
  if (operation.mode === "send") {
    return !!assistant.id && !baseline.messageIds.includes(assistant.id);
  }
  if (operation.mode === "continue") {
    return assistant.id === baseline.lastAssistantId
      && assistant.content !== baseline.lastAssistantContent;
  }
  if (operation.mode === "regen") {
    return assistant.id === baseline.lastAssistantId && (
      assistant.content !== baseline.lastAssistantContent
      || assistant.candidateCount !== baseline.lastAssistantCandidateCount
      || assistant.candidateIndex !== baseline.lastAssistantCandidateIndex
    );
  }
  return false;
}

export function chatProjectionChanged(
  snapshot: SurfaceSnapshot | null,
  instanceId: string,
  operation: RecoverableChatOperation,
): boolean {
  const chat = chatProjection(snapshot, instanceId);
  const baseline = operation.baseline;
  if (!chat || !baseline) return false;
  const ids = Array.isArray(chat.message_ids) ? chat.message_ids : [];
  if (ids.length !== baseline.messageIds.length || ids.some((id, index) => id !== baseline.messageIds[index])) {
    return true;
  }
  const assistant = lastAssistant(chat);
  return assistant.id !== baseline.lastAssistantId
    || assistant.content !== baseline.lastAssistantContent
    || assistant.candidateCount !== baseline.lastAssistantCandidateCount
    || assistant.candidateIndex !== baseline.lastAssistantCandidateIndex;
}

export function projectedSessionPhase(snapshot: SurfaceSnapshot | null): string | null {
  const activity = snapshot?.blueprint.widgets.find((widget) => widget.type === "core.activity");
  const props = activity?.props as { live?: { phase?: unknown } } | null;
  return typeof props?.live?.phase === "string" ? props.live.phase : null;
}

export type ChatRecoveryProjection = "recovery_required" | "committed" | "busy" | "changed" | "unchanged" | "unknown";

/** Recovery lock always wins over a projection that merely looks committed. */
export function classifyChatRecoveryProjection(
  snapshot: SurfaceSnapshot | null,
  instanceId: string,
  operation: RecoverableChatOperation,
): ChatRecoveryProjection {
  const phase = projectedSessionPhase(snapshot);
  if (phase === "recovering") return "recovery_required";
  if (chatOperationIsCommitted(snapshot, instanceId, operation)) return "committed";
  if (phase === "generating" || phase === "committing") return "busy";
  if (phase === "idle") {
    return chatProjectionChanged(snapshot, instanceId, operation) ? "changed" : "unchanged";
  }
  return "unknown";
}

export function cancellationIsExplicitlyNotCommitted(commitState: unknown): boolean {
  return commitState === "not_committed";
}
