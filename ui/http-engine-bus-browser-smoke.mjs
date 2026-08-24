import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
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
  child = spawn(engine, ["daemon", "--port", String(port)], {
    cwd: root,
    env: {
      ...process.env,
      AIRP_DATA_DIR: data,
      AIRP_DESKTOP_WEBUI_DIR: webuiDir,
      AIRP_ACCESS_KEY: bearer,
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
    card_json: JSON.stringify({ spec: "chara_card_v2", data: { name: "Smoke Alice", first_mes: "Hello" } }),
  });
  const sessionId = await request("POST", `/v1/sessions/${encodeURIComponent(imported.character_id)}`);
  const widgetSource = `export default () => ({ mount(element, ctx) {
    let storageBlocked = false;
    let hostDomBlocked = false;
    try { sessionStorage.getItem("airp_bearer"); } catch { storageBlocked = true; }
    try { parent.document.body; } catch { hostDomBlocked = true; }
    element.textContent = JSON.stringify({ storageBlocked, hostDomBlocked,
      instanceId: ctx.instance.id, capabilities: ctx.capabilities });
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
  // Force the Engine's bounded-props marker. Read-only authorization must be
  // explicit; it must never be inferred from the normal messages[] shape.
  const historyDir = path.join(data, "characters", imported.character_id, "sessions", String(sessionId), "history");
  mkdirSync(historyDir, { recursive: true });
  const now = new Date().toISOString();
  writeFileSync(path.join(historyDir, "chat_log_meta.json"), JSON.stringify({
    session_id: String(sessionId), character_id: imported.character_id,
    created_at: now, updated_at: now, revision: 1,
  }));
  writeFileSync(path.join(historyDir, "chat_log.jsonl"), `${JSON.stringify({
    role: "assistant", content: "x".repeat(210_000), id: "oversized-projection", ts: now,
  })}\n`);
  const desktopSession = await request("POST", "/v1/desktop-session");
  assert.equal(typeof desktopSession.token, "string");

  browser = await chromium.launch({ headless: true, executablePath: chrome });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const failures = [];
  const httpFailures = [];
  let intentRequests = 0;
  page.on("console", (message) => {
    if (message.type() === "error") failures.push(`${message.location().url}: ${message.text()}`);
  });
  page.on("requestfailed", (request) => failures.push(`${request.url()}: ${request.failure()?.errorText}`));
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/v1/ui/intents") intentRequests += 1;
  });
  page.on("response", (response) => {
    if (response.status() >= 400) httpFailures.push(`${response.status()} ${response.url()}`);
  });
  await page.goto(
    `${origin}/desktop/?character_id=${encodeURIComponent(imported.character_id)}&session_id=${encodeURIComponent(sessionId)}#airp-token=${desktopSession.token}`,
    { waitUntil: "domcontentloaded" },
  );
  await page.getByText("Engine 已连接").waitFor({ state: "visible" });
  await page.locator('[data-blueprint-version="2"]').waitFor({ state: "visible" });
  assert.match(await page.locator(".surface__kicker").innerText(), /^Surface \/ session:/i);
  assert.equal(await page.locator(".w-chat-composer input").isDisabled(), true,
    "truncated Engine projection must remain explicitly read-only");
  await page.locator(".w-chat-composer").dispatchEvent("submit");
  await page.waitForTimeout(50);
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
  });
  assert.equal(intentRequests, 0, "read-only /desktop/ path dispatched an intent request");
  assert.equal(httpFailures.length, 0, `HTTP failures: ${httpFailures.join("\n")}`);
  assert.equal(failures.length, 0, `browser errors: ${failures.join("\n")}`);
  console.log("HttpEngineBus real-Engine browser smoke passed");
} finally {
  await browser?.close().catch(() => {});
  if (child && child.exitCode === null) {
    child.kill();
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
  }
  rmSync(root, { recursive: true, force: true });
}
