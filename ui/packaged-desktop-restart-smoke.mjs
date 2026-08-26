import assert from "node:assert/strict";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import process from "node:process";
import { chromium } from "playwright-core";

const phase = process.argv[2];
const cdpUrl = process.env.AIRP_SMOKE_CDP_URL;
const origin = process.env.AIRP_SMOKE_ORIGIN;
const evidenceFile = process.env.AIRP_SMOKE_RESTART_EVIDENCE_FILE;
assert.ok(["before", "after"].includes(phase), "phase must be before or after");
for (const [name, value] of Object.entries({ cdpUrl, origin, evidenceFile })) {
  assert.ok(value, `${name} is required`);
}

async function connect() {
  let lastError;
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      return await chromium.connectOverCDP(cdpUrl);
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  throw new Error(`WebView2 CDP endpoint did not become ready: ${lastError}`);
}

async function findDesktopPage(browser) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const page = browser.contexts().flatMap((context) => context.pages())
      .find((candidate) => candidate.url().startsWith(origin));
    if (page) return page;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`WebView2 did not expose an AIRP page under ${origin}`);
}

async function bearer(page) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const token = await page.evaluate(() => sessionStorage.getItem("airp_bearer"));
    if (/^[0-9a-f]{32}$/.test(token ?? "")) return token;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error("desktop token was not bootstrapped into sessionStorage");
}

async function authorized(page, token, pathname, init = {}) {
  return page.evaluate(async ({ token, pathname, init }) => {
    const headers = new Headers(init.headers ?? {});
    headers.set("Authorization", `Bearer ${token}`);
    if (init.body !== undefined) headers.set("Content-Type", "application/json");
    const response = await fetch(pathname, {
      ...init,
      headers,
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    });
    const body = await response.json().catch(() => null);
    return { status: response.status, body };
  }, { token, pathname, init });
}

async function surface(page, token, characterId, sessionId) {
  const result = await authorized(
    page,
    token,
    `/v1/ui/surfaces/session/${encodeURIComponent(sessionId)}?character_id=${encodeURIComponent(characterId)}`,
  );
  assert.equal(result.status, 200, `Surface fetch failed: ${JSON.stringify(result.body)}`);
  return result.body.snapshot;
}

const browser = await connect();
try {
  const page = await findDesktopPage(browser);
  await page.waitForFunction(() => location.hash === "", null, { timeout: 20_000 });
  assert.equal(new URL(page.url()).hash, "", "desktop token fragment was not scrubbed");
  if (phase === "after") assert.ok(existsSync(evidenceFile), `restart evidence file missing: ${evidenceFile}`);
  const before = phase === "after" ? JSON.parse(readFileSync(evidenceFile, "utf8")) : null;
  if (phase === "after") {
    await page.getByText("Engine 已连接", { exact: true }).waitFor({ state: "visible", timeout: 20_000 });
    await page.waitForFunction(
      (oldToken) => sessionStorage.getItem("airp_bearer") !== oldToken,
      before.token,
      { timeout: 20_000 },
    );
  }
  const token = await bearer(page);

  if (phase === "before") {
    const characterId = `desktop-restart-${Date.now()}`;
    const imported = await authorized(page, token, "/v1/characters/import", {
      method: "POST",
      body: {
        character_id: characterId,
        card_json: JSON.stringify({
          spec: "chara_card_v2",
          data: { name: "Desktop Restart Smoke", first_mes: "Ready" },
        }),
      },
    });
    assert.equal(imported.status, 200, `character import failed: ${JSON.stringify(imported.body)}`);
    const created = await authorized(page, token, `/v1/sessions/${encodeURIComponent(characterId)}`, {
      method: "POST",
    });
    assert.equal(created.status, 200, `session create failed: ${JSON.stringify(created.body)}`);
    const sessionId = created.body;
    assert.equal(typeof sessionId, "string");
    await page.evaluate(({ characterId, sessionId }) => {
      sessionStorage.setItem("airp_character_id", characterId);
      sessionStorage.setItem("airp_session_id", sessionId);
    }, { characterId, sessionId });
    await page.goto(`${origin}/desktop/`, { waitUntil: "domcontentloaded" });
    await page.getByText("Engine 已连接", { exact: true }).waitFor({ state: "visible", timeout: 20_000 });

    const initial = await surface(page, token, characterId, sessionId);
    const memory = initial.blueprint.widgets.find((widget) => widget.id === "memory")?.props;
    const state = initial.blueprint.widgets.find((widget) => widget.id === "character-state")?.props;
    assert.equal(typeof memory?.content_hash, "string");
    assert.equal(Number.isInteger(state?.revision), true);
    const memoryDraft = "Unsaved memory draft across Engine recovery";
    const stateDraft = { ...state.state, restart_smoke: "saved-after-recovery" };
    await page.getByRole("tab", { name: "memory-node", exact: true }).click();
    await page.getByLabel("编辑未分类 resident memory").fill(memoryDraft);
    await page.getByLabel("编辑顶层 JSON object").fill(JSON.stringify(stateDraft, null, 2));
    writeFileSync(evidenceFile, JSON.stringify({
      token,
      characterId,
      sessionId,
      memoryDraft,
      stateDraft,
      initialMemory: memory.content,
      initialState: state.state,
    }));
    console.log("packaged desktop pre-restart credential/draft evidence captured");
  } else {
    assert.notEqual(token, before.token, "Engine recovery did not bootstrap a fresh desktop token");
    const oldProbe = await authorized(page, before.token, "/v1/characters");
    assert.equal(oldProbe.status, 401, "pre-restart in-memory desktop token remained valid");
    const newProbe = await authorized(page, token, "/v1/characters");
    assert.equal(newProbe.status, 200, "fresh desktop token was not accepted after Engine recovery");
    assert.ok(newProbe.body.includes(before.characterId), "durable character disappeared after Engine recovery");
    let recovered = await surface(page, token, before.characterId, before.sessionId);
    let memory = recovered.blueprint.widgets.find((widget) => widget.id === "memory")?.props;
    let state = recovered.blueprint.widgets.find((widget) => widget.id === "character-state")?.props;
    assert.equal(memory?.content, before.initialMemory, "unsaved memory draft was replayed during recovery");
    assert.deepEqual(state?.state, before.initialState, "unsaved Character State draft was replayed during recovery");
    assert.equal(await page.evaluate(() => sessionStorage.getItem("airp_character_id")), before.characterId);
    assert.equal(await page.evaluate(() => sessionStorage.getItem("airp_session_id")), before.sessionId);
    const memoryEditor = page.getByLabel("编辑未分类 resident memory");
    const stateEditor = page.getByLabel("编辑顶层 JSON object");
    assert.equal(await memoryEditor.inputValue(), before.memoryDraft, "Memory dirty draft was lost during recovery");
    assert.deepEqual(JSON.parse(await stateEditor.inputValue()), before.stateDraft,
      "Character State dirty draft was lost during recovery");
    await page.getByRole("button", { name: "保存", exact: true }).click();
    await page.getByText("已保存并刷新权威 Surface。", { exact: true }).waitFor({ state: "visible" });
    await page.getByRole("button", { name: "应用字段变更", exact: true }).click();
    await page.getByText("已保存并刷新权威 Surface。", { exact: true }).last().waitFor({ state: "visible" });
    recovered = await surface(page, token, before.characterId, before.sessionId);
    memory = recovered.blueprint.widgets.find((widget) => widget.id === "memory")?.props;
    state = recovered.blueprint.widgets.find((widget) => widget.id === "character-state")?.props;
    assert.equal(memory?.content, before.memoryDraft, "Memory draft did not save after recovery");
    assert.deepEqual(state?.state, before.stateDraft, "Character State draft did not save after recovery");
    const settings = await authorized(page, token, "/v1/settings");
    assert.equal(settings.status, 200);
    assert.equal(settings.body.api_key_set, false, "restart smoke unexpectedly used a provider credential");
    console.log("packaged desktop Engine restart credential/recovery smoke passed");
  }
} finally {
  await browser.close();
}
