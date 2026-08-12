import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const page = await readFile(new URL('../screens/40-worldbook-graph.html', import.meta.url), 'utf8');
const runtime = await readFile(new URL('../assets/worldbook-graph.js', import.meta.url), 'utf8');
const styles = await readFile(new URL('../assets/worldbook-graph.css', import.meta.url), 'utf8');

test('worldbook graph exposes focusable node buttons beside the canvas', () => {
  assert.ok(page.indexOf('id="graph-canvas"') < page.indexOf('id="graph-node-list"'));
  assert.match(page, /<ul[^>]*id="graph-node-list"/);
  assert.match(page, /节点键盘入口/);
  assert.match(page, /使用 Tab 选择节点，按 Enter 查看同一节点详情/);
  assert.match(runtime, /function renderNodeAccess\(\)/);
  assert.match(runtime, /node\('button', 'btn btn-secondary graph-node-button'\)/);
  assert.match(runtime, /node\('li', 'graph-node-item'\)/);
  assert.match(runtime, /button\.setAttribute\('aria-label', '查看世界书节点：/);
  assert.match(runtime, /button\.addEventListener\('keydown'/);
  assert.match(runtime, /event\.key === 'Enter'/);
  assert.match(runtime, /showNodeDetail\(n\)/);
  assert.match(styles, /\.graph-node-button:focus-visible/);
});

test('worldbook graph cancels stale frames and ignores stale graph responses', () => {
  assert.match(runtime, /let animationFrameId = null/);
  assert.match(runtime, /function cancelSimulation\(\)/);
  assert.match(runtime, /cancelAnimationFrame\(animationFrameId\)/);
  assert.match(runtime, /let simulationGeneration = 0/);
  assert.match(runtime, /if \(generation !== simulationGeneration\) return;/);
  assert.match(runtime, /const requestId = \+\+graphRequestId/);
  assert.match(runtime, /if \(requestId !== graphRequestId \|\| requestedCharacterId !== characterId\) return;/);
  assert.match(runtime, /#graph-character'\)\.addEventListener\('change',[\s\S]*loadGraph\(\);/);
  assert.match(runtime, /function renderGraph\(graph\) \{\s*cancelSimulation\(\);/);
});

test('worldbook graph clears stale node detail before every graph load', () => {
  assert.match(runtime, /async function loadGraph\(\) \{\s*cancelSimulation\(\);\s*clearNodeDetail\(\);/);
  assert.match(runtime, /function clearNodeDetail\(\) \{[\s\S]*selectedNode = null;/);
  assert.match(runtime, /wrap\.hidden = true;/);
  assert.match(runtime, /content\.textContent = '';/);
});
