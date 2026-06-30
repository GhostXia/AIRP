import { describe, it, expect } from "vitest";
import { computeWindow } from "./virtual-window";

describe("computeWindow", () => {
  it("returns empty window for no items", () => {
    expect(computeWindow({ scrollTop: 0, viewportHeight: 200, itemHeight: 48, total: 0 })).toEqual({
      start: 0,
      end: 0,
      padTop: 0,
      padBottom: 0,
    });
  });

  it("renders only the viewport slice + overscan, with matching spacers", () => {
    // 1000 items × 48px; viewport 480px (~10 rows); scrolled to row 100.
    const w = computeWindow({
      scrollTop: 100 * 48,
      viewportHeight: 480,
      itemHeight: 48,
      total: 1000,
      overscan: 5,
    });
    expect(w.start).toBe(95); // 100 - overscan
    expect(w.end).toBe(115); // 100 + 10 visible + 5 overscan
    expect(w.padTop).toBe(95 * 48);
    expect(w.padBottom).toBe((1000 - 115) * 48);
    // rendered rows are a tiny fraction of the total (the whole point)
    expect(w.end - w.start).toBeLessThan(30);
  });

  it("clamps at the top", () => {
    const w = computeWindow({ scrollTop: 0, viewportHeight: 480, itemHeight: 48, total: 1000, overscan: 5 });
    expect(w.start).toBe(0);
    expect(w.padTop).toBe(0);
  });

  it("clamps at the bottom", () => {
    const w = computeWindow({ scrollTop: 1000 * 48, viewportHeight: 480, itemHeight: 48, total: 1000, overscan: 5 });
    expect(w.end).toBe(1000);
    expect(w.padBottom).toBe(0);
  });
});
