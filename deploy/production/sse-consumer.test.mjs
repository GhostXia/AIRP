import test from 'node:test';
import assert from 'node:assert/strict';
import { consumeGenerationSse } from './sse-consumer.mjs';

function responseFrom(...parts) {
  const encoded = parts.map(part => new TextEncoder().encode(part));
  return new Response(new ReadableStream({
    start(controller) {
      for (const part of encoded) controller.enqueue(part);
      controller.close();
    },
  }));
}

test('strict consumer accepts CRLF/multiline data and done-before-EOF', async () => {
  const result = await consumeGenerationSse(responseFrom(
    'event: message\r\ndata: {"type":"body_chunk","text":"hello"}\r\n\r\n',
    'event: message\r\ndata: {"type":\r\ndata: "done"}\r\n\r\n',
  ));
  assert.equal(result.terminal, 'done');
  assert.equal(result.text, 'hello');
  assert.equal(result.typedError, null);
});

test('strict consumer preserves typed errors instead of treating chunks as success', async () => {
  const result = await consumeGenerationSse(responseFrom(
    'event: message\ndata: {"type":"body_chunk","text":"partial"}\n\n',
    'event: error\ndata: {"type":"error","error":{"code":"upstream","message":"failed","commit_state":"partially_committed"}}\n\n',
  ));
  assert.equal(result.terminal, 'error');
  assert.equal(result.text, 'partial');
  assert.equal(result.typedError.code, 'upstream');
  assert.equal(result.typedError.commit_state, 'partially_committed');
});

test('strict consumer accepts only the typed cancellation terminal when enabled', async () => {
  const result = await consumeGenerationSse(responseFrom(
    'event: error\ndata: {"type":"error","error":{"code":"cancelled","message":"generation cancelled","commit_state":"partially_committed"}}\n\n',
  ), { allowCancellation: true });
  assert.equal(result.terminal, 'cancelled');
  assert.equal(result.typedError.code, 'cancelled');
  assert.equal(result.typedError.commit_state, 'partially_committed');
});

test('strict consumer rejects an unexpected cancellation error and early EOF', async () => {
  const unexpected = await consumeGenerationSse(responseFrom(
    'event: error\ndata: {"type":"error","error":{"code":"timeout","message":"timed out","commit_state":"partially_committed"}}\n\n',
  ), { allowCancellation: true });
  assert.equal(unexpected.terminal, 'invalid_error');

  const early = await consumeGenerationSse(responseFrom(
    'event: message\ndata: {"type":"body_chunk","text":"partial"}\n\n',
  ));
  assert.equal(early.terminal, 'eof');
  assert.match(early.error, /before a typed terminal/);
});

test('strict consumer rejects malformed JSON frames', async () => {
  const result = await consumeGenerationSse(responseFrom('event: message\ndata: {broken}\n\n'));
  assert.equal(result.terminal, 'malformed');
  assert.match(result.error, /JSON parse failed/);
});
