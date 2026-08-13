export type WorkspaceId = "story" | "world" | "director" | "debug";

export interface WorkspacePreset {
  id: WorkspaceId;
  label: string;
  shortLabel: string;
  description: string;
}

export const WORKSPACE_PRESETS: readonly WorkspacePreset[] = [
  { id: "story", label: "故事", shortLabel: "故", description: "叙事、对话与当前场景" },
  { id: "world", label: "世界", shortLabel: "世", description: "设定、人物与关系" },
  { id: "director", label: "导演", shortLabel: "导", description: "剧情推进与创作控制" },
  { id: "debug", label: "诊断", shortLabel: "诊", description: "活动、错误与运行证据" },
] as const;

export function nextWorkspaceIndex(current: number, key: string, count = WORKSPACE_PRESETS.length): number {
  if (count <= 0) return -1;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (key === "ArrowDown" || key === "ArrowRight") return (current + 1) % count;
  if (key === "ArrowUp" || key === "ArrowLeft") return (current - 1 + count) % count;
  return current;
}
