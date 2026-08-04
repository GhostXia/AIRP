// SSE 事件合同一致性测试（webui 消费端）：
// 读取 protocol/sse-events.json 机器可读规格，断言 webui/assets/api-client.js
// 的 consumeSse 解析行为覆盖全部锁定判别符，并遵循 additive-only 规则
// （容忍未知 message data type 与未知字段）。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { AirpStreamError, consumeSse } = require('../assets/api-client.js');

const contract = JSON.parse(
  readFileSync(new URL('../../protocol/sse-events.json', import.meta.url), 'utf8'),
);

function response(body) {
  return new Response(new TextEncoder().encode(body), { status: 200 });
}

function sseFrame(event, payload) {
  return `event: ${event}\ndata: ${JSON.stringify(payload)}\n\n`;
}

// 按规格声明的字段类型生成示例负载（支持嵌套 fields 对象）。
function sampleValue(kind, label) {
  if (kind === 'string') return `示例-${label}`;
  if (kind === 'string[]') return ['选项甲', '选项乙'];
  if (kind === 'boolean') return false;
  if (kind && typeof kind === 'object' && kind.fields) return sampleFields(kind.fields, label);
  throw new Error(`规格含未知字段类型标记: ${JSON.stringify(kind)}`);
}

function sampleFields(fields, prefix) {
  const value = {};
  for (const [field, kind] of Object.entries(fields)) {
    value[field] = sampleValue(kind, `${prefix}.${field}`);
  }
  return value;
}

function samplePayload(typeName, spec) {
  return { type: typeName, ...sampleFields(spec.fields || {}, typeName) };
}

test('contract locks the closed event-name set and message discriminators', () => {
  assert.equal(contract.compatibility, 'additive-only');
  assert.deepEqual(contract.eventNames, ['message', 'error']);
  assert.equal(contract.events.message.dataDiscriminator, 'type');
  assert.deepEqual(
    Object.keys(contract.events.message.dataTypes).sort(),
    ['action_options', 'body_chunk', 'done', 'think_chunk'],
  );
  assert.ok(
    contract.extensionRules.some(rule => /未知/i.test(rule) || /unknown/i.test(rule)),
    'additive-only 规则必须写入规格说明',
  );
});

test('webui consumer dispatches every contract message data type', async () => {
  const { dataTypes } = contract.events.message;
  const chunkTypes = Object.keys(dataTypes).filter(type => type !== 'done');

  for (const typeName of chunkTypes) {
    const payload = samplePayload(typeName, dataTypes[typeName]);
    const chunks = [];
    let doneCount = 0;
    const body = sseFrame('message', payload) + sseFrame('message', { type: 'done' });
    const result = await consumeSse(response(body), {
      onChunk: chunk => chunks.push(chunk),
      onDone: () => { doneCount += 1; },
    });
    assert.equal(result.completed, true, `${typeName} 流应正常完成`);
    assert.equal(doneCount, 1, `${typeName} 后应收到 done 终态`);
    assert.equal(chunks.length, 1, `${typeName} 应作为增量帧交给 onChunk`);
    assert.equal(chunks[0].type, typeName);
  }
});

test('webui consumer tolerates unknown message data types (additive-only)', async () => {
  const chunks = [];
  const body =
    sseFrame('message', { type: 'future_chunk', text: '未来新增的类型', extra: 42 }) +
    sseFrame('message', { type: 'body_chunk', text: '正文' }) +
    sseFrame('message', { type: 'done' });
  const result = await consumeSse(response(body), {
    onChunk: chunk => chunks.push(chunk),
  });
  assert.equal(result.completed, true, '未知 type 不得导致解析失败');
  assert.deepEqual(chunks.map(chunk => chunk.type), ['future_chunk', 'body_chunk']);
});

test('webui consumer ignores unknown fields on locked frames (additive-only)', async () => {
  const chunks = [];
  const body =
    sseFrame('message', { type: 'body_chunk', text: '正文', latency_ms: 12, meta: { k: 1 } }) +
    sseFrame('message', { type: 'done', trace_id: 't-1' });
  const result = await consumeSse(response(body), { onChunk: chunk => chunks.push(chunk) });
  assert.equal(result.completed, true);
  assert.equal(chunks.length, 1);
  assert.equal(chunks[0].text, '正文');
});

test('webui consumer surfaces the contract error envelope', async () => {
  const errorSpec = contract.events.error.fields.error.fields;
  const errorPayload = {
    type: 'error',
    text: '上游超时',
    error: {
      code: 'upstream_timeout',
      message: '上游超时',
      retryable: true,
      commit_state: 'not_committed',
    },
  };
  // 规格锁定的结构化字段缺一不可地被传递到 AirpStreamError。
  for (const field of Object.keys(errorSpec)) {
    assert.ok(field in errorPayload.error, `测试负载应覆盖规格字段 ${field}`);
  }
  await assert.rejects(
    consumeSse(response(sseFrame('error', errorPayload)), {}),
    error =>
      error instanceof AirpStreamError &&
      error.code === 'upstream_timeout' &&
      error.retryable === true &&
      error.commitState === 'not_committed',
  );
});

test('webui consumer treats error event and error payload type consistently', async () => {
  const payload = {
    type: 'error',
    text: '失败',
    error: { code: 'upstream', message: '失败', retryable: false, commit_state: 'partially_committed' },
  };
  // event: message 但 payload.type=error 也必须按错误处理（api-client 双通道判定）。
  await assert.rejects(
    consumeSse(response(sseFrame('message', payload)), {}),
    error => error instanceof AirpStreamError && error.commitState === 'partially_committed',
  );
});
