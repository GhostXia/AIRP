import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const chatSpaceScript = await readFile(new URL('../assets/chat-space.js', import.meta.url), 'utf8');

function createElement(tagName, textContent = '') {
  return {
    tagName: tagName.toUpperCase(),
    textContent,
    value: '',
    hidden: false,
    className: '',
    children: [],
    listeners: new Map(),
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    append(...children) {
      children.forEach(child => this.appendChild(child));
    },
    prepend(child) {
      this.children.unshift(child);
    },
    replaceChildren(...children) {
      this.children = [...children];
    },
    addEventListener(type, handler) {
      this.listeners.set(type, handler);
    },
    classList: {
      add() {},
      remove() {},
    },
  };
}

function createHarness() {
  const selectors = [
    '#message-flow', '#session-list', '#message-input', '#send-message', '#engine-status',
    '#event-log', '#connection-address', '#session-operation-status', '#state-hud',
    '#hud-body', '#bgm-hud', '#bgm-body', '#new-session', '#refresh-history',
    '#continue-message', '#regen-message', '#clear-log', '#toggle-log', '#export-md',
    '#export-json', '#search-input', '#stream-status', '#character-name', '#character-avatar',
    '#character-model', '#chat-crumb', '#context-count', '#pane-right',
  ];
  const elements = Object.fromEntries(selectors.map(selector => [selector, createElement('div')]));
  const spoken = [];
  const speechSynthesis = {
    onvoiceschanged: null,
    getVoices() { return []; },
    cancel() {},
    speak(utterance) { spoken.push(utterance.text); },
  };
  const window = {
    speechSynthesis,
    setInterval() { return 1; },
    clearInterval() {},
  };
  const document = {
    querySelector(selector) {
      if (!elements[selector]) elements[selector] = createElement('div');
      return elements[selector];
    },
    querySelectorAll() { return []; },
    createElement(tagName) { return createElement(tagName); },
  };
  const context = {
    AIRPApi: {
      createClient() { return { request() {}, stream() {} }; },
      errorMessage(_data, fallback) { return fallback || '请求失败'; },
    },
    URLSearchParams,
    encodeURIComponent,
    location: { origin: 'https://example.test', search: '' },
    sessionStorage: {
      getItem() { return null; },
      setItem() {},
      removeItem() {},
    },
    document,
    window,
    speechSynthesis,
    SpeechSynthesisUtterance: function SpeechSynthesisUtterance(text) {
      this.text = text;
    },
    console,
    globalThis: null,
  };
  context.globalThis = context;

  const instrumented = chatSpaceScript.replace(
    /\bboot\(\);/,
    'globalThis.__chatSpaceTest = { speakText, suggestBgm };',
  );
  assert.notEqual(instrumented, chatSpaceScript, 'chat-space boot call must be instrumented for isolated tests');
  vm.runInNewContext(instrumented, context);
  return { elements, spoken, api: context.__chatSpaceTest };
}

test('BGM matches only direct semantic field values', () => {
  const harness = createHarness();
  const hud = harness.elements['#bgm-hud'];

  harness.api.suggestBgm({ combat: true });
  assert.equal(hud.hidden, true, 'a matching key must not trigger BGM');

  harness.api.suggestBgm({ metadata: { mood: 'combat' } });
  assert.equal(hud.hidden, true, 'an unrelated nested value must not trigger BGM');

  harness.api.suggestBgm({ mood: 'not combat' });
  assert.equal(hud.hidden, true, 'a negated keyword must not trigger BGM');

  harness.api.suggestBgm({ mood: 'combat' });
  assert.equal(hud.hidden, false);
  assert.equal(harness.elements['#bgm-body'].children[0].textContent, '紧张');
});

test('TTS removes multiline actions and markdown markers while retaining prose', () => {
  const harness = createHarness();

  harness.api.speakText('你好\n[抬头\n看向窗外]\n[轻轻\n微笑]\n~~旧内容~~\n**继续说**');

  assert.deepEqual(harness.spoken, ['你好\n旧内容\n继续说']);
});
