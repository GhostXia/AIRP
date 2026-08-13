import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const uiRoot = path.dirname(fileURLToPath(import.meta.url));
const outFlag = process.argv.indexOf("--out");
const outDir = path.resolve(outFlag >= 0 ? process.argv[outFlag + 1] : path.join(uiRoot, "dist", "shell-smoke"));
const port = 1421;
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

let vite = null;
let browser = null;
let viteOutput = "";
const failures = [];
const evidence = [];
const supplementalEvidence = [];
const profiles = [
  { name: "1024x768", width: 1024, height: 768, scale: 1 },
  { name: "1440x900", width: 1440, height: 900, scale: 1 },
  { name: "1920x1080", width: 1920, height: 1080, scale: 1 },
  { name: "windows-125", width: 1152, height: 720, scale: 1.25 },
  { name: "windows-150", width: 1280, height: 720, scale: 1.5 },
];

try {
  vite = spawn(process.execPath, [path.join(uiRoot, "node_modules", "vite", "bin", "vite.js"), "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
    cwd: uiRoot,
    stdio: ["ignore", "pipe", "pipe"],
  });
  for (const stream of [vite.stdout, vite.stderr]) stream.on("data", (chunk) => { viteOutput = (viteOutput + chunk).slice(-4000); });
  browser = await chromium.launch({ headless: true, executablePath: chromeExecutable() });
  await waitForVite(vite);
  mkdirSync(outDir, { recursive: true });
  for (const profile of profiles) {
    const context = await browser.newContext({
      viewport: { width: profile.width, height: profile.height },
      deviceScaleFactor: profile.scale,
      reducedMotion: "reduce",
    });
    const page = await context.newPage();
    const pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await page.goto(`${origin}/?airp_agent_test=1`, { waitUntil: "networkidle" });
    try {
      await page.locator("main.desktop-shell").waitFor({ state: "visible" });
      assert.equal(await page.locator('nav[aria-label="工作区"]').count(), 1);
      assert.equal(await page.locator('section[aria-label="动态 Surface 容器"]').count(), 1);
      const unnamedButtons = await page.locator("button").evaluateAll((buttons) => buttons.filter((button) => {
        const label = button.getAttribute("aria-label") || button.textContent || "";
        return label.trim().length === 0;
      }).length);
      assert.equal(unnamedButtons, 0, "every button needs an accessible name");
      const overflow = await page.evaluate(() => ({
        page: document.documentElement.scrollWidth - document.documentElement.clientWidth,
        shell: document.querySelector(".desktop-shell").scrollWidth - document.querySelector(".desktop-shell").clientWidth,
      }));
      assert.ok(overflow.page <= 1 && overflow.shell <= 1, `horizontal overflow: ${JSON.stringify(overflow)}`);

      const active = page.locator('[aria-current="page"]');
      await active.focus();
      await page.keyboard.press("ArrowDown");
      await page.waitForFunction(() => document.querySelector('[aria-current="page"]')?.textContent?.includes("世界"));
      assert.match(await page.locator('[aria-current="page"]').getAttribute("aria-label"), /^世界/);
      await page.keyboard.press("ArrowUp");
      await page.waitForFunction(() => document.querySelector('[aria-current="page"]')?.textContent?.includes("故事"));

      const file = path.join(outDir, `${profile.name}.png`);
      const image = await page.screenshot({ path: file });
      assert.ok(image.byteLength > 15_000, `screenshot pixel evidence too small: ${image.byteLength} bytes`);
      assert.deepEqual(pageErrors, []);
      evidence.push({ ...profile, screenshotBytes: image.byteLength, overflow });

      if (profile.name === "1024x768") {
        const contextToggle = page.locator('button[aria-label="切换上下文检查器"]');
        await page.waitForFunction(() => document.querySelector('button[aria-label="切换上下文检查器"]')?.getAttribute("aria-pressed") === "false");
        await contextToggle.click();
        await page.locator("#context-inspector-body").waitFor({ state: "visible" });
        const inspectorBox = await page.locator("aside.inspector").boundingBox();
        assert.ok(inspectorBox, "context inspector has no layout box");
        assert.ok(
          Math.abs(inspectorBox.y) <= 1 && Math.abs(inspectorBox.height - profile.height) <= 1,
          `context inspector must span the viewport: ${JSON.stringify(inspectorBox)}`,
        );
        const contextFile = path.join(outDir, "1024x768-context-open.png");
        const contextImage = await page.screenshot({ path: contextFile });
        assert.ok(contextImage.byteLength > 15_000, "context-open screenshot is empty");
        supplementalEvidence.push({ name: "1024x768-context-open", screenshotBytes: contextImage.byteLength });
        await page.locator(".inspector__toggle").click();

        const focusToggle = page.locator(".rail__focus");
        await focusToggle.click();
        const focusColumns = await page.locator("main.desktop-shell").evaluate((shell) =>
          getComputedStyle(shell).gridTemplateColumns.split(/\s+/).filter(Boolean),
        );
        assert.equal(focusColumns.length, 2, `compact Focus Mode must have two columns: ${focusColumns.join(" ")}`);
        await focusToggle.click();

        await page.evaluate(() => window.__AIRP_AGENT_TEST__?.setBusError("Engine 暂时不可用；请检查服务后重试。"));
        await page.setViewportSize({ width: 1024, height: 600 });
        const alert = page.locator('[role="alert"]');
        await alert.waitFor({ state: "visible" });
        await alert.locator('button[aria-label="重试 Engine 连接"]').waitFor({ state: "visible" });
        const errorFile = path.join(outDir, "1024x600-error.png");
        const errorImage = await page.screenshot({ path: errorFile });
        assert.ok(errorImage.byteLength > 15_000, "short-viewport error screenshot is empty");
        supplementalEvidence.push({ name: "1024x600-error", screenshotBytes: errorImage.byteLength });
      }

      if (profile.name === "1440x900") {
        const contextToggle = page.locator('button[aria-label="切换上下文检查器"]');
        await contextToggle.focus();
        const focusFile = path.join(outDir, "1440x900-keyboard-focus.png");
        const focusImage = await page.screenshot({ path: focusFile });
        assert.ok(focusImage.byteLength > 15_000, "keyboard-focus screenshot is empty");
        supplementalEvidence.push({ name: "1440x900-keyboard-focus", screenshotBytes: focusImage.byteLength });
      }
    } catch (error) {
      failures.push(`${profile.name}: ${error.message}`);
    } finally {
      await context.close();
    }
  }
} finally {
  if (browser) await browser.close();
  if (vite) vite.kill();
}

writeFileSync(path.join(outDir, "manifest.json"), JSON.stringify({ origin, profiles: evidence, supplemental: supplementalEvidence, failures }, null, 2));
if (failures.length > 0) {
  console.error(`desktop shell smoke failed:\n${failures.join("\n")}`);
  console.error(viteOutput);
  process.exit(1);
}
console.log(`desktop shell smoke passed (${evidence.length} profiles)`);
