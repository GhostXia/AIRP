import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const asset = name => readFile(new URL('../assets/' + name, import.meta.url), 'utf8');
const helper = await asset('confirm-dialog.js');
const callers = await Promise.all([
  asset('chat-space.js'),
  asset('console-runtime.js'),
  asset('plugin-tools.js'),
  asset('provider-management.js'),
  asset('role-list.js'),
  asset('workbench-reextract.js'),
]);
const screens = await Promise.all([
  '01-role-list.html', '02-chat-space.html', '03-workbench.html',
  '04-world-book.html', '05-presets.html', '06-user-persona.html',
  '07-agent-runs.html', '08-settings.html', '17-memory-state.html',
  '19-branch-tree.html', '20-assembly-preview.html', '21-usage-quota.html',
  '22-backup-restore.html', '23-diagnostics.html', '24-plugins.html',
  '25-notes-connections.html', '32-style-review.html',
  '43-provider-management.html', '44-plugin-tools.html',
].map(name => readFile(new URL('../screens/' + name, import.meta.url), 'utf8')));

test('destructive WebUI paths use the shared confirmation UI', () => {
  for (const script of callers) {
    assert.doesNotMatch(script, /window\.confirm\s*\(/, 'native confirm must not remain in a caller');
    assert.match(script, /AIRPConfirm\.confirm|options\.confirm/, 'caller must use the shared confirmation API');
  }
  for (const html of screens) assert.match(html, /assets\/confirm-dialog\.js/);
  assert.match(helper, /root\.AIRPConfirm\s*=\s*\{\s*confirm\s*\}/);
});

test('shared confirmation resolves cancel/approve and exposes alertdialog semantics', async () => {
  class Element {
    constructor(tagName) {
      this.tagName = tagName.toUpperCase();
      this.children = [];
      this.listeners = new Map();
      this.classList = { add() {}, remove() {} };
      this.open = false;
      this.hidden = false;
      this.textContent = '';
      this.id = '';
    }
    append(...children) { this.children.push(...children); }
    appendChild(child) { this.children.push(child); return child; }
    addEventListener(type, listener) {
      const list = this.listeners.get(type) || [];
      list.push(listener);
      this.listeners.set(type, list);
    }
    dispatch(type, event = {}) {
      for (const listener of this.listeners.get(type) || []) listener(event);
    }
    setAttribute(name, value) { this[name] = String(value); }
    focus() { this.ownerDocument.activeElement = this; }
    showModal() { this.open = true; }
    close() { this.open = false; }
  }
  const document = {
    activeElement: null,
    createElement(tagName) {
      const element = new Element(tagName);
      element.ownerDocument = document;
      return element;
    },
    body: null,
  };
  document.body = document.createElement('body');
  const context = { document, console, globalThis: null };
  context.globalThis = context;
  vm.runInNewContext(helper, context);

  const cancelled = context.AIRPConfirm.confirm('cancel me');
  await new Promise(resolve => setImmediate(resolve));
  const modal = document.body.children[0];
  assert.equal(modal.role, 'alertdialog');
  assert.equal(modal['aria-modal'], 'true');
  assert.equal(modal.open, true);
  modal.dispatch('cancel', { preventDefault() {} });
  assert.equal(await cancelled, false);

  const approved = context.AIRPConfirm.confirm('approve me');
  await new Promise(resolve => setImmediate(resolve));
  const actions = modal.children[2];
  actions.children[1].dispatch('click');
  assert.equal(await approved, true);
  assert.equal(modal.open, false);
});
