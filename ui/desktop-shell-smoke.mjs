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

const vite = spawn(process.execPath, [path.join(uiRoot, "node_modules", "vite", "bin", "vite.js"), "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: uiRoot,
  stdio: ["ignore", "pipe", "pipe"],
});
let viteOutput = "";
for (const stream of [vite.stdout, vite.stderr]) stream.on("data", (chunk) => { viteOutput = (viteOutput + chunk).slice(-4000); });

const browser = await chromium.launch({ headless: true, executablePath: chromeExecutable() });
const failures = [];
const evidence = [];
const profiles = [
  { name: "1024x768", width: 1024, height: 768, scale: 1 },
  { name: "1440x900", width: 1440, height: 900, scale: 1 },
  { name: "1920x1080", width: 1920, height: 1080, scale: 1 },
  { name: "windows-125", width: 1152, height: 720, scale: 1.25 },
  { name: "windows-150", width: 1280, height: 720, scale: 1.5 },
];

try {
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
    await page.goto(origin, { waitUntil: "networkidle" });
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
    } catch (error) {
      failures.push(`${profile.name}: ${error.message}`);
    } finally {
      await context.close();
    }
  }
} finally {
  await browser.close();
  vite.kill();
}

writeFileSync(path.join(outDir, "manifest.json"), JSON.stringify({ origin, profiles: evidence, failures }, null, 2));
if (failures.length > 0) {
  console.error(`desktop shell smoke failed:\n${failures.join("\n")}`);
  console.error(viteOutput);
  process.exit(1);
}
console.log(`desktop shell smoke passed (${evidence.length} profiles)`);
