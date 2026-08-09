// Targeted responsive-height contracts for the issue #519 hotspots.
// This intentionally covers the cited graph/console/Vue components only;
// it is not a claim that every WebUI screen has been audited.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const graphCss = await readFile(new URL('../assets/worldbook-graph.css', import.meta.url), 'utf8');
const consoleCss = await readFile(new URL('../assets/console.css', import.meta.url), 'utf8');
const appVue = await readFile(new URL('../../ui/src/App.vue', import.meta.url), 'utf8');
const rendererVue = await readFile(new URL('../../ui/src/components/BlueprintRenderer.vue', import.meta.url), 'utf8');
const cardVue = await readFile(new URL('../../ui/src/widgets/CardWidget.vue', import.meta.url), 'utf8');

function cssBlock(source, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return source.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's'))?.[1] ?? '';
}

test('graph canvas keeps its intrinsic aspect ratio while sizing to the viewport', () => {
  const block = cssBlock(graphCss, '#graph-canvas');
  assert.ok(block, '#graph-canvas rule must remain present');
  assert.match(block, /height:\s*auto/);
  assert.match(block, /aspect-ratio:\s*3\s*\/\s*2/);
  assert.doesNotMatch(block, /height:\s*600px/);
  assert.match(graphCss, /150dvh/, 'short viewports must be able to reduce the displayed canvas width/height');
});

test('console runtime containers use flexible bounds instead of fixed minimum heights', () => {
  for (const selector of ['.runtime-output', '.runtime-output.tall', '.runtime-textarea.code']) {
    const block = cssBlock(consoleCss, selector);
    assert.ok(block, `${selector} rule must remain present`);
    assert.match(block, /min-height:\s*clamp\(/, `${selector} should scale with its viewport`);
    assert.doesNotMatch(block, /min-height:\s*(?:120|280|360)px/);
  }
  assert.match(consoleCss, /\.console-body\s*\{[^}]*display:\s*flex/s);
  assert.match(consoleCss, /\.console-main\s*\{[^}]*min-height:\s*0/s);
  assert.match(consoleCss, /max-height:\s*\d+dvh/);
});

test('desktop app and card widget retain usable intrinsic controls across viewport sizes', () => {
  const appBlock = cssBlock(appVue, '.app');
  assert.match(appBlock, /height:\s*100dvh/);
  assert.match(appVue, /\.app\s*>\s*\.blueprint\s*\{[^}]*flex:\s*1/s);

  const cardBlock = cssBlock(cardVue, '.card');
  assert.match(cardBlock, /min-height:\s*clamp\(/);
  assert.doesNotMatch(cardBlock, /min-height:\s*84px/);
  assert.match(cardBlock, /aspect-ratio:\s*3\s*\/\s*4/);
});

test('blueprint and areas own bounded scrolling without fixed-width overflow', () => {
  const blueprintBlock = cssBlock(rendererVue, '.blueprint');
  const areaBlock = cssBlock(rendererVue, '.area');
  assert.match(blueprintBlock, /min-width:\s*0/);
  assert.match(blueprintBlock, /overflow:\s*auto/);
  assert.match(areaBlock, /min-width:\s*0/);
  assert.match(areaBlock, /overflow:\s*auto/);
  assert.match(rendererVue, /@media\s*\(max-width:\s*900px\)/);
  assert.match(rendererVue, /overflow-x:\s*hidden/);
});
