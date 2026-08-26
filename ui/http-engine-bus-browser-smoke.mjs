import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const uiRoot = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.dirname(uiRoot);
const webuiDir = process.env.AIRP_WEBUI_DIR
  || path.join(uiRoot, "src-tauri", "webui-bundle");
const engine = process.env.AIRP_ENGINE_BINARY
  || path.join(repoRoot, "target", "release", "airp-core.exe");
const chrome = [
  process.env.AIRP_CHROME_PATH,
  process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, "Google", "Chrome", "Application", "chrome.exe"),
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
].filter(Boolean).find(existsSync);
assert.ok(existsSync(engine), `Engine binary missing: ${engine}`);
assert.ok(existsSync(path.join(webuiDir, "desktop", "index.html")), "run npm run bundle:webui first");
assert.ok(chrome, "Chrome/Chromium not found; set AIRP_CHROME_PATH");

const root = mkdtempSync(path.join(tmpdir(), "airp-http-bus-smoke-"));
const data = path.join(root, "data");
const port = await new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const address = server.address();
    server.close(() => resolve(address.port));
  });
});
const origin = `http://127.0.0.1:${port}`;
const bearer = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
let child;
let browser;
let providerServer;
let providerCancelled = false;

async function request(method, pathname, body) {
  const response = await fetch(`${origin}${pathname}`, {
    method,
    headers: {
      Authorization: `Bearer ${bearer}`,
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const payload = await response.json().catch(() => null);
  assert.ok(response.ok, `${method} ${pathname} failed: ${response.status} ${JSON.stringify(payload)}`);
  return payload;
}

try {
  let providerRequests = 0;
  providerServer = http.createServer((_request, response) => {
    providerRequests += 1;
    const replies = ["Smoke reply", "Regenerated reply", " continued", " unfinished"];
    const logicalRequest = Math.floor((providerRequests - 1) / 2);
    const reply = replies[Math.min(logicalRequest, replies.length - 1)];
    response.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });
    response.write(`data: ${JSON.stringify({ choices: [{ delta: { content: reply } }] })}\n\n`);
    if (providerRequests === 7) {
      let completed = false;
      const timer = setTimeout(() => {
        completed = true;
        response.end("data: [DONE]\n\n");
      }, 5_000);
      response.once("close", () => {
        clearTimeout(timer);
        if (!completed) providerCancelled = true;
      });
    } else {
      response.end("data: [DONE]\n\n");
    }
  });
  await new Promise((resolve, reject) => {
    providerServer.once("error", reject);
    providerServer.listen(0, "127.0.0.1", resolve);
  });
  const providerAddress = providerServer.address();
  assert.equal(typeof providerAddress, "object");
  const providerEndpoint = `http://127.0.0.1:${providerAddress.port}/v1/chat/completions`;
  child = spawn(engine, ["daemon", "--port", String(port)], {
    cwd: root,
    env: {
      ...process.env,
      AIRP_DATA_DIR: data,
      AIRP_DESKTOP_WEBUI_DIR: webuiDir,
      AIRP_ACCESS_KEY: bearer,
      AIRP_ENDPOINT: providerEndpoint,
      AIRP_API_KEY: "smoke-provider-key",
      AIRP_MODEL: "smoke-model",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk) => { output = (output + chunk).slice(-6000); });
  }
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Engine exited early (${child.exitCode})\n${output}`);
    try {
      if ((await fetch(`${origin}/version`)).ok) break;
    } catch { /* retry */ }
    if (attempt === 99) throw new Error(`Engine did not become ready\n${output}`);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  const imported = await request("POST", "/v1/characters/import", {
    character_id: "smoke-alice",
    card_json: JSON.stringify({
      spec: "chara_card_v2",
      data: {
        name: "Smoke Alice",
        first_mes: "Hello",
        character_book: { entries: [{ keys: ["smoke"], content: "Smoke lore" }] },
      },
    }),
  });
  const sessionId = await request("POST", `/v1/sessions/${encodeURIComponent(imported.character_id)}`);
  await request("POST", "/v1/chat/history", {
    character_id: imported.character_id,
    session_id: sessionId,
    limit: 1,
  });
  const historyDir = path.join(data, "characters", imported.character_id, "sessions", sessionId, "history");
  assert.ok(existsSync(historyDir), `Engine did not create the canonical history path: ${historyDir}`);
  const historyLines = Array.from({ length: 5_000 }, (_, index) => JSON.stringify({
    role: index % 2 === 0 ? "user" : "assistant",
    content: `Durable history ${String(index + 1).padStart(4, "0")}`,
    id: `m${(index + 1).toString(16).padStart(32, "0")}`,
  }));
  writeFileSync(path.join(historyDir, "chat_log.jsonl"), `${historyLines.join("\n")}\n`);
  const widgetSource = `export default () => ({ mount(element, ctx) {
    globalThis.__airpSmokeMounts = (globalThis.__airpSmokeMounts || 0) + 1;
    let storageBlocked = false;
    let hostDomBlocked = false;
    try { sessionStorage.getItem("airp_bearer"); } catch { storageBlocked = true; }
    try { parent.document.body; } catch { hostDomBlocked = true; }
    element.textContent = JSON.stringify({ storageBlocked, hostDomBlocked,
      instanceId: ctx.instance.id, capabilities: ctx.capabilities,
      mountCalls: globalThis.__airpSmokeMounts });
  }});`;
  const widgetBytes = Buffer.from(widgetSource);
  const widgetSha = createHash("sha256").update(widgetBytes).digest("hex");
  const installedWidget = await request("POST", "/v1/extensions/install", {
    manifest: {
      type: "acme.desktop-smoke",
      version: "1.0.0",
      title: "Desktop sandbox smoke",
      host_api: "1",
      capabilities: ["read:state"],
      entry: { kind: "esm", source: "https://invalid.example/widget.js", sandbox: true },
    },
    files: [{
      path: "index.js",
      content_base64: widgetBytes.toString("base64"),
      sha256: widgetSha,
    }],
    slot: "workbench.grid",
  });
  await request("POST", `/v1/extensions/${encodeURIComponent(installedWidget.id)}/grants`, {
    action: "grant",
  });
  const desktopSession = await request("POST", "/v1/desktop-session");
  assert.equal(typeof desktopSession.token, "string");

  browser = await chromium.launch({ headless: true, executablePath: chrome });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const failures = [];
  const httpFailures = [];
  let intentRequests = 0;
  let cooperativeStopRequested = false;
  page.on("console", (message) => {
    if (message.type() === "error") failures.push(`${message.location().url}: ${message.text()}`);
  });
  page.on("requestfailed", (request) => {
    const expectedStopAbort = cooperativeStopRequested
      && new URL(request.url()).pathname === "/v1/ui/intents"
      && request.failure()?.errorText === "net::ERR_ABORTED";
    if (!expectedStopAbort) failures.push(`${request.url()}: ${request.failure()?.errorText}`);
  });
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/v1/ui/intents") intentRequests += 1;
  });
  page.on("response", (response) => {
    if (response.status() >= 400) httpFailures.push(`${response.status()} ${response.url()}`);
  });
  const firstSurfaceResponse = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return response.request().method() === "GET"
      && /^\/v1\/ui\/surfaces\/session\/[^/]+$/.test(url.pathname);
  });
  await page.goto(
    `${origin}/desktop/?character_id=${encodeURIComponent(imported.character_id)}&session_id=${encodeURIComponent(sessionId)}#airp-token=${desktopSession.token}`,
    { waitUntil: "domcontentloaded" },
  );
  await page.getByText("Engine 已连接").waitFor({ state: "visible" });
  await page.locator('[data-blueprint-version="2"]').waitFor({ state: "visible" });
  const firstSurface = await (await firstSurfaceResponse).json();
  const firstChat = firstSurface.snapshot.blueprint.widgets.find((widget) => widget.id === "chat")?.props;
  const firstMemory = firstSurface.snapshot.blueprint.widgets.find((widget) => widget.id === "memory")?.props;
  const firstCharacterState = firstSurface.snapshot.blueprint.widgets
    .find((widget) => widget.id === "character-state")?.props;
  assert.equal(firstChat.messages.length, 50, "initial Surface did not contain exactly the latest page");
  assert.equal(firstChat.messages[0].content, "Durable history 4951");
  assert.equal(firstChat.messages.at(-1).content, "Durable history 5000");
  assert.equal(firstChat.total, 5_000);
  assert.equal(firstChat.has_more, true);
  assert.equal(firstChat.oldest_id, "m00000000000000000000000000001357");
  assert.deepEqual(firstChat.context, {
    character_id: imported.character_id,
    session_id: sessionId,
    persona_id: null,
    persona_source: null,
    scene_id: null,
    worldbook_source_ids: [`character:${imported.character_id}`],
  });
  assert.deepEqual(firstMemory.source, {
    kind: "resident_memory",
    scope: "session",
    character_id: imported.character_id,
    session_id: sessionId,
  });
  assert.equal(typeof firstMemory.content_hash, "string");
  assert.equal(firstMemory.char_count, 0);
  assert.deepEqual(firstCharacterState.source, {
    kind: "character_state",
    scope: "character",
    character_id: imported.character_id,
  });
  assert.equal(Number.isInteger(firstCharacterState.revision), true);
  assert.equal(typeof firstCharacterState.state, "object");
  assert.match(await page.locator(".surface__kicker").innerText(), /^Surface \/ session:/i);
  assert.equal(await page.locator(".w-chat-composer input").isDisabled(), false,
    "core.chat must remain writable on the production Surface");
  assert.equal(await page.locator('[data-widget-instance="memory"] textarea').evaluate((element) => element.readOnly), false,
    "core.memory must be writable on the production Surface");
  assert.equal(await page.locator('[data-widget-instance="character-state"] textarea')
    .evaluate((element) => element.readOnly), false,
  "core.character-state must be writable on the production Surface");
  const contextLabels = await page.getByRole("list", { name: "当前对话上下文" })
    .locator(".context-chip").evaluateAll((chips) => chips.map((chip) => chip.getAttribute("aria-label")));
  assert.deepEqual(contextLabels, [
    `角色 ${imported.character_id}`,
    `会话 ${sessionId}`,
    `世界书 character:${imported.character_id}`,
  ], "real Engine context chips did not preserve authoritative stable identifiers");
  await page.evaluate(() => {
    globalThis.__airpChatHostBeforeStateWrites = document.querySelector(
      '[data-widget-instance="chat"] .widget-host',
    );
  });

  await page.getByRole("tab", { name: "memory-node", exact: true }).click();
  await page.getByText("未分类 resident memory", { exact: true }).waitFor({ state: "visible" });
  const memoryEditor = page.getByLabel("编辑未分类 resident memory");
  const memoryIntentRequest = page.waitForRequest((request) => {
    if (new URL(request.url()).pathname !== "/v1/ui/intents" || request.method() !== "POST") return false;
    return request.postDataJSON()?.name === "memory.replace";
  });
  const memorySurfaceResponse = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return response.request().method() === "GET"
      && /^\/v1\/ui\/surfaces\/session\/[^/]+$/.test(url.pathname);
  });
  await memoryEditor.fill("Smoke resident memory edit");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  assert.deepEqual((await memoryIntentRequest).postDataJSON().params, {
    content: "Smoke resident memory edit",
    expected_content_hash: firstMemory.content_hash,
  }, "memory editor did not use exact content-hash CAS");
  const memorySurface = await (await memorySurfaceResponse).json();
  const updatedMemory = memorySurface.snapshot.blueprint.widgets.find((widget) => widget.id === "memory")?.props;
  assert.equal(updatedMemory.content, "Smoke resident memory edit");
  assert.equal(updatedMemory.char_count, 26);
  await page.getByText("已保存并刷新权威 Surface。", { exact: true }).waitFor({ state: "visible" });
  assert.equal(await memoryEditor.inputValue(), "Smoke resident memory edit");

  const characterEditor = page.getByLabel("编辑顶层 JSON object");
  const nextCharacterState = { ...firstCharacterState.state, smoke_status: "edited" };
  const characterIntentRequest = page.waitForRequest((request) => {
    if (new URL(request.url()).pathname !== "/v1/ui/intents" || request.method() !== "POST") return false;
    return request.postDataJSON()?.name === "characterState.patch";
  });
  const characterSurfaceResponse = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return response.request().method() === "GET"
      && /^\/v1\/ui\/surfaces\/session\/[^/]+$/.test(url.pathname);
  });
  await characterEditor.fill(JSON.stringify(nextCharacterState, null, 2));
  await page.getByRole("button", { name: "应用字段变更", exact: true }).click();
  assert.deepEqual((await characterIntentRequest).postDataJSON().params, {
    expected_revision: firstCharacterState.revision,
    patch: [{ op: "add", path: "/smoke_status", value: "edited" }],
  }, "character-state editor did not send a top-level revisioned patch");
  const characterSurface = await (await characterSurfaceResponse).json();
  const updatedCharacterState = characterSurface.snapshot.blueprint.widgets
    .find((widget) => widget.id === "character-state")?.props;
  assert.equal(updatedCharacterState.state.smoke_status, "edited");
  assert.equal(updatedCharacterState.revision, firstCharacterState.revision + 1);
  await page.getByText("已保存并刷新权威 Surface。", { exact: true }).last().waitFor({ state: "visible" });
  assert.deepEqual(JSON.parse(await characterEditor.inputValue()), nextCharacterState);
  assert.deepEqual(await page.evaluate(() => {
    const before = globalThis.__airpChatHostBeforeStateWrites;
    const after = document.querySelector('[data-widget-instance="chat"] .widget-host');
    return { same: before === after, connected: before?.isConnected === true };
  }), { same: true, connected: true }, "state writes rebuilt the unrelated Chat Widget Host");
  assert.equal(providerRequests, 0, "Memory/State writes unexpectedly invoked the provider");

  await page.getByRole("tab", { name: "chat-node", exact: true }).click();
  const chatLog = page.locator(".w-chat-log");
  await page.getByText("Durable history 5000", { exact: true }).waitFor({ state: "visible" });
  assert.ok(await chatLog.locator(".msg").count() < 50, "initial virtual DOM rendered the full history page");
  const olderPageRequest = page.waitForRequest((request) => {
    if (new URL(request.url()).pathname !== "/v1/ui/intents" || request.method() !== "POST") return false;
    return request.postDataJSON()?.name === "chat.loadMore";
  });
  const olderPageResponse = page.waitForResponse((response) => {
    const request = response.request();
    if (new URL(response.url()).pathname !== "/v1/ui/intents" || request.method() !== "POST") return false;
    return request.postDataJSON()?.name === "chat.loadMore";
  });
  await chatLog.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });
  const olderPageParams = (await olderPageRequest).postDataJSON().params;
  assert.deepEqual(olderPageParams, { before: firstChat.oldest_id, limit: 50 },
    "scroll did not request exactly one 50-message cursor page");
  assert.equal((await olderPageResponse).ok(), true, "50-message cursor page request failed");
  await page.getByText("Durable history 4901", { exact: true }).waitFor({ state: "visible" });
  assert.ok(await chatLog.locator(".msg").count() < 50, "virtual DOM became unbounded after pagination");
  await page.evaluate(() => {
    globalThis.__airpUnrelatedHosts = Object.fromEntries(
      ["memory", "character-state", "activity"].map((id) => [
        id, document.querySelector(`[data-widget-instance="${id}"] .widget-host`),
      ]),
    );
  });
  await page.locator(".w-chat-composer input").fill("Hello Engine");
  await page.locator(".w-chat-composer").dispatchEvent("submit");
  await page.getByText("Smoke reply", { exact: true }).waitFor({ state: "visible" });
  assert.deepEqual(await page.evaluate(() => Object.entries(globalThis.__airpUnrelatedHosts).map(([id, before]) => {
    const after = document.querySelector(`[data-widget-instance="${id}"] .widget-host`);
    return { id, same: before === after, connected: before?.isConnected === true };
  })), [
    { id: "memory", same: true, connected: true },
    { id: "character-state", same: true, connected: true },
    { id: "activity", same: true, connected: true },
  ], "chat Surface patch rebuilt an unrelated Widget Host");
  await page.getByRole("button", { name: "重新生成" }).click();
  await page.getByText("Regenerated reply", { exact: true }).waitFor({ state: "visible" });
  await page.getByRole("button", { name: "上一个候选" }).click();
  await page.getByText("Smoke reply", { exact: true }).waitFor({ state: "visible" });
  await page.getByRole("button", { name: "继续", exact: true }).click();
  await page.getByText("Smoke reply continued", { exact: true }).waitFor({ state: "visible" });
  await page.getByRole("button", { name: "继续", exact: true }).click();
  await page.getByRole("button", { name: "停止" }).waitFor({ state: "visible" });
  await page.getByText("unfinished", { exact: true }).waitFor({ state: "visible" });
  cooperativeStopRequested = true;
  await page.getByRole("button", { name: "停止" }).click();
  await page.getByRole("button", { name: "停止" }).waitFor({ state: "hidden" });
  for (let attempt = 0; attempt < 100 && !providerCancelled; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.equal(providerCancelled, true, "stop did not close the active upstream provider stream");
  // The router-wide limiter covers Surface polling and all setup calls too;
  // allow its bucket to replenish before the one canonical history probe.
  await new Promise((resolve) => setTimeout(resolve, 2_500));
  const stoppedHistory = await request("POST", "/v1/chat/history", {
    character_id: imported.character_id,
    session_id: sessionId,
    limit: 50,
  });
  assert.equal(stoppedHistory.messages?.at(-1)?.content, "Smoke reply continued",
    "cancelled continue did not converge to the Engine's canonical history");
  assert.equal(new URL(page.url()).hash, "", "token fragment was not scrubbed");
  assert.equal(await page.evaluate(() => sessionStorage.getItem("airp_bearer")), desktopSession.token);
  const renewed = await page.evaluate(async () => {
    const oldToken = sessionStorage.getItem("airp_bearer");
    const response = await fetch("/v1/desktop-session/renew", {
      method: "POST", headers: { Authorization: `Bearer ${oldToken}` },
    });
    const body = await response.json();
    sessionStorage.setItem("airp_bearer", body.token);
    window.dispatchEvent(new CustomEvent("airp-bearer-renewed", { detail: { expires_in: body.expires_in } }));
    const probe = await fetch("/v1/characters", { headers: { Authorization: `Bearer ${body.token}` } });
    return { ok: response.ok && probe.ok, oldToken, newToken: body.token };
  });
  assert.equal(renewed.ok, true);
  assert.notEqual(renewed.newToken, renewed.oldToken, "desktop-session renewal did not rotate the token");

  const sandboxContract = await page.evaluate(async () => {
    const token = sessionStorage.getItem("airp_bearer");
    const headers = { Authorization: `Bearer ${token}` };
    const catalog = await fetch("/v1/extensions/catalog", { headers }).then((response) => response.json());
    const grants = await fetch("/v1/grants", { headers }).then((response) => response.json());
    const manifest = catalog.manifests.find((item) => item.type === "acme.desktop-smoke");
    const grant = grants.grants.find((item) => item.type === "acme.desktop-smoke");
    if (!manifest || !grant) throw new Error("installed widget missing from Engine catalog/grants");
    const session = crypto.randomUUID();
    const instanceId = "desktop-sandbox-smoke";
    const frameUrl = new URL("/assets/widgets/sandbox-frame.html", location.origin);
    frameUrl.searchParams.set("src", new URL(manifest.entry.source, location.origin).href);
    frameUrl.searchParams.set("origin", location.origin);
    frameUrl.searchParams.set("bridge_session", session);
    frameUrl.searchParams.set("instance_id", instanceId);
    const iframe = document.createElement("iframe");
    iframe.dataset.pr7Smoke = "true";
    iframe.setAttribute("sandbox", "allow-scripts");
    iframe.setAttribute("referrerpolicy", "no-referrer");
    iframe.src = frameUrl.href;
    document.body.appendChild(iframe);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("sandbox frame did not become ready")), 5_000);
      function onMessage(event) {
        const message = event.data;
        if (event.source !== iframe.contentWindow || message?.bridge_session !== session
          || message?.instance_id !== instanceId || message?.kind !== "ready") return;
        clearTimeout(timer);
        window.removeEventListener("message", onMessage);
        // A stale-session mount must be ignored before the valid mount arrives.
        iframe.contentWindow.postMessage({
          kind: "mount", instance: { id: instanceId, type: manifest.type },
          capabilities: grant.granted_capabilities,
          bridge_session: "stale-session", instance_id: instanceId,
        }, "*");
        iframe.contentWindow.postMessage({
          kind: "mount", instance: { id: instanceId, type: manifest.type },
          capabilities: grant.granted_capabilities,
          bridge_session: session, instance_id: instanceId,
        }, "*");
        resolve();
      }
      window.addEventListener("message", onMessage);
    });
    return {
      source: manifest.entry.source,
      sandbox: iframe.getAttribute("sandbox"),
      referrerPolicy: iframe.getAttribute("referrerpolicy"),
      granted: grant.granted_capabilities,
    };
  });
  assert.match(sandboxContract.source, /^\/extensions\/[0-9a-f]{64}\/index\.js$/,
    "Engine catalog did not pin the widget source to its package digest");
  assert.equal(sandboxContract.sandbox, "allow-scripts");
  assert.equal(sandboxContract.referrerPolicy, "no-referrer");
  assert.deepEqual(sandboxContract.granted, ["read:state"]);
  const sandboxBody = page.frameLocator('iframe[data-pr7-smoke="true"]').locator("body");
  await sandboxBody.getByText(/storageBlocked/).waitFor({ state: "visible" });
  const sandboxEvidence = JSON.parse(await sandboxBody.innerText());
  assert.deepEqual(sandboxEvidence, {
    storageBlocked: true,
    hostDomBlocked: true,
    instanceId: "desktop-sandbox-smoke",
    capabilities: ["read:state"],
    mountCalls: 1,
  });
  assert.equal(intentRequests, 9, "Surface write and Chat vertical slices did not dispatch every expected intent");
  assert.ok(providerRequests >= 7, "Chat vertical slice did not reuse the Engine provider pipeline");
  assert.equal(httpFailures.length, 0, `HTTP failures: ${httpFailures.join("\n")}`);
  assert.equal(failures.length, 0, `browser errors: ${failures.join("\n")}`);
  console.log("HttpEngineBus real-Engine browser smoke passed");
} finally {
  await browser?.close().catch(() => {});
  if (providerServer) {
    await new Promise((resolve) => {
      providerServer.close(resolve);
      providerServer.closeAllConnections();
    });
  }
  if (child && child.exitCode === null) {
    child.kill();
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
  }
  rmSync(root, { recursive: true, force: true });
}
