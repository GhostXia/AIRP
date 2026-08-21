import { describe, expect, it, vi } from "vitest";
import { WidgetLifecycle } from "./widget-lifecycle";

describe("WidgetLifecycle", () => {
  it("tears down a resource acquired after disposal without allowing use", () => {
    const lifecycle = new WidgetLifecycle(() => {});
    const teardown = vi.fn();

    lifecycle.dispose();

    expect(lifecycle.adopt(teardown)).toBe(false);
    expect(teardown).toHaveBeenCalledTimes(1);
  });

  it("tears down an owned resource exactly once", () => {
    const lifecycle = new WidgetLifecycle(() => {});
    const teardown = vi.fn();

    expect(lifecycle.adopt(teardown)).toBe(true);
    lifecycle.dispose();
    lifecycle.dispose();

    expect(teardown).toHaveBeenCalledTimes(1);
  });

  it("waits for an in-flight mount before tearing its resource down", async () => {
    const lifecycle = new WidgetLifecycle(() => {});
    const teardown = vi.fn();
    lifecycle.adopt(teardown);
    const release = lifecycle.hold();
    let finishMount!: () => void;
    const mount = new Promise<void>((resolve) => {
      finishMount = resolve;
    }).finally(release);

    lifecycle.dispose();
    expect(teardown).not.toHaveBeenCalled();

    finishMount();
    await mount;
    release();
    expect(teardown).toHaveBeenCalledTimes(1);
  });

  it("clears state callbacks and rejects late subscriptions on disposal", () => {
    const lifecycle = new WidgetLifecycle(() => {});
    const before = vi.fn();
    const after = vi.fn();
    lifecycle.onState(before);

    lifecycle.dispose();
    lifecycle.pushState({ value: 1 });
    lifecycle.onState(after);
    lifecycle.pushState({ value: 2 });

    expect(before).not.toHaveBeenCalled();
    expect(after).not.toHaveBeenCalled();
  });

  it("contains callback and teardown errors locally", () => {
    const errors: unknown[] = [];
    const lifecycle = new WidgetLifecycle((error) => errors.push(error));
    const callbackError = new Error("callback failed");
    lifecycle.onState(() => {
      throw callbackError;
    });
    lifecycle.adopt(() => {
      throw new Error("teardown failed");
    });

    expect(() => lifecycle.pushState({})).not.toThrow();
    expect(() => lifecycle.dispose()).not.toThrow();
    expect(errors).toEqual([callbackError]);
  });

  it("suppresses effects and failures after disposal", () => {
    const onError = vi.fn();
    const lifecycle = new WidgetLifecycle(onError);
    const effect = vi.fn();
    lifecycle.dispose();

    lifecycle.run(effect);
    lifecycle.fail(new Error("late failure"));

    expect(effect).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });
});
