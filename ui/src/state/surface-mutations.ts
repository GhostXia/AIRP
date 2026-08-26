export const WRITABLE_SURFACE_WIDGET_TYPES = [
  "core.chat",
  "core.memory",
  "core.character-state",
] as const;

export const NON_STREAMING_SURFACE_MUTATIONS = [
  "memory.replace",
  "characterState.patch",
] as const;

export function isNonStreamingSurfaceMutation(name: string): boolean {
  return (NON_STREAMING_SURFACE_MUTATIONS as readonly string[]).includes(name);
}
