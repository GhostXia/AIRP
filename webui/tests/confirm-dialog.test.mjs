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
      const classes = new Set();
      this.classList = {
        add: (...names) => names.forEach(name => classes.add(name)),
        remove: (...names) => names.forEach(name => classes.delete(name)),
        contains: name => classes.has(name),
      };
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
    setAttribute(name, value) {
      if (name === 'open') this.open = true;
      else this[name] = String(value);
    }
    removeAttribute(name) { delete this[name]; }
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
    addEventListener() {},
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
  const modalActions = modal.children[0].children[2];
  assert.equal(document.activeElement, modalActions.children[0], 'danger actions start on Cancel');
  modal.dispatch('cancel', { preventDefault() {} });
  assert.equal(await cancelled, false);

  const approved = context.AIRPConfirm.confirm('approve me', { danger: false });
  await new Promise(resolve => setImmediate(resolve));
  const actions = modal.children[0].children[2];
  assert.equal(document.activeElement, actions.children[1], 'non-danger actions start on Confirm');
  actions.children[1].dispatch('click');
  assert.equal(await approved, true);
  assert.equal(modal.open, false);
});

test('fallback confirmation traps focus, cancels on Escape/outside click, and pumps queued requests', async () => {
  class Element {
    constructor(tagName) {
      this.tagName = tagName.toUpperCase();
      this.children = [];
      this.listeners = new Map();
      const classes = new Set();
      this.classList = {
        add: (...names) => names.forEach(name => classes.add(name)),
        remove: (...names) => names.forEach(name => classes.delete(name)),
        contains: name => classes.has(name),
      };
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
      for (const listener of this.listeners.get(type) || []) listener({ target: this, ...event });
    }
    setAttribute(name, value) {
      if (name === 'open') this.open = true;
      else this[name] = String(value);
    }
    removeAttribute(name) { delete this[name]; }
    focus() { this.ownerDocument.activeElement = this; }
    close() { this.open = false; }
  }
  const document = {
    activeElement: null,
    listeners: new Map(),
    createElement(tagName) {
      const element = new Element(tagName);
      element.ownerDocument = document;
      return element;
    },
    addEventListener(type, listener) {
      const list = this.listeners.get(type) || [];
      list.push(listener);
      this.listeners.set(type, list);
    },
    dispatch(type, event = {}) {
      for (const listener of this.listeners.get(type) || []) listener(event);
    },
    body: null,
  };
  document.body = document.createElement('body');
  const context = { document, console, globalThis: null };
  context.globalThis = context;
  vm.runInNewContext(helper, context);

  const trigger = document.createElement('button');
  document.body.appendChild(trigger);
  trigger.focus();
  const first = context.AIRPConfirm.confirm('first');
  const second = context.AIRPConfirm.confirm('second');
  await new Promise(resolve => setImmediate(resolve));
  const modal = document.body.children[1];
  assert.equal(modal.classList.contains('is-fallback'), true);
  assert.equal(document.activeElement, modal.children[0].children[2].children[0], 'fallback danger action starts on Cancel');
  document.dispatch('keydown', { key: 'Escape', preventDefault() {} });
  assert.equal(await first, false);
  assert.equal(modal.open, true, 'queued confirmation opens after cancellation');
  modal.dispatch('click', { target: modal });
  assert.equal(await second, false, 'outside click cancels the queued fallback');
  assert.equal(modal.open, false);
  assert.equal(document.activeElement, trigger, 'fallback cancellation restores the triggering focus');
});
