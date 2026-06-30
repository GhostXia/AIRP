/**
 * Pure fixed-height windowing math for virtualized lists (performance contract:
 * only render the viewport slice). Framework-agnostic and unit-testable; the
 * actual scroll/fps behavior is a runtime spike (see PLAN unverified ledger).
 */

export interface VirtualWindow {
  /** First item index to render (inclusive). */
  start: number;
  /** One past the last item index to render (exclusive). */
  end: number;
  /** Spacer height above the rendered slice, in px. */
  padTop: number;
  /** Spacer height below the rendered slice, in px. */
  padBottom: number;
}

export interface WindowOptions {
  scrollTop: number;
  viewportHeight: number;
  itemHeight: number;
  total: number;
  /** Extra rows rendered above/below the viewport. Default 6. */
  overscan?: number;
}

/** Compute which fixed-height rows to render for the current scroll position. */
export function computeWindow(opts: WindowOptions): VirtualWindow {
  const { scrollTop, viewportHeight, itemHeight, total } = opts;
  const overscan = opts.overscan ?? 6;

  if (itemHeight <= 0 || total <= 0) {
    return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  }

  const first = Math.floor(Math.max(0, scrollTop) / itemHeight);
  const visible = Math.ceil(Math.max(0, viewportHeight) / itemHeight);
  const start = Math.max(0, first - overscan);
  const end = Math.min(total, first + visible + overscan);

  return {
    start,
    end,
    padTop: start * itemHeight,
    padBottom: Math.max(0, (total - end) * itemHeight),
  };
}
