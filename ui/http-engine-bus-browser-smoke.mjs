import assert from "node:assert/strict";
import { spawn } from "node:child_process";
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
