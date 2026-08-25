import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const uiRoot = path.dirname(fileURLToPath(import.meta.url));
const outFlag = process.argv.indexOf("--out");
const outDir = path.resolve(outFlag >= 0 ? process.argv[outFlag + 1] : path.join(uiRoot, "dist", "runtime-smoke"));
const port = 1422;
const origin = `http://127.0.0.1:${port}`;

function chromeExecutable() {
  const candidates = [
    process.env.AIRP_CHROME_PATH,
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, "Google", "Chrome", "Application", "chrome.exe"),
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
  ].filter(Boolean);
  const found = candidates.find((candidate) => existsSync(candidate));
  assert.ok(found, "Chrome/Chromium not found; set AIRP_CHROME_PATH");
  return found;
}

async function waitForVite(child) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`Vite exited early with ${child.exitCode}`);
    try {
      const response = await fetch(origin);
      if (response.ok) return;
    } catch { /* retry */ }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error("Vite did not become ready within 30s");
}

async function stopChild(child) {
  if (!child || child.exitCode !== null) return;
  const exited = new Promise((resolve) => child.once("exit", resolve));
  child.kill();
  const completed = await Promise.race([
    exited.then(() => true),
    new Promise((resolve) => setTimeout(() => resolve(false), 5_000)),
  ]);
  if (!completed) {
    console.error("Vite did not exit within 5s; forcing termination");
    child.kill("SIGKILL");
  }
}

let vite = null;
let browser = null;
let viteOutput = "";

try {
  vite = spawn(process.execPath, [path.join(uiRoot, "node_modules", "vite", "bin", "vite.js"), "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
    cwd: uiRoot,
    stdio: ["ignore", "pipe", "pipe"],
  });
  for (const stream of [vite.stdout, vite.stderr]) stream.on("data", (chunk) => { viteOutput = (viteOutput + chunk).slice(-4000); });
  browser = await chromium.launch({ headless: true, executablePath: chromeExecutable() });
  await waitForVite(vite);
  mkdirSync(outDir, { recursive: true });

  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(`${origin}/?airp_agent_test=1&airp_fixture=1`, { waitUntil: "networkidle" });
  await page.locator('[data-blueprint-version="2"]').waitFor({ state: "visible" });

  for (const kind of ["split", "tabs", "stack", "widget"]) {
    assert.ok(await page.locator(`.layout-${kind}`).count(), `${kind} layout node did not render`);
  }

  const tabs = page.locator('[role="tab"]');
  assert.equal(await tabs.count(), 2);
  assert.equal(await tabs.nth(0).getAttribute("aria-selected"), "true");
  assert.equal(await page.locator('[role="tabpanel"]').nth(1).locator(".widget-host").count(), 1, "inactive tab must stay mounted");
  await tabs.nth(0).focus();
  await page.keyboard.press("ArrowRight");
  assert.equal(await tabs.nth(1).getAttribute("aria-selected"), "true");
  assert.equal(await tabs.nth(1).evaluate((tab) => tab === document.activeElement), true);

  const virtualRows = await page.evaluate(async () => {
    const harness = window.__AIRP_AGENT_TEST__;
    const order = Array.from({ length: 5_000 }, (_, index) => `m-${index}`);
    const messages = Object.fromEntries(order.map((id, index) => [id, { id, role: "narrator", text: `message ${index}` }]));
    harness.setWidgetState("w-chat", {
      order,
      messages,
      context: {
        character_id: "character-with-a-complete-stable-identifier",
        session_id: "00000000-0000-4000-8000-000000000002",
        persona_id: "persona-with-a-complete-stable-identifier",
        scene_id: "scene-with-a-complete-stable-identifier",
        worldbook_source_ids: ["character:character-with-a-complete-stable-identifier"],
      },
    });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    return document.querySelectorAll(".w-chat .msg").length;
  });
  assert.ok(virtualRows > 0 && virtualRows < 100, `5,000-message fixture rendered ${virtualRows} rows`);
  const contextLabels = await page.locator('[aria-label="当前对话上下文"] .context-chip')
    .evaluateAll((chips) => chips.map((chip) => chip.getAttribute("aria-label")));
  assert.deepEqual(contextLabels, [
    "角色 character-with-a-complete-stable-identifier",
    "会话 00000000-0000-4000-8000-000000000002",
    "Persona persona-with-a-complete-stable-identifier",
    "场景 scene-with-a-complete-stable-identifier",
    "世界书 character:character-with-a-complete-stable-identifier",
  ], "context chips did not preserve complete stable identifiers");
  await tabs.nth(1).focus();
  await page.keyboard.press("ArrowLeft");
  const virtualScroll = await page.evaluate(async () => {
    const log = document.querySelector(".w-chat-log");
    const rows = [...document.querySelectorAll(".w-chat .msg")];
    const rowHeight = rows.length > 1
      ? rows[1].getBoundingClientRect().top - rows[0].getBoundingClientRect().top
      : rows[0]?.getBoundingClientRect().height;
    if (!rowHeight || rowHeight <= 0) throw new Error("could not derive virtual chat row height");
    log.scrollTop = 2_500 * rowHeight;
    log.dispatchEvent(new Event("scroll"));
    await new Promise(requestAnimationFrame);
    const middle = [...document.querySelectorAll(".w-chat .msg .text")].map((node) => node.textContent ?? "");
    log.scrollTop = log.scrollHeight;
    log.dispatchEvent(new Event("scroll"));
    await new Promise(requestAnimationFrame);
    const end = [...document.querySelectorAll(".w-chat .msg .text")].map((node) => node.textContent ?? "");
    return { middle, end, rows: document.querySelectorAll(".w-chat .msg").length };
  });
  assert.ok(virtualScroll.middle.some((text) => /^message 2[0-9]{3}$/.test(text)), "middle scroll did not render the middle window");
  assert.ok(virtualScroll.end.includes("message 4999"), "end scroll did not render the final message");
  assert.ok(virtualScroll.rows < 100, `end scroll rendered ${virtualScroll.rows} rows`);

  const movePreservedHost = await page.evaluate(async () => {
    const harness = window.__AIRP_AGENT_TEST__;
    const snapshot = harness.getSnapshot().surface;
    const root = snapshot.blueprint.root;
    const tabsIndex = root.children.findIndex((node) => node.id === "story-tabs");
    const stackIndex = root.children.findIndex((node) => node.id === "story-tools");
    const characterIndex = root.children[tabsIndex].children.findIndex((node) => node.id === "characters-node");
    const baseRevision = snapshot.revision;
    const revision = (BigInt(baseRevision) + 1n).toString();
    window.__airpBeforeHost = document.querySelector('[data-widget-instance="w-characters"] .widget-host');
    const result = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision,
      revision,
      patch: [{
        op: "move",
        from: `/blueprint/root/children/${tabsIndex}/children/${characterIndex}`,
        path: `/blueprint/root/children/${stackIndex}/children/-`,
      }],
    });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const after = document.querySelector('[data-widget-instance="w-characters"] .widget-host');
    return {
      applied: result.status === "applied",
      beforeFound: Boolean(window.__airpBeforeHost),
      afterFound: Boolean(after),
      same: window.__airpBeforeHost === after,
      outletCount: document.querySelectorAll('[data-widget-instance="w-characters"]').length,
      revision: harness.getSnapshot().surface?.revision,
      expectedRevision: revision,
    };
  });
  const { revision: moveRevision, expectedRevision: expectedMoveRevision, ...moveEvidence } = movePreservedHost;
  assert.equal(moveRevision, expectedMoveRevision);
  assert.deepEqual(moveEvidence, {
    applied: true,
    beforeFound: true,
    afterFound: true,
    same: true,
    outletCount: 1,
  }, "moving a Widget remounted its host");

  const removal = await page.evaluate(async () => {
    const harness = window.__AIRP_AGENT_TEST__;
    const snapshot = harness.getSnapshot().surface;
    const root = snapshot.blueprint.root;
    const stackIndex = root.children.findIndex((node) => node.id === "story-tools");
    const characterNodeIndex = root.children[stackIndex].children.findIndex((node) => node.id === "characters-node");
    const characterWidgetIndex = snapshot.blueprint.widgets.findIndex((widget) => widget.id === "w-characters");
    const result = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision: snapshot.revision,
      revision: (BigInt(snapshot.revision) + 1n).toString(),
      patch: [
        { op: "remove", path: `/blueprint/root/children/${stackIndex}/children/${characterNodeIndex}` },
        { op: "remove", path: `/blueprint/widgets/${characterWidgetIndex}` },
      ],
    });
    await Promise.resolve();
    await Promise.resolve();
    return result.status;
  });
  assert.equal(removal, "applied");
  assert.equal(await page.locator('[data-widget-instance="w-characters"]').count(), 0);

  const patchPerformance = await page.evaluate(async () => {
    const harness = window.__AIRP_AGENT_TEST__;
    const durations = [];
    let revision = BigInt(harness.getSnapshot().surface.revision);
    const clockIndex = harness.getSnapshot().surface.blueprint.widgets.findIndex((widget) => widget.id === "w-clock");
    for (let index = 0; index < 40; index += 1) {
      const next = revision + 1n;
      const start = performance.now();
      const result = harness.applySurface({
        kind: "patch",
        protocol: { major: 2, minor: 0 },
        surfaceId: "story.preview",
        baseRevision: String(revision),
        revision: String(next),
        patch: index === 0
          ? [{ op: "add", path: `/blueprint/widgets/${clockIndex}/props`, value: { tick: index } }]
          : [{ op: "replace", path: `/blueprint/widgets/${clockIndex}/props/tick`, value: index }],
      });
      if (result.status !== "applied") throw new Error(`performance patch ${index} failed`);
      await Promise.resolve();
      await Promise.resolve();
      durations.push(performance.now() - start);
      revision = next;
    }
    durations.sort((left, right) => left - right);
    return { samples: durations.length, p95Ms: durations[Math.floor((durations.length - 1) * 0.95)] };
  });
  assert.ok(patchPerformance.p95Ms < 16, `warm patch + Vue flush (excluding layout and paint) p95 ${patchPerformance.p95Ms.toFixed(2)}ms exceeds 16ms`);

  const isolatedFallbacks = await page.evaluate(async () => {
    const harness = window.__AIRP_AGENT_TEST__;
    const snapshot = harness.getSnapshot().surface;
    const root = snapshot.blueprint.root;
    const stackIndex = root.children.findIndex((node) => node.id === "story-tools");
    const result = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision: snapshot.revision,
      revision: (BigInt(snapshot.revision) + 1n).toString(),
      patch: [
        { op: "add", path: "/blueprint/widgets/-", value: { id: "unknown-1", type: "agent-test.unknown" } },
        { op: "add", path: "/blueprint/widgets/-", value: { id: "throw-1", type: "agent-test.throw" } },
        { op: "add", path: `/blueprint/root/children/${stackIndex}/children/-`, value: { type: "widget", id: "unknown-node", instanceId: "unknown-1" } },
        { op: "add", path: `/blueprint/root/children/${stackIndex}/children/-`, value: { type: "widget", id: "throw-node", instanceId: "throw-1" } },
      ],
    });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    return {
      applied: result.status === "applied",
      unknown: document.querySelectorAll(".widget-missing").length,
      failed: document.querySelectorAll(".widget-error").length,
      sibling: document.querySelectorAll('[data-widget-instance="w-clock"] .widget-host').length,
    };
  });
  assert.deepEqual(isolatedFallbacks, { applied: true, unknown: 1, failed: 1, sibling: 1 });

  const lifecycle = await page.evaluate(async () => {
    const harness = window.__AIRP_AGENT_TEST__;
    let snapshot = harness.getSnapshot().surface;
    const stackIndex = snapshot.blueprint.root.children.findIndex((node) => node.id === "story-tools");
    const added = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision: snapshot.revision,
      revision: (BigInt(snapshot.revision) + 1n).toString(),
      patch: [
        { op: "add", path: "/blueprint/widgets/-", value: { id: "lifecycle-1", type: "agent-test.lifecycle", props: { version: 1 } } },
        { op: "add", path: `/blueprint/root/children/${stackIndex}/children/-`, value: { type: "widget", id: "lifecycle-node", instanceId: "lifecycle-1" } },
      ],
    });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const afterAdd = harness.getWidgetLifecycle();

    snapshot = harness.getSnapshot().surface;
    const lifecycleIndex = snapshot.blueprint.widgets.findIndex((widget) => widget.id === "lifecycle-1");
    const propsChanged = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision: snapshot.revision,
      revision: (BigInt(snapshot.revision) + 1n).toString(),
      patch: [{ op: "replace", path: `/blueprint/widgets/${lifecycleIndex}/props/version`, value: 2 }],
    });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const afterProps = harness.getWidgetLifecycle();

    harness.setWidgetState("lifecycle-1", { phase: "ready" });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const afterState = harness.getWidgetLifecycle();

    harness.patchWidgetState("lifecycle-1", [{ op: "replace", path: "/phase", value: "streamed" }]);
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const afterPatch = harness.getWidgetLifecycle();

    harness.setWidgetState("lifecycle-1", null);
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const afterNullState = harness.getWidgetLifecycle();

    snapshot = harness.getSnapshot().surface;
    const currentLifecycleIndex = snapshot.blueprint.widgets.findIndex((widget) => widget.id === "lifecycle-1");
    const typeChanged = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision: snapshot.revision,
      revision: (BigInt(snapshot.revision) + 1n).toString(),
      patch: [{ op: "replace", path: `/blueprint/widgets/${currentLifecycleIndex}/type`, value: "agent-test.unknown" }],
    });
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    return {
      statuses: [added.status, propsChanged.status, typeChanged.status],
      afterAdd,
      afterProps,
      afterState,
      afterPatch,
      afterNullState,
      afterType: harness.getWidgetLifecycle(),
      missing: document.querySelectorAll(".widget-missing").length,
      sibling: document.querySelectorAll('[data-widget-instance="w-clock"] .widget-host').length,
    };
  });
  assert.deepEqual(lifecycle, {
    statuses: ["applied", "applied", "applied"],
    afterAdd: { mounts: 1, unmounts: 0, lastProps: { version: 1 }, lastState: null },
    afterProps: { mounts: 1, unmounts: 0, lastProps: { version: 2 }, lastState: null },
    afterState: { mounts: 1, unmounts: 0, lastProps: { version: 2 }, lastState: { phase: "ready" } },
    afterPatch: { mounts: 1, unmounts: 0, lastProps: { version: 2 }, lastState: { phase: "streamed" } },
    afterNullState: { mounts: 1, unmounts: 0, lastProps: { version: 2 }, lastState: null },
    afterType: { mounts: 1, unmounts: 1, lastProps: { version: 2 }, lastState: null },
    missing: 2,
    sibling: 1,
  });

  const rejected = await page.evaluate(() => {
    const harness = window.__AIRP_AGENT_TEST__;
    const snapshot = harness.getSnapshot().surface;
    const result = harness.applySurface({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story.preview",
      baseRevision: snapshot.revision,
      revision: (BigInt(snapshot.revision) + 1n).toString(),
      patch: [{ op: "replace", path: "/blueprint/root", value: { type: "widget", id: "bad", instanceId: "missing" } }],
    });
    return { result, beforeRevision: snapshot.revision, afterRevision: harness.getSnapshot().surface.revision };
  });
  assert.equal(rejected.result.status, "resync");
  assert.equal(rejected.afterRevision, rejected.beforeRevision);
  assert.deepEqual(pageErrors, []);

  const screenshot = await page.screenshot({ path: path.join(outDir, "runtime-1440x900.png") });
  assert.ok(screenshot.byteLength > 15_000, "runtime screenshot is empty");
  const evidence = { performance: patchPerformance, virtualRows, virtualScrollRows: virtualScroll.rows, movePreservedHost, lifecycle, screenshotBytes: screenshot.byteLength };
  writeFileSync(path.join(outDir, "runtime-evidence.json"), `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
  console.log(`blueprint runtime smoke passed (${patchPerformance.p95Ms.toFixed(2)}ms p95, ${virtualRows} virtual rows)`);
} catch (error) {
  if (viteOutput) console.error(viteOutput);
  throw error;
} finally {
  try {
    await browser?.close();
  } catch (error) {
    console.error("failed to close runtime-smoke browser", error);
  }
  try {
    await stopChild(vite);
  } catch (error) {
    console.error("failed to stop runtime-smoke Vite process", error);
  }
}
