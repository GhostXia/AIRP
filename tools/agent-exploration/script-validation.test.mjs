import test from 'node:test';
import assert from 'node:assert/strict';
import { validateScript } from './script-validation.mjs';

const VALID_SCRIPT = `export async function run(ctx) {
  await ctx.harness.getDomSnapshot();
}`;

test('validateScript accepts a syntactically valid run(ctx) module', async () => {
  assert.deepEqual(await validateScript(VALID_SCRIPT), []);
});

test('validateScript rejects malformed module syntax before execution', async () => {
  const diagnostics = await validateScript('export async function run(ctx) {');
  assert.ok(diagnostics.some(diagnostic => diagnostic.startsWith('JavaScript module syntax check failed:')));
});

test('validateScript rejects a module without the required run(ctx) export', async () => {
  const diagnostics = await validateScript('export async function inspect(ctx) {}');
  assert.ok(diagnostics.includes('Missing required module export: export async function run(ctx)'));
});

test('validateScript rejects a non-async run export', async () => {
  const diagnostics = await validateScript('export function run(ctx) {}');
  assert.ok(diagnostics.includes('Missing required module export: export async function run(ctx)'));
});
