import { afterEach, describe, expect, it, vi } from "vitest";
import * as Vue from "vue";
import { createRenderer, h, nextTick, ref } from "vue";
import { compileScript, parse } from "vue/compiler-sfc";
import ts from "typescript";
import type { Json } from "../protocol/types";
import source from "./ChatWidget.vue?raw";
import * as virtualWindow from "./virtual-window";

// The suite's Node environment normally compiles .vue imports for SSR. Compile
// the unchanged SFC for client rendering so its actual template/ref/listener run.
const { descriptor } = parse(source, { filename: "ChatWidget.vue" });
const script = compileScript(descriptor, { id: "chat-widget-test", inlineTemplate: true });
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const componentExports = { default: {} as Vue.Component };
new Function("require", "exports", compiled)((name: string) => {
  if (name === "vue") return Vue;
  if (name === "./virtual-window") return virtualWindow;
  throw new Error(`Unexpected component import: ${name}`);
}, componentExports);
const ChatWidget = componentExports.default;

// Exercise the real component's lifecycle, reactive props and scroll listener
// without adding a DOM dependency to the Node test suite. Geometry is explicit:
// this checks pagination policy, not the browser's layout/scroll anchoring.
class TestElement {
  parent: TestElement | null = null;
  children: TestElement[] = [];
  props: Record<string, unknown> = {};
  scrollTop = 0;
  scrollHeight = 1440;
  clientHeight = 288;
  textContent = "";
  addEventListener() {}
  removeEventListener() {}
}

const renderer = createRenderer<TestElement, TestElement>({
  createElement: () => new TestElement(),
  createText: () => new TestElement(),
  createComment: () => new TestElement(),
  setText: (node, text) => { node.textContent = text; },
  setElementText: (node, text) => { node.textContent = text; },
  patchProp: (node, key, _previous, value) => { node.props[key] = value; },
  parentNode: (node) => node.parent,
  nextSibling: (node) => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  insert(node, parent, anchor = null) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
    node.parent = parent;
    const index = anchor ? parent.children.indexOf(anchor) : -1;
    parent.children.splice(index < 0 ? parent.children.length : index, 0, node);
  },
  remove(node) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
    node.parent = null;
  },
});

function findLog(node: TestElement): TestElement | undefined {
  if (node.props.class === "w-chat-log") return node;
  for (const child of node.children) {
    const log = findLog(child);
    if (log) return log;
  }
  return undefined;
}

afterEach(() => vi.unstubAllGlobals());

describe("ChatWidget history pagination (#603 / PR #601)", () => {
  it("retries a failed page exactly once after leaving and re-entering the top threshold", async () => {
    vi.stubGlobal("ResizeObserver", class {
      observe() {}
      disconnect() {}
    });
    const state = {
      messages: Array.from({ length: 20 }, (_, index) => ({ role: "user", content: `Message ${index}` })),
      message_ids: Array.from({ length: 20 }, (_, index) => `message-${index}`),
      has_more: true,
      oldest_id: "message-0",
    };
    const olderPage = {
      messages: [{ role: "user", content: "Older message" }],
      message_ids: ["older-0"],
      has_more: true,
      oldest_id: "older-0",
    };
    const operation = ref<{ status?: string; error?: string; history?: typeof olderPage }>({});
    let rejectPage!: (reason: Error) => void;
    let resolveRetry!: (page: typeof olderPage) => void;
    const loadMore = vi.fn()
      .mockImplementationOnce(() => new Promise((_, reject) => { rejectPage = reject; }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveRetry = resolve; }));
    let pending: Promise<void> | undefined;
    const onIntent = vi.fn((name: string, params?: Json) => {
      if (name !== "chat.loadMore") throw new Error(`Unexpected intent: ${name}`);
      // Model App.vue's async dispatch contract: loading_history -> error or
      // idle + history. The latch and all scroll decisions remain real code.
      operation.value = { ...operation.value, status: "loading_history" };
      pending = loadMore(params).then(
        (history: typeof olderPage) => { operation.value = { history }; },
        (error: Error) => { operation.value = { ...operation.value, status: "error", error: String(error) }; },
      );
    });
    const app = renderer.createApp({
      render: () => h(ChatWidget, {
        instance: { id: "chat", type: "core.chat" },
        state,
        operation: operation.value,
        onIntent,
      }),
    });
    const root = new TestElement();
    app.mount(root);
    try {
      await nextTick(); // Drain initial follow-latest positioning before scrolling.
      const log = findLog(root)!;
      expect(log).toBeDefined();
      const scroll = async (top: number) => {
        log.scrollTop = top;
        (log.props.onScroll as () => void)();
        await nextTick();
      };
      await scroll(0);
      expect(loadMore).toHaveBeenCalledExactlyOnceWith({ before: "message-0", limit: 50 });
      await scroll(0);
      expect(loadMore).toHaveBeenCalledTimes(1);

      rejectPage(new Error("history unavailable"));
      await pending;
      await nextTick();
      expect(operation.value).toMatchObject({ status: "error", error: "Error: history unavailable" });
      expect(loadMore).toHaveBeenCalledTimes(1); // No reactive replay on failure.
      await scroll(0);
      await scroll(143); // Still inside the 2 * 72px top threshold.
      await scroll(0);
      expect(loadMore).toHaveBeenCalledTimes(1);

      await scroll(144); // Leave the threshold; leaving alone must not retry.
      expect(loadMore).toHaveBeenCalledTimes(1);
      await scroll(143);
      expect(loadMore).toHaveBeenCalledTimes(2);
      expect(loadMore).toHaveBeenNthCalledWith(2, { before: "message-0", limit: 50 });
      await scroll(0);
      await scroll(0);
      expect(loadMore).toHaveBeenCalledTimes(2); // No concurrent duplicate retry.

      resolveRetry(olderPage);
      await pending;
      await nextTick();
      expect(operation.value.history).toEqual(olderPage);
      expect(log.scrollTop).toBe(0);
      log.scrollHeight += 72;
      await scroll(0); // PR #601: layout scroll after prepending must not chain.
      await scroll(0);
      expect(onIntent.mock.calls).toEqual([
        ["chat.loadMore", { before: "message-0", limit: 50 }],
        ["chat.loadMore", { before: "message-0", limit: 50 }],
      ]);
      expect(loadMore).toHaveBeenCalledTimes(2);
    } finally {
      app.unmount();
    }
  });
});
