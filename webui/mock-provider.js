// 零密钥 OpenAI-compatible mock provider —— 仅用于 PR B 浏览器全链路验收。
// 不进 Cargo/npm workspace，零依赖纯 node（与 serve.js 同风格）。
//
// 实现两个 OpenAI 兼容端点：
//   GET  /v1/models            → {"data":[{"id":"..."}]}
//   POST /v1/chat/completions  → SSE 流式 data: {"choices":[{"delta":{"content":"..."}}]} ... data: [DONE]
//
// engine adapter 的 OpenAI 兼容路径（engine/src/adapter.rs::parse_openai_sse_line）
// 逐帧解析 `choices[0].delta.content`，遇到 `data: [DONE]` 结束。本 mock 严格对齐该契约。
//
// 启动：node webui/mock-provider.js   默认监听 127.0.0.1:8889
// 可用 env：MOCK_PROVIDER_PORT / MOCK_PROVIDER_HOST / MOCK_PROVIDER_MODEL
//
// 故意不依赖任何外部包：验收环境零安装、可复现，且不污染 D 盘工具链。

const http = require('http');
const https = require('https');
const fs = require('fs');

const PORT = parseInt(process.env.MOCK_PROVIDER_PORT || '8889', 10);
const HOST = process.env.MOCK_PROVIDER_HOST || '127.0.0.1';
const MODEL = process.env.MOCK_PROVIDER_MODEL || 'airp-mock-1';
const TLS_CERT_FILE = process.env.MOCK_PROVIDER_TLS_CERT_FILE || '';
const TLS_KEY_FILE = process.env.MOCK_PROVIDER_TLS_KEY_FILE || '';
if (Boolean(TLS_CERT_FILE) !== Boolean(TLS_KEY_FILE)) {
  throw new Error('MOCK_PROVIDER_TLS_CERT_FILE and MOCK_PROVIDER_TLS_KEY_FILE must be set together');
}

// 健鉴权：mock 不要求任何 key，但若调用方带了 Authorization 头则回显一行日志便于验收断言。
// 不做真鉴权——零密钥是本里程碑的硬约束（WEBUI-MVP-PLAN.md §1.2/§4 PR B）。

// ── 回复片段池 ─────────────────────────────────────────────────────────────
// 验收只需"三轮流式 RP + 刷新恢复 + regen/rollback"，不需要真实智能。
// 用一段固定 RP 风格的片段，逐 token 切片流式输出，让 engine adapter / FSM / unpacker 真跑一遍。
const REPLY_TEMPLATE =
  '（轻轻点头）你说得对。这里的风总是带着旧故事的气味——' +
  '我在这条街上走了很多年，却还是第一次觉得它值得停下来看一眼。\n' +
  '告诉我，你接下来想往哪走？';

// 按 UTF-8 codepoint 切片，避免把多字节字符拦腰切断（adapter 解析 JSON 时会拒 malformed）。
function chunkReply(text) {
  const chunks = [];
  // 用 spread 展开 surrogate-pair 正确的 codepoints，再用 Buffer.utf8ToBytes 量 Token 化
  const chars = [...text];
  // 每 2-4 个字符一帧：足够产生多帧流式效果，又不会把单帧 JSON 撑大。
  const stride = 3;
  for (let i = 0; i < chars.length; i += stride) {
    chunks.push(chars.slice(i, i + stride).join(''));
  }
  return chunks;
}

function sendJson(res, status, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Headers': 'Authorization, Content-Type',
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  });
  res.end(body);
}

function handleModels(res) {
  sendJson(res, 200, {
    object: 'list',
    data: [
      { id: MODEL, object: 'model', owned_by: 'airp-mock' },
      // 多列一个，让 WebUI 的 model 列表渲染有真数量，不是单行孤例。
      { id: MODEL + '-lite', object: 'model', owned_by: 'airp-mock' },
    ],
  });
}

function handleChatCompletions(req, res) {
  // 真实 SSE 响应。engine adapter 读字节流、按 `\n` 切行、解析 `data:` 前缀。
  res.writeHead(200, {
    'Content-Type': 'text/event-stream; charset=utf-8',
    'Cache-Control': 'no-cache, no-transform',
    'Connection': 'keep-alive',
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Headers': 'Authorization, Content-Type',
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  });

  // 首 chunk 带 role 元数据（对齐 OpenAI 真实流：第一帧常是 delta.role=assistant）。
  res.write('data: ' + JSON.stringify({
    id: 'chatcmpl-airp-mock',
    object: 'chat.completion.chunk',
    created: Math.floor(Date.now() / 1000),
    model: MODEL,
    choices: [{ index: 0, delta: { role: 'assistant' }, finish_reason: null }],
  }) + '\n\n');

  // 逐帧流式内容
  const chunks = chunkReply(REPLY_TEMPLATE);
  let i = 0;
  const timer = setInterval(() => {
    if (i < chunks.length) {
      res.write('data: ' + JSON.stringify({
        id: 'chatcmpl-airp-mock',
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model: MODEL,
        choices: [{ index: 0, delta: { content: chunks[i] }, finish_reason: null }],
      }) + '\n\n');
      i++;
    } else {
      clearInterval(timer);
      // 最后一帧带 finish_reason，再发 [DONE] 终止标记（adapter 据此结束流）。
      res.write('data: ' + JSON.stringify({
        id: 'chatcmpl-airp-mock',
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model: MODEL,
        choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
      }) + '\n\n');
      res.write('data: [DONE]\n\n');
      res.end();
    }
  }, 12); // 12ms/帧 ≈ 真实流式节奏，且整段 ~2s 内完成，不拖慢验收

  // 响应连接断开时清 timer。不能监听 req.close：POST 请求体读完就会触发，
  // 那会在 finish chunk / [DONE] 前误停流，导致 engine 无法 finalize assistant。
  res.on('close', () => clearInterval(timer));
}

const handler = (req, res) => {
  // CORS 预检
  if (req.method === 'OPTIONS') {
    res.writeHead(204, {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Headers': 'Authorization, Content-Type',
      'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    });
    res.end();
    return;
  }

  const url = req.url.split('?')[0];
  if (req.method === 'GET' && url === '/v1/models') return handleModels(res);
  if (req.method === 'POST' && url === '/v1/chat/completions') return handleChatCompletions(req, res);

  // 404 用 OpenAI 风格 JSON，便于 engine adapter 的 typed error 路径也被覆盖。
  sendJson(res, 404, { error: { message: 'unknown endpoint: ' + req.method + ' ' + url, type: 'not_found' } });
};
const tlsEnabled = Boolean(TLS_CERT_FILE);
const server = tlsEnabled
  ? https.createServer({ cert: fs.readFileSync(TLS_CERT_FILE), key: fs.readFileSync(TLS_KEY_FILE) }, handler)
  : http.createServer(handler);

server.listen(PORT, HOST, () => {
  console.log('AIRP mock provider on ' + (tlsEnabled ? 'https://' : 'http://') + HOST + ':' + PORT);
  console.log('  GET  /v1/models');
  console.log('  POST /v1/chat/completions  (SSE, model=' + MODEL + ')');
  console.log('零密钥；不做真鉴权。按 Ctrl+C 停止。');
});
