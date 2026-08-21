/** Lifecycle guard for asynchronously acquired widget resources. */
export class WidgetLifecycle {
  private active = true;
  private teardown: (() => void) | null = null;
  private stateCallback: ((state: unknown) => void) | null = null;
  private holds = 0;

  constructor(private readonly onError: (error: unknown) => void) {}

  /** Own a resource, or tear it down immediately if acquisition completed late. */
  adopt(teardown: () => void): boolean {
    if (!this.active) {
      this.contain(teardown);
      return false;
    }
    this.teardown = teardown;
    return true;
  }

  /** Delay teardown while an already-started async mount settles. */
  hold(): () => void {
    if (!this.active) return () => {};
    this.holds += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.holds -= 1;
      this.flushTeardown();
    };
  }

  onState(callback: (state: unknown) => void): () => void {
    if (!this.active) return () => {};
    this.stateCallback = callback;
    return () => {
      if (this.stateCallback === callback) this.stateCallback = null;
    };
  }

  pushState(state: unknown): void {
    const callback = this.stateCallback;
    if (this.active && callback) this.contain(() => callback(state));
  }

  run(effect: () => void): void {
    if (this.active) this.contain(effect);
  }

  fail(error: unknown): void {
    if (!this.active) return;
    this.onError(error);
    this.dispose();
  }

  dispose(): void {
    if (!this.active) return;
    this.active = false;
    this.stateCallback = null;
    this.flushTeardown();
  }

  private flushTeardown(): void {
    if (this.active || this.holds > 0) return;
    const teardown = this.teardown;
    this.teardown = null;
    if (teardown) this.contain(teardown);
  }

  private contain(effect: () => void): void {
    try {
      effect();
    } catch (error) {
      this.fail(error);
    }
  }
}
