// WebUI MVP PR B —— 零密钥 mock-provider 引擎闭环 smoke。
// 不依赖浏览器，也不依赖 Playwright/puppeteer——直接打 engine HTTP/SSE，
// 断言 engine 端真实持久化（ChatLog / persona / preset / session），而非看 DOM。
//
// 这覆盖 WEBUI-MVP-PLAN.md §1 判据 1-10 的引擎侧真相：
//   连接健康 → provider 验证 → 角色导入 → Persona/Preset → session 生命周期 →
//   三轮流式 RP → 刷新恢复（history 断言）→ regen/rollback → 删除会话 →
//   错误类型 → 跨 session 无串扰。
//
// 运行前置：engine（:8000）+ mock provider（:8889）已起。
//   完整一键：双击 webui/start.bat（它会起 mock + engine + webui，再单独跑 node webui/smoke.mjs）
//   只跑 smoke：先起 mock 与 engine，再 node webui/smoke.mjs
//
// 退出码：0 = 全绿；1 = 有断言失败（含详细 diff）。非零即验收未过。

import { writeFileSync } from 'node:fs';

const ENGINE = process.env.AIRP_ENGINE_URL || 'http://127.0.0.1:8000';
const MOCK = process.env.AIRP_MOCK_URL || 'http://127.0.0.1:8889';
const AUTH_HEADER = process.env.AIRP_AUTH_HEADER || '';
const KEEP_SESSION = process.env.AIRP_SMOKE_KEEP_SESSION === '1';
const RESULT_FILE = process.env.AIRP_SMOKE_RESULT_FILE || '';

// ── 断言工具 ─────────────────────────────────────────────────────────────
const failures = [];
let checks = 0;
function ok(cond, msg) {
  checks++;
  if (cond) {
    console.log('  ✓ ' + msg);
  } else {
    failures.push(msg);
    console.log('  ✗ ' + msg);
  }
}
function eq(actual, expected, msg) {
  checks++;
  const a = JSON.stringify(actual), e = JSON.stringify(expected);
  if (a === e) {
    console.log('  ✓ ' + msg);
  } else {
    failures.push(msg + `  actual=${a} expected=${e}`);
    console.log('  ✗ ' + msg + `  actual=${a} expected=${e}`);
  }
}

// ── HTTP 工具 ────────────────────────────────────────────────────────────
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// engine daemon 用 tower_governor 限流 10 req/s burst 20（daemon/mod.rs::create_router）。
// smoke 整链路 ~30+ 请求会打空 burst 桶；throttle 200ms（5 req/s）+ 关键节点前置 sleep 避误伤。
let lastReqAt = 0;
async function api(method, path, body, { bearer } = {}) {
  const wait = 200 - (Date.now() - lastReqAt);
  if (wait > 0) await sleep(wait);
  lastReqAt = Date.now();
  const headers = { 'Content-Type': 'application/json' };
  if (bearer) headers['Authorization'] = 'Bearer ' + bearer;
  else if (AUTH_HEADER) headers['Authorization'] = AUTH_HEADER;
  const res = await fetch(ENGINE + path, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  let data = null;
  const text = await res.text();
  if (text) {
    try { data = JSON.parse(text); } catch { data = text; }
  }
  return { status: res.status, ok: res.ok, data, text };
}

// 消费 engine 的 SSE chat 流，收集所有 chunk event，返回完整文本。
// engine axum SSE 帧形（chat_pipeline.rs::chunks_result_to_events）：
//   `event: message\ndata: {"type":"body_chunk","text":"..."}\n\n`
//   `event: error\ndata: {"type":"body_chunk","text":"[Error/...]"}\n\n`
// UnpackedChunk 序列化为 `{type, text}`（xml_unpacker.rs `#[serde(tag="type", content="text")]`）。
async function streamChat(payload) {
  const startedAt = Date.now();
  const res = await fetch(ENGINE + '/v1/chat/completions', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(AUTH_HEADER ? { Authorization: AUTH_HEADER } : {}),
    },
    body: JSON.stringify(payload),
  });
  ok(res.ok, 'chat SSE 200 OK');
  if (!res.ok) return { chunks: [], text: '', error: await res.text() };
  if (!res.body) return { chunks: [], text: '', error: 'chat SSE response has no body' };

  const chunks = [];
  let text = '';
  let errorText = '';
  let readBatches = 0;
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    readBatches++;
    buf += decoder.decode(value, { stream: true });
    let idx;
    while ((idx = buf.indexOf('\n\n')) >= 0) {
      const frame = buf.slice(0, idx);
      buf = buf.slice(idx + 2);
      let evt = null, dta = null;
      for (const line of frame.split('\n')) {
        if (line.startsWith('event:')) evt = line.slice(6).trim();
        else if (line.startsWith('data:')) dta = line.slice(5).trim();
      }
      if (evt === 'message' && dta) {
        try {
          const obj = JSON.parse(dta);
          // body_chunk = 正常正文；think_chunk = 心理独白（也算流式产出，验收计数含它）
          if (obj && typeof obj.text === 'string' && (obj.type === 'body_chunk' || obj.type === 'think_chunk')) {
            chunks.push(obj.text);
            if (obj.type === 'body_chunk') text += obj.text;
          }
        } catch (e) {
          ok(false, 'chunk JSON 解析失败: ' + e.message);
        }
      } else if (evt === 'error' && dta) {
        try {
          const obj = JSON.parse(dta);
          if (obj && typeof obj.text === 'string') {
            errorText += obj.text;
          }
        } catch (e) {
          ok(false, 'error chunk JSON 解析失败: ' + e.message);
        }
      }
    }
  }
  return { chunks, text, error: errorText || null, readBatches, elapsedMs: Date.now() - startedAt };
}

async function waitFor(url, { name = '', timeoutMs = 10000 } = {}) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const r = await fetch(url, {
        headers: url.startsWith(ENGINE) && AUTH_HEADER ? { Authorization: AUTH_HEADER } : {},
      });
      if (r.ok) return true;
    } catch {}
    await sleep(250);
  }
  ok(false, name + ' 探活超时');
  return false;
}

// ── fixture：Tavern V2 角色卡（最小且合法）──────────────────────────────
const TAVERN_V2_CARD = {
  spec: 'chara_card_v2',
  spec_version: '2.0',
  data: {
    name: 'Lyra',
    description: '一位在旧街区长大的吟游诗人，话不多但观察敏锐。',
    personality: '沉静；用词克制；对陌生人保持礼貌的距离感。',
    scenario: '暮色中的旧街区，空气里有旧书的气味。',
    first_mes: '（轻轻抬头）你来了。这条街的风今天有些不一样——像是有什么故事要被说出来。',
    mes_example: '{{user}}: 你常来这里吗？\n{{char}}: 多少年了。这条街比你们看起来更老。',
    creator_notes: 'smoke fixture',
    system_prompt: '你是 {{char}}，一位在旧街区生活的吟游诗人。保持克制、第二人称叙述。',
    post_history_instructions: '',
    alternate_greetings: [],
    character_book: null,
    tags: ['smoke'],
    creator: 'airp-smoke',
    character_version: '1',
    extensions: {},
  },
};

// ── fixture：Tavern 预设（最小且合法：prompts[] + model）──────────────────
const TAVERN_PRESET = {
  prompts: [
    { identifier: 'main', name: 'Main Prompt', content: 'You are an assistant.', role: 'system', injection_position: 'relative', injection_depth: 4 },
    { identifier: 'dialog', name: 'Dialogue Format', content: 'Respond in character.', role: 'system', injection_position: 'relative', injection_depth: 4 },
  ],
  temperature: 0.8,
  max_tokens: 512,
  model: 'airp-mock-1',
};

// ════════════════════════════════════════════════════════════════════════
// 链路
// ════════════════════════════════════════════════════════════════════════

console.log('AIRP WebUI PR B smoke');
console.log('  engine=' + ENGINE + '  mock=' + MOCK);
console.log();

// 判据 1：启动健康状态
await waitFor(MOCK + '/v1/models', { name: 'mock provider' });
await waitFor(ENGINE + '/health', { name: 'engine' });

{
  const h = await api('GET', '/health');
  ok(h.ok, '/health 200');
  ok(h.data && h.data.engine, 'health 含 engine 字段');
  // mock provider 已被 start.bat 配进 engine settings，故 provider_configured 应为 true
  ok(h.data && h.data.provider_configured === true, 'provider 已配置（指向 mock）');
}

// 判据 2：provider 通过真实 /v1/models 验证
{
  const m = await api('GET', '/v1/models');
  const models = Array.isArray(m.data?.data) ? m.data.data : [];
  if (!m.ok) {
    console.error(`  models proxy diagnostic: status=${m.status} code=${m.data?.error?.code || 'unknown'}`);
  }
  ok(m.ok, '/v1/models 200');
  ok(models.length >= 1, '/v1/models 返回 model 列表');
  ok(models.some((x) => x.id === 'airp-mock-1'), 'model 列表含 airp-mock-1');
}

// 判据 3：导入并选择角色（用 card_json fallback；smoke 是可信本地进程，但守 RR-001 不走 card_path）
const characterId = 'smoke-lyra';
{
  const imp = await api('POST', '/v1/characters/import', {
    character_id: characterId,
    card_json: JSON.stringify(TAVERN_V2_CARD),
  });
  ok(imp.ok, 'import V2 card 成功');
  eq(imp.data?.character_id, characterId, 'import 返回 character_id');

  const list = await api('GET', '/v1/characters');
  ok(list.ok, '/v1/characters list 200');
  // engine 返回裸字符串数组 ["id1","id2"]（handlers.rs::list_characters）
  const ids = Array.isArray(list.data) ? list.data : (list.data?.characters || []).map((c) => c.id || c.character_id);
  ok(ids.includes(characterId), 'character 列表包含导入的 Lyra');

  const card = await api('GET', '/v1/characters/' + characterId);
  ok(card.ok, '/v1/characters/:id card 200');
  ok(card.data?.data?.name === 'Lyra' || card.data?.name === 'Lyra', 'card 内容正确');
}

// 判据 5：持久化的基础 User Persona + 选择 Preset
const personaUser = 'smoke-user';
let personaRevision = 0;
{
  const init = await api('GET', '/v1/users/' + personaUser + '/persona');
  // 首次可能为 404（无 persona）；或已存在带 revision
  if (init.ok && typeof init.data?.revision === 'number') personaRevision = init.data.revision;

  const upd = await api('PUT', '/v1/users/' + personaUser + '/persona', {
    expected_revision: personaRevision,
    name: 'Tester',
    description: 'smoke harness',
    variables: { mood: 'curious' },
  });
  ok(upd.ok, 'persona PUT 成功');
  ok(typeof upd.data?.revision === 'number', 'persona 返回 revision');
  personaRevision = upd.data?.revision ?? personaRevision + 1;
  eq(upd.data?.name, 'Tester', 'persona name 落盘');

  const conflict = await api('PUT', '/v1/users/' + personaUser + '/persona', {
    expected_revision: 999, // 故意错的 revision
    name: 'Tester',
    description: '',
    variables: {},
  });
  ok(!conflict.ok && conflict.status === 400, 'persona revision 冲突返回 400');
}

// 判据 5b（A2a）：多 Persona CRUD（plural endpoints）
const multiPersonaUser = 'smoke-multi-persona';
{
  // 初始 list — 至少含 default
  const listInit = await api('GET', '/v1/users/' + multiPersonaUser + '/personas');
  ok(listInit.ok, 'plural persona list 200');
  const idsInit = Array.isArray(listInit.data) ? listInit.data : [];
  ok(idsInit.includes('default'), 'plural persona list 含 default');

  // 创建 default 应被拒绝
  const createDefault = await api('POST', '/v1/users/' + multiPersonaUser + '/personas', {
    persona_id: 'default',
    name: 'x',
    description: '',
    variables: {},
  });
  ok(!createDefault.ok && createDefault.status === 400, 'create default 被拒绝 400');

  // 创建一个非 default persona
  const pid = 'writer';
  const created = await api('POST', '/v1/users/' + multiPersonaUser + '/personas', {
    persona_id: pid,
    name: 'Writer',
    description: 'concise persona',
    variables: { tone: 'concise' },
  });
  ok(created.ok, 'create non-default persona 200');
  eq(created.data?.name, 'Writer', 'created persona name 落盘');
  ok(typeof created.data?.revision === 'number', 'created persona 有 revision');

  // list 应含 default + writer
  const listAfterCreate = await api('GET', '/v1/users/' + multiPersonaUser + '/personas');
  const idsAfterCreate = Array.isArray(listAfterCreate.data) ? listAfterCreate.data : [];
  ok(idsAfterCreate.includes('default') && idsAfterCreate.includes(pid), 'list 含 default + writer');

  // get 单个
  const got = await api('GET', '/v1/users/' + multiPersonaUser + '/personas/' + pid);
  ok(got.ok, 'get non-default persona 200');
  eq(got.data?.variables?.tone, 'concise', 'get persona variables 正确');

  // get 不存在的 persona → 404
  const notFound = await api('GET', '/v1/users/' + multiPersonaUser + '/personas/nonexistent');
  ok(!notFound.ok && notFound.status === 404, 'get nonexistent persona 404');

  // update
  const rev = created.data?.revision ?? 0;
  const updated = await api('PUT', '/v1/users/' + multiPersonaUser + '/personas/' + pid, {
    expected_revision: rev,
    name: 'Writer Pro',
    description: 'updated',
    variables: { tone: 'precise' },
  });
  ok(updated.ok, 'update non-default persona 200');
  eq(updated.data?.name, 'Writer Pro', 'updated persona name 落盘');
  eq(updated.data?.variables?.tone, 'precise', 'updated persona variables 落盘');

  // revision conflict
  const conflictMulti = await api('PUT', '/v1/users/' + multiPersonaUser + '/personas/' + pid, {
    expected_revision: 999,
    name: 'stale',
    description: '',
    variables: {},
  });
  ok(!conflictMulti.ok && conflictMulti.status === 400, 'plural persona revision 冲突 400');

  // delete default 被拒绝
  const delDefault = await api('DELETE', '/v1/users/' + multiPersonaUser + '/personas/default');
  ok(!delDefault.ok && delDefault.status === 400, 'delete default 被拒绝 400');

  // delete writer
  const deleted = await api('DELETE', '/v1/users/' + multiPersonaUser + '/personas/' + pid);
  ok(deleted.ok, 'delete non-default persona 204');
  eq(deleted.status, 204, 'delete returns 204');

  // delete 后 list 不再含 writer
  const listAfterDelete = await api('GET', '/v1/users/' + multiPersonaUser + '/personas');
  const idsAfterDelete = Array.isArray(listAfterDelete.data) ? listAfterDelete.data : [];
  ok(!idsAfterDelete.includes(pid), 'delete 后 list 不含 writer');
  ok(idsAfterDelete.includes('default'), 'delete 后 list 仍含 default');

  // 重复 delete 非 default persona → idempotent 204（合同：删除缺失的非 default 返 204）
  const reDelete = await api('DELETE', '/v1/users/' + multiPersonaUser + '/personas/' + pid);
  ok(reDelete.ok, 're-delete non-default persona idempotent 204');
}

const presetId = 'smoke-preset-' + Date.now();
{
  const imp = await api('POST', '/v1/presets/import', {
    preset_id: presetId,
    preset_json: JSON.stringify(TAVERN_PRESET),
  });
  ok(imp.ok, 'import preset 成功');
  eq(imp.data?.prompts_count, 2, 'preset prompts_count=2');

  const list = await api('GET', '/v1/presets');
  ok(list.ok, '/v1/presets list 200');
  // engine 返回裸字符串数组 ["id1","id2"]（handlers.rs::list_presets_endpoint）
  const ids = Array.isArray(list.data) ? list.data : (list.data?.presets || []).map((p) => p.id || p.preset_id);
  ok(ids.includes(presetId), 'preset 列表包含导入项');

  const p = await api('GET', '/v1/presets/' + presetId);
  ok(p.ok, '/v1/presets/:id 200');
  // engine 返回裸 prompt 数组 [{identifier,...}]（handlers.rs::get_preset_endpoint）
  const prompts = Array.isArray(p.data) ? p.data : (p.data?.prompts || []);
  ok(prompts.length === 2, 'preset 内容含 2 个 prompt');
}

// 判据 4：创建、选择和删除会话；session-scoped 操作不串流/串历史
let sessionId = null;
{
  const create = await api('POST', '/v1/sessions/' + characterId);
  ok(create.ok, 'create session 成功');
  ok(typeof create.data === 'string' && create.data.length > 0, 'session 返回 UUID');
  sessionId = create.data;

  const list = await api('GET', '/v1/sessions/' + characterId);
  ok(list.ok, 'list sessions 200');
  // engine 返回裸字符串数组 ["uuid1","uuid2"]（handlers.rs::list_sessions_endpoint）
  const sids = Array.isArray(list.data) ? list.data : (list.data?.sessions || []).map((s) => (typeof s === 'string' ? s : s.id || s.session_id));
  ok(sids.includes(sessionId), 'session 列表包含新建 session');

  // 建第二个 session 验证"无串扰"（稍后在三轮聊天后用）
  const create2 = await api('POST', '/v1/sessions/' + characterId);
  ok(create2.ok, 'create 第二个 session 成功（用于串扰断言）');
  const otherSessionId = create2.data;
  ok(otherSessionId !== sessionId, '两个 session UUID 不同');
}

// 判据 6 + 7：连续三轮流式 RP，刷新后仍可从历史恢复
const sentMessages = ['你好，初次见面。', '今晚的风有什么不一样？', '你愿意带我去你说的那条街走走吗？'];
const replies = [];
for (let i = 0; i < sentMessages.length; i++) {
  const out = await streamChat({
    character_id: characterId,
    session_id: sessionId,
    // 不传 user_id：多租户隔离不在本里程碑，且 HistoryQuery 无 user_id 字段；
    // 若传了 user_id，finalize 会把消息落盘到 data/users/{uid}/ 而非全局 data/，
    // 导致 history 查询（走全局 root）读到 0 条——这是验收 smoke 的自造坑，非 engine bug。
    user_profile: { name: 'Tester', variables: { mood: 'curious' } },
    preset_id: presetId,
    message: sentMessages[i],
  });
  ok(out.chunks.length >= 1, `第 ${i + 1} 轮流式产生 ≥1 chunk`);
  ok(out.readBatches >= 2 && out.elapsedMs >= 30, `第 ${i + 1} 轮 SSE 经多次读取增量到达`);
  ok(out.text.length > 0, `第 ${i + 1} 轮回复非空`);
  replies.push(out.text);
}

// 刷新恢复 = 断言 engine 端真实 history（不等 DOM）
{
  const hist = await api('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId });
  ok(hist.ok, 'history 查询成功');
  const msgs = hist.data?.messages || [];
  // 三轮 user + 三轮 assistant = 6
  eq(msgs.length, 6, '三轮后 history 含 6 条消息（3 user + 3 assistant）');
  // 验消息顺序：user/assistant 交错
  const roles = msgs.map((m) => m.role);
  ok(roles[0] === 'user' && roles[1] === 'assistant', '消息顺序：第 1 轮 user→assistant');
  ok(roles[4] === 'user' && roles[5] === 'assistant', '消息顺序：第 3 轮 user→assistant');
  // 验用户消息内容真实落盘
  ok(msgs.filter((m) => m.role === 'user').map((m) => m.content).join('|') === sentMessages.join('|'),
    '三轮 user 消息内容真实落盘且顺序正确');
  // scope_session_id 暴露给前端以便关联（#85 O1）
  ok(hist.data?.scope_session_id === sessionId || hist.data?.session_id === sessionId,
    'history 暴露 scope session id');
  ok(Array.isArray(hist.data?.message_ids) && hist.data.message_ids.length === msgs.length,
    'history 暴露 durable message_ids 且与 messages 等长');

  const tail = await api('POST', '/v1/chat/history', {
    character_id: characterId, session_id: sessionId, limit: 2,
  });
  ok(tail.ok, 'history cursor 首屏查询成功');
  eq((tail.data?.messages || []).length, 2, 'history cursor 首屏按 limit 返回 2 条');
  eq(tail.data?.total, 6, 'history cursor total 保留完整消息数');
  ok(tail.data?.has_more === true && Boolean(tail.data?.oldest_id), 'history cursor 暴露 has_more/oldest_id');
  const earlier = await api('POST', '/v1/chat/history', {
    character_id: characterId, session_id: sessionId, limit: 2, before: tail.data.oldest_id,
  });
  ok(earlier.ok, 'history cursor 加载更早成功');
  eq((earlier.data?.messages || []).length, 2, 'history cursor 更早页长度正确');
}

// 判据 4 续：跨 session 无串扰——另一个 session 的 history 应为空（不含本 session 的消息）
{
  const otherList = await api('GET', '/v1/sessions/' + characterId);
  const others = (Array.isArray(otherList.data) ? otherList.data : (otherList.data?.sessions || []).map((s) => (typeof s === 'string' ? s : s.id || s.session_id)))
    .filter((s) => s !== sessionId);
  const otherId = others[0];
  const otherHist = await api('POST', '/v1/chat/history', { character_id: characterId, session_id: otherId });
  ok(otherHist.ok, '另一 session history 查询成功');
  eq((otherHist.data?.messages || []).length, 0, '另一 session 无消息（无跨 session 串扰）');
}

// 判据 7：regen 和 rollback；破坏性操作有确认
{
  // rollback 到第 4 条 durable ID：保留前 4 条，丢弃第 3 轮 user+assistant。
  const beforeRollback = await api('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId });
  const rollbackId = beforeRollback.data?.message_ids?.[3];
  ok(Boolean(rollbackId), 'rollback 取得 durable message ID');
  const rb = await api('POST', '/v1/chat/rollback', { character_id: characterId, session_id: sessionId, message_id: rollbackId });
  ok(rb.ok, 'rollback-by-ID 成功');
  eq((rb.data?.messages || []).length, 4, 'rollback 后剩 4 条消息');

  // regen 删除最后一条（剩 4 条中的最后一条 assistant）
  const rg = await api('POST', '/v1/chat/regen', { character_id: characterId, session_id: sessionId });
  ok(rg.ok, 'regen 成功');
  eq((rg.data?.messages || []).length, 3, 'regen 后剩 3 条消息');
}

// 判据 9：provider/HTTP/SSE 错误在 UI 中给可行动提示
// 这里只断言 engine 在错误路径下返回 typed error（UI 已有 formatError 展示路径）。
// 错误端点连发易撞 governor 限流（10/s burst 20）；链路末端 burst 桶已被打空，
// 先 sleep 4s 让令牌桶完全恢复，再每发前 sleep 300ms 避连发误伤。
{
  await sleep(4000);
  const bad = await api('GET', '/v1/no-such-endpoint');
  ok(!bad.ok && bad.status === 404, '未知 endpoint 返回 404');

  await sleep(300);
  const noChar = await api('GET', '/v1/characters/does-not-exist-zzz');
  ok(!noChar.ok && noChar.status === 404, '未知 character 返回 404');

  await sleep(300);
  const noPreset = await api('GET', '/v1/presets/no-such-preset-zzz');
  ok(!noPreset.ok && noPreset.status === 404, '未知 preset 返回 404');

  await sleep(300);
  const badPreset = await api('POST', '/v1/presets/import', { preset_id: 'bad-zzz-' + Date.now(), preset_json: 'not-json' });
  ok(!badPreset.ok && badPreset.status === 400, '非法 preset JSON 返回 400');
}

if (RESULT_FILE) {
  writeFileSync(RESULT_FILE, JSON.stringify({ character_id: characterId, session_id: sessionId }) + '\n');
}

// 判据 4 续：删除会话，且删除后不可再访问（确定性生命周期）。
// 链路末端 burst 桶已被前面请求打空，前置 sleep 让令牌桶恢复避 429 误伤。
if (!KEEP_SESSION) {
  await sleep(4000);
  const del = await api('DELETE', '/v1/sessions/' + characterId + '/' + sessionId);
  ok(del.ok, 'delete session 成功');

  // 删除后 list 不含该 session
  const list = await api('GET', '/v1/sessions/' + characterId);
  const ids = Array.isArray(list.data) ? list.data : (list.data?.sessions || []).map((s) => (typeof s === 'string' ? s : s.id || s.session_id));
  ok(!ids.includes(sessionId), '删除后 session 不在列表');

  // 删除后 history 也应空 / 404
  const hist = await api('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId });
  ok(!hist.ok || (hist.data?.messages || []).length === 0, '删除后 history 不可访问或为空');
}

// ════════════════════════════════════════════════════════════════════════
// 收尾
// ════════════════════════════════════════════════════════════════════════
console.log();
console.log(`checks=${checks}  failures=${failures.length}`);
if (failures.length === 0) {
  console.log('PASS —— WebUI MVP PR B 全链路断言通过');
  process.exit(0);
} else {
  console.log('FAIL —— 失败项:');
  for (const f of failures) console.log('  - ' + f);
  process.exit(1);
}
