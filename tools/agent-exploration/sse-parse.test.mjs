import test from 'node:test';
import assert from 'node:assert/strict';
import { parseSseContent } from './sse-parse.mjs';

// #522：sseCall 合同错位回归锁定——engine 的 SSE data 帧是
// {type: "body_chunk", text}（protocol/sse-events.json），不是
// OpenAI 风格的 {role: "assistant", content}。

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
    'data: {"type":"body_chunk","text":"visible"}\n\n';
  assert.equal(parseSseContent(sse), 'visible');
});

test('parseSseContent ignores non-JSON frames like [DONE]', () => {
  const sse = 'data: {"type":"body_chunk","text":"ok"}\n\n' + 'data: [DONE]\n\n';
  assert.equal(parseSseContent(sse), 'ok');
});

test('parseSseContent ignores legacy role/content frames (not engine contract)', () => {
  const sse = 'data: {"role":"assistant","content":"legacy"}\n\n';
  assert.equal(parseSseContent(sse), '');
});

test('parseSseContent handles error frames without throwing', () => {
  const sse = 'data: {"type":"error","text":"boom","error":{"code":"x","message":"x"}}\n\n';
  assert.equal(parseSseContent(sse), '');
});

test('parseSseContent ignores non-data lines', () => {
  const sse = 'event: message\n' + 'data: {"type":"body_chunk","text":"a"}\n\n';
  assert.equal(parseSseContent(sse), 'a');
});
