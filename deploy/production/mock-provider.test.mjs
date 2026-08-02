import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';

const port = 19000 + Math.floor(Math.random() * 1000);
const provider = spawn(process.execPath, [fileURLToPath(new URL('./mock-provider.js', import.meta.url))], {
  env: {
    ...process.env,
    MOCK_PROVIDER_HOST: '127.0.0.1',
    MOCK_PROVIDER_PORT: String(port),
    MOCK_PROVIDER_HOLD_MAX_MS: '10000',
  },
  stdio: ['ignore', 'pipe', 'pipe'],
});

async function waitForProvider(timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/v1/models`);
      if (response.ok) return;
    } catch {}
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error('mock provider did not become ready');
}

test.after(async () => {
  if (!provider.killed) provider.kill();
  await Promise.race([
    once(provider, 'exit'),
    new Promise(resolve => setTimeout(resolve, 1000)),
  ]);
});

test('synthetic cancellation model flushes headers but holds the first token', async () => {
  await waitForProvider();
  const response = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model: 'airp-smoke-cancel-hold', messages: [] }),
  });
  assert.equal(response.status, 200);
  const reader = response.body.getReader();
  let timer;
  const first = await Promise.race([
    reader.read(),
    new Promise(resolve => { timer = setTimeout(() => resolve({ held: true }), 500); }),
  ]);
  clearTimeout(timer);
  assert.equal(first.held, true, 'hold gate must not emit a token before cancellation');
  await reader.cancel();

  const ordinary = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model: 'airp-mock-1', messages: [] }),
  });
  const ordinaryReader = ordinary.body.getReader();
  const firstOrdinary = await ordinaryReader.read();
  assert.equal(ordinary.status, 200);
  assert.match(new TextDecoder().decode(firstOrdinary.value), /data:/);
  await ordinaryReader.cancel();
});
