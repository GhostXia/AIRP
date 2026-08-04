// SSE 事件合同一致性测试（deploy 生产 smoke 消费端）：
// 读取 protocol/sse-events.json 机器可读规格，断言 sse-consumer.mjs 的解析
// 与合同一致：覆盖全部锁定的 message data type、错误 envelope 字段类型、
// additive-only 规则（容忍未知 type 与未知字段）。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { consumeGenerationSse } from './sse-consumer.mjs';

const contract = JSON.parse(
  readFileSync(new URL('../../protocol/sse-events.json', import.meta.url), 'utf8'),
);

function responseFrom(...parts) {
  const encoded = parts.map(part => new TextEncoder().encode(part));
  return new Response(new ReadableStream({
    start(controller) {
      for (const part of encoded) controller.enqueue(part);
      controller.close();
    },
  }));
}

function frame(event, payload) {
  return `event: ${event}\ndata: ${JSON.stringify(payload)}\n\n`;
}

function sampleValue(kind, label) {
  if (kind === 'string') return `sample-${label}`;
  if (kind === 'string[]') return ['option-a', 'option-b'];
  if (kind === 'boolean') return false;
  if (kind && typeof kind === 'object' && kind.fields) return sampleFields(kind.fields, label);
  throw new Error(`contract declares unknown field kind: ${JSON.stringify(kind)}`);
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

test('contract locks the event names, discriminators and additive-only rule', () => {
  assert.equal(contract.compatibility, 'additive-only');
  assert.deepEqual(contract.eventNames, ['message', 'error']);
  assert.equal(contract.events.message.dataDiscriminator, 'type');
  assert.deepEqual(
    Object.keys(contract.events.message.dataTypes).sort(),
    ['action_options', 'body_chunk', 'done', 'think_chunk'],
  );
  assert.deepEqual(Object.keys(contract.events.error.fields).sort(), ['error', 'text']);
  assert.deepEqual(
    Object.entries(contract.events.error.fields.error.fields).sort(),
    [['code', 'string'], ['commit_state', 'string'], ['message', 'string'], ['retryable', 'boolean']],
  );
});

test('strict consumer accepts every contract message data type', async () => {
  const { dataTypes } = contract.events.message;
  const result = await consumeGenerationSse(responseFrom(
    ...Object.keys(dataTypes)
      .filter(type => type !== 'done')
      .map(type => frame('message', samplePayload(type, dataTypes[type]))),
    frame('message', { type: 'done' }),
  ));
  assert.equal(result.terminal, 'done');
  assert.equal(result.chunks.length, 2, 'body_chunk 与 think_chunk 都计入 chunks');
  assert.equal(result.optionFrames, 1, 'action_options 帧被接受');
  assert.deepEqual(result.unknownTypes, []);
});

test('strict consumer tolerates unknown message data types and fields (additive-only)', async () => {
  const result = await consumeGenerationSse(responseFrom(
    frame('message', { type: 'future_chunk', text: 'new', extra: true }),
    frame('message', { type: 'body_chunk', text: 'ok', latency_ms: 3 }),
    frame('message', { type: 'done', trace_id: 't-9' }),
  ));
  assert.equal(result.terminal, 'done', '未知 type 不得中断流');
  assert.deepEqual(result.unknownTypes, ['future_chunk']);
  assert.equal(result.text, 'ok');
});

test('strict consumer validates the contract error envelope field types', async () => {
  const valid = await consumeGenerationSse(responseFrom(
    frame('error', {
      type: 'error',
      text: 'upstream failed',
      error: { code: 'upstream', message: 'upstream failed', retryable: false, commit_state: 'partially_committed' },
    }),
  ));
  assert.equal(valid.terminal, 'error');
  assert.equal(valid.typedError.retryable, false);

  for (const broken of [
    { type: 'error', error: { code: 'x', message: 'm', retryable: false, commit_state: 's' } }, // 缺顶层 text
    { type: 'error', text: 't', error: { code: 'x', message: 'm', commit_state: 's' } }, // 缺 retryable
    { type: 'error', text: 't', error: { code: 'x', message: 'm', retryable: 'yes', commit_state: 's' } }, // retryable 非 boolean
    { type: 'error', text: 't', error: { code: 'x', retryable: false, commit_state: 's' } }, // 缺 message
  ]) {
    const result = await consumeGenerationSse(responseFrom(frame('error', broken)));
    assert.equal(result.terminal, 'malformed_error', `残缺 envelope 必须被拒绝: ${JSON.stringify(broken)}`);
  }
});

test('strict consumer keeps unknown event names out of the closed set', async () => {
  // eventNames 是封闭集合（合同只允许 message data type 追加）。
  const result = await consumeGenerationSse(responseFrom('event: telemetry\ndata: {"type":"done"}\n\n'));
  assert.equal(result.terminal, 'invalid_event');
});
