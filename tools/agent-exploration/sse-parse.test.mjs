import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  createSseContentParser,
  parseSseContent,
  SseProtocolError,
} from './sse-parse.mjs';

const runnerSource = await readFile(new URL('./runner.mjs', import.meta.url), 'utf8');

// #522/#532：engine SSE data 帧是 {type: "body_chunk", text}（protocol/sse-events.json），
// 不是 OpenAI 风格的 {role: "assistant", content}。成功响应还必须以 {type: "done"} 终止。

test('parseSseContent concatenates body_chunk text frames', () => {
  const sse =
    'data: {"type":"body_chunk","text":"Hello"}\n\n' +
    'data: {"type":"body_chunk","text":" world"}\n\n' +
    'data: {"type":"done"}\n\n';
  assert.equal(parseSseContent(sse), 'Hello world');
});

test('parseSseContent ignores think_chunk (not reply body)', () => {
  const sse =
    'data: {"type":"think_chunk","text":"internal"}\n\n' +
    'data: {"type":"body_chunk","text":"visible"}\n\n' +
    'data: {"type":"done"}\n\n';
  assert.equal(parseSseContent(sse), 'visible');
});

test('parseSseContent ignores non-JSON frames like [DONE] while requiring engine done', () => {
  const sse =
    'data: {"type":"body_chunk","text":"ok"}\n\n' +
    'data: [DONE]\n\n' +
    'data: {"type":"done"}\n\n';
  assert.equal(parseSseContent(sse), 'ok');
});

test('parseSseContent ignores legacy role/content frames (not engine contract)', () => {
  const sse =
    'data: {"role":"assistant","content":"legacy"}\n\n' +
    'data: {"type":"done"}\n\n';
  assert.equal(parseSseContent(sse), '');
});

test('typed engine error frames fail closed with structured details', () => {
  const sse =
    'event: message\ndata: {"type":"body_chunk","text":"partial"}\n\n' +
    'event: error\n' +
    'data: {"type":"error","text":"boom","error":{"code":"upstream","message":"boom","retryable":false,"commit_state":"partially_committed"}}\n\n';

  assert.throws(
    () => parseSseContent(sse),
    error => {
      assert.ok(error instanceof SseProtocolError);
      assert.equal(error.code, 'upstream');
      assert.equal(error.engineError.code, 'upstream');
      assert.equal(error.engineError.commit_state, 'partially_committed');
      assert.equal(error.details.content, 'partial');
      return true;
    },
  );
});

test('error payload without event:error fails closed as a schema mismatch', () => {
  const sse = 'data: {"type":"error","text":"boom","error":{}}\n\n';
  assert.throws(
    () => parseSseContent(sse),
    error => error instanceof SseProtocolError && error.code === 'schema_mismatch',
  );
});

test('EOF residual done frame is dispatched instead of being dropped', () => {
  const sse =
    'data: {"type":"body_chunk","text":"tail"}\n\n' +
    'event: message\ndata: {"type":"done"}';
  assert.equal(parseSseContent(sse), 'tail');
});

test('EOF residual body is retained in structured early-EOF error', () => {
  const sse = 'data: {"type":"body_chunk","text":"partial"}';
  assert.throws(
    () => parseSseContent(sse),
    error => {
      assert.ok(error instanceof SseProtocolError);
      assert.equal(error.code, 'eof');
      assert.equal(error.details.content, 'partial');
      return true;
    },
  );
});

test('stream parser handles transport chunk boundaries and CRLF framing', () => {
  const parser = createSseContentParser();
  parser.push('event: message\r\ndata: {"type":"body_chunk","text":"hel');
  parser.push('lo"}\r\n\r\nevent: message\r\ndata: {"type":"done"}');
  assert.deepEqual(parser.finish(), { content: 'hello' });
});

test('parseSseContent ignores non-data lines', () => {
  const sse =
    ': heartbeat\n' +
    'event: message\n' +
    'id: ignored\n' +
    'data: {"type":"body_chunk","text":"a"}\n\n' +
    'data: {"type":"done"}\n\n';
  assert.equal(parseSseContent(sse), 'a');
});

test('early EOF without a terminal frame is never treated as success', () => {
  assert.throws(
    () => parseSseContent('data: {"type":"body_chunk","text":"partial"}\n\n'),
    error => error instanceof SseProtocolError && error.code === 'eof',
  );
});

test('runner delegates SSE parsing to the shared implementation', () => {
  assert.match(runnerSource, /import\s*{\s*parseSseContent\s*}\s*from\s*['"]\.\/sse-parse\.mjs['"]/);
  assert.match(runnerSource, /return\s*\{\s*content:\s*parseSseContent\(sseText\)\s*\}/);
  assert.doesNotMatch(runnerSource, /JSON\.parse\(data\)/);
});
