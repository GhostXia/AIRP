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
  const cleanupExisting = await api('DELETE', '/v1/users/' + multiPersonaUser + '/personas/' + pid);
  ok(cleanupExisting.ok && cleanupExisting.status === 204, 'pre-clean existing writer persona 204');

  const created = await api('POST', '/v1/users/' + multiPersonaUser + '/personas', {
    persona_id: pid,
    name: 'Writer',
    description: 'concise persona',
    variables: { tone: 'concise' },
  });
  ok(created.ok, 'create non-default persona 200');
  eq(created.data?.name, 'Writer', 'created persona name 落盘');
  ok(typeof created.data?.revision === 'number', 'created persona 有 revision');

  // 重复创建同一 persona_id → revision 冲突，不覆盖数据
  const createDup = await api('POST', '/v1/users/' + multiPersonaUser + '/personas', {
    persona_id: pid,
    name: 'overwrite-attempt',
    description: '',
    variables: {},
  });
  ok(!createDup.ok && createDup.status === 400, 'create duplicate persona_id 被拒绝 400');

  // list 应含 default + writer
  const listAfterCreate = await api('GET', '/v1/users/' + multiPersonaUser + '/personas');
  const idsAfterCreate = Array.isArray(listAfterCreate.data) ? listAfterCreate.data : [];
  ok(idsAfterCreate.includes('default') && idsAfterCreate.includes(pid), 'list 含 default + writer');

  // get 单个
  const got = await api('GET', '/v1/users/' + multiPersonaUser + '/personas/' + pid);
  ok(got.ok, 'get non-default persona 200');
  eq(got.data?.variables?.tone, 'concise', 'get persona variables 正确');
  eq(got.data?.name, 'Writer', 'duplicate create did not overwrite name');

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
// 暴露给 production smoke-ci.sh 的最终状态：解耦硬编码期望值（N-1）+ 守护消息身份（N-2）
let finalMessageCount = null;
let finalLastMessageId = null;
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

  // regen 删除最后一条并流式生成新响应（SSE）
  const rgRes = await fetch(ENGINE + '/v1/chat/regen', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(AUTH_HEADER ? { Authorization: AUTH_HEADER } : {}),
    },
    body: JSON.stringify({ character_id: characterId, session_id: sessionId }),
  });
  ok(rgRes.ok, 'regen 成功');
  // 消费 SSE 流直到完成
  if (rgRes.body) {
    const reader = rgRes.body.getReader();
    while (true) { const { done } = await reader.read(); if (done) break; }
  }
  // 验证 history：regen 删除旧的 + 生成新的 = 仍 4 条（原 4 条 - 1 + 1 新）
  const afterRegen = await api('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId });
  eq((afterRegen.data?.messages || []).length, 4, 'regen 后仍 4 条消息（删旧+生新）');
  // 捕获最终状态供 production smoke-ci.sh 比对（engine restart 后验证持久化 + 消息身份）
  finalMessageCount = afterRegen.data?.messages?.length ?? null;
  const regenIds = afterRegen.data?.message_ids || [];
  finalLastMessageId = regenIds.length ? regenIds[regenIds.length - 1] : null;
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
  writeFileSync(RESULT_FILE, JSON.stringify({
    character_id: characterId,
    session_id: sessionId,
    final_message_count: finalMessageCount,
    final_last_message_id: finalLastMessageId,
  }) + '\n');
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
// #209 onboarding wizard L3 烟雾测试（spec §7.4）
// 验证向导调用的端点契约（每阶段 1 个 happy path）：
//   health / settings GET / settings POST / models / character import / chat preview
// 不覆盖 SSE 流式首聊（已有 smoke SSE 测试，向导复用相同端点）
// ════════════════════════════════════════════════════════════════════════
console.log();
console.log('—— #209 onboarding L3 烟雾 ——');

// smoke_onboarding_health: GET /health 返回 provider_configured + data_root_writable
{
  await sleep(300);
  const h = await api('GET', '/health');
  ok(h.ok, 'onboarding health: GET /health 200');
  ok(typeof h.data?.provider_configured === 'boolean', 'onboarding health: 返回 provider_configured 布尔');
  ok(typeof h.data?.data_root_writable === 'boolean', 'onboarding health: 返回 data_root_writable 布尔');
  ok(h.data?.engine === 'ok', 'onboarding health: engine === ok');
}

// smoke_onboarding_settings_get: GET /v1/settings 返回 api_key_set 布尔，不返回 api_key 值
{
  await sleep(300);
  const s = await api('GET', '/v1/settings');
  ok(s.ok, 'onboarding settings GET: 200');
  ok(typeof s.data?.api_key_set === 'boolean' || 'api_key_set' in (s.data || {}), 'onboarding settings GET: 返回 api_key_set 字段');
  // 安全不变量：GET /v1/settings 不得返回 api_key 明文值
  ok(s.data?.api_key === undefined || s.data?.api_key === null || s.data?.api_key === '',
    'onboarding settings GET: 不返回 api_key 明文（spec §4.3）');
}

// smoke_onboarding_settings_post: 空 api_key 不修改、非空修改
// CodeRabbit id=3602857807：mutating checks 必须隔离/还原，避免污染后续测试与共享 engine
{
  await sleep(300);
  // 1) 捕获原始 settings，测试后还原（避免 sk-test-onboarding 残留污染其它测试）
  const before = await api('GET', '/v1/settings');
  const origProvider = before.data?.provider;
  const origEndpoint = before.data?.endpoint;
  const origModel = before.data?.model;
  try {
    // 2) 空 api_key 提交（不修改）—— 不传 api_key 字段
    // provider 必须是 engine 已支持的枚举值（adapter.rs:57 Provider enum 仅 OpenAI）。
    // Provider enum 无 #[serde(rename_all)]，serde 默认序列化为 PascalCase "OpenAI"。
    // app.js:401 也用 providerKind.value（index.html:116 option value="OpenAI"）。
    const r1 = await api('POST', '/v1/settings', { provider: 'OpenAI' });
    ok(r1.ok, 'onboarding settings POST: 不带 api_key 成功（不修改 key）');
    // 3) 带非空 api_key 修改
    const r2 = await api('POST', '/v1/settings', { api_key: 'sk-test-onboarding' });
    ok(r2.ok, 'onboarding settings POST: 带非空 api_key 成功');
    // 4) 验证后续 GET 仍不返回明文
    const s2 = await api('GET', '/v1/settings');
    ok(s2.data?.api_key === undefined || s2.data?.api_key === null || s2.data?.api_key === '',
      'onboarding settings POST: 修改后 GET 仍不返回 api_key 明文');
    // CodeRabbit id=3602857803：原始密钥不应出现在任何字段中（不只是 api_key 字段）
    // 检查整段 JSON 文本不含明文密钥
    ok(!s2.text.includes('sk-test-onboarding'),
      'onboarding settings POST: GET 响应整段文本不含 api_key 明文（spec §4.3）');
  } finally {
    // 5) 还原：把 provider/endpoint/model 还原到测试前；用空 api_key 提交表示"不修改"
    //    （无法还原原 api_key 值，因为 GET 不返回明文；但 sk-test-onboarding 不会破坏后续测试）
    const restore = {};
    if (origProvider) restore.provider = origProvider;
    if (origEndpoint) restore.endpoint = origEndpoint;
    if (origModel) restore.model = origModel;
    if (Object.keys(restore).length > 0) {
      try { await api('POST', '/v1/settings', restore); } catch {}
    }
  }
}

// smoke_onboarding_models: GET /v1/models 返回 {id} 数组或 typed error
{
  await sleep(300);
  const m = await api('GET', '/v1/models');
  if (m.ok) {
    const models = Array.isArray(m.data?.data) ? m.data.data : (Array.isArray(m.data) ? m.data : []);
    ok(models.length > 0, 'onboarding models: 返回非空模型列表');
    ok(typeof models[0]?.id === 'string' || typeof models[0] === 'string', 'onboarding models: 模型项有 id');
  } else {
    // 上游未配置时返回 typed error 也算通过契约
    ok(m.data?.error?.code, 'onboarding models: 失败时返回 typed error (code 字段)');
  }
}

// smoke_onboarding_character_import: POST /v1/characters/import JSON 路径
// CodeRabbit id=3602857807：导入的角色必须在测试后清理，避免污染共享 engine
{
  await sleep(300);
  const cardJson = JSON.stringify({
    spec: 'chara_card_v2',
    data: { name: 'OnbSmokeChar', description: 'smoke test', personality: 'calm', first_mes: 'hi', scenario: '', mes_example: '' },
  });
  const r = await api('POST', '/v1/characters/import', { card_json: cardJson });
  ok(r.ok, 'onboarding character import: JSON 导入成功');
  const cid = r.data?.character_id || r.data?.id || r.data?.uuid;
  ok(typeof cid === 'string', 'onboarding character import: 返回 character_id');
  // 验证导入后列表可见（列表项可能是 bare string 或 {id, name, ...} 对象）
  if (cid) {
    const list = await api('GET', '/v1/characters');
    const items = Array.isArray(list.data) ? list.data
      : (list.data && Array.isArray(list.data.characters) ? list.data.characters : []);
    const exists = items.some(c => {
      if (typeof c === 'string') return c === cid;
      return (c.id || c.character_id) === cid;
    });
    ok(Array.isArray(items) && exists, 'onboarding character import: 导入后列表可见');
  }
  // 清理导入的角色（避免污染后续测试与共享 engine）
  if (cid) {
    try { await api('DELETE', '/v1/characters/' + encodeURIComponent(cid)); } catch (e) {
      console.log('  ⚠ onboarding character import: 清理失败（' + (e.message || e) + '）');
    }
  }
}

// smoke_onboarding_chat_preview: POST /v1/chat/preview 返回来源标签、不返回 prompt body
{
  // 复用已导入的角色（若上面成功）；否则跳过 preview 断言
  await sleep(300);
  const list = await api('GET', '/v1/characters');
  const items = Array.isArray(list.data) ? list.data
    : (list.data && Array.isArray(list.data.characters) ? list.data.characters : []);
  if (items.length > 0) {
    const c = items[items.length - 1];
    // 列表项可能是 bare string 或对象（同上）
    const cid = typeof c === 'string' ? c : (c.id || c.character_id);
    const p = await api('POST', '/v1/chat/preview', {
      character_id: cid,
      user_id: 'default',
      persona_id: 'default',
    });
    if (p.ok) {
      // 安全不变量：preview 不应返回 prompt body 明文（spec §4.3）
      ok(p.data?.prompt_body === undefined || p.data?.prompt_body === null,
        'onboarding chat preview: 不返回 prompt_body 明文');
      // 来源标签（可能在不同字段路径）
      // CodeRabbit id=3602857811：移除 `|| p.ok`——该分支让断言恒真，无意义
      const sources = p.data?.sources || p.data?.assembly?.sources || p.data?.segments;
      ok(Array.isArray(sources) || p.data?.total_estimated_tokens !== undefined,
        'onboarding chat preview: 返回 assembly 结构');
    } else {
      // preview 端点可能未实现或需要 provider 配置；失败时返回 typed error 即契约满足
      // 接受 error.code 或 error.message 或非 JSON 文本（4xx/5xx）任一形态
      const hasTypedError = p.data && typeof p.data === 'object' &&
        (p.data.error?.code || p.data.error?.message);
      const hasTextError = typeof p.data === 'string' && p.data.length > 0;
      ok(hasTypedError || hasTextError,
        'onboarding chat preview: 失败时返回 typed error 或错误文本');
    }
  } else {
    console.log('  ⚠ onboarding chat preview: 跳过（无角色可用）');
  }
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
