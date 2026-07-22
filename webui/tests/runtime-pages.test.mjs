import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readdir } from 'node:fs/promises';

const rolePage = await readFile(new URL('../screens/01-role-list.html', import.meta.url), 'utf8');
const chatPage = await readFile(new URL('../screens/02-chat-space.html', import.meta.url), 'utf8');
const onboardingPage = await readFile(new URL('../screens/16-onboarding.html', import.meta.url), 'utf8');
const entryPage = await readFile(new URL('../index.html', import.meta.url), 'utf8');
const entryScript = await readFile(new URL('../assets/entry.js', import.meta.url), 'utf8');
const onboardingScript = await readFile(new URL('../assets/onboarding.js', import.meta.url), 'utf8');

test('runtime entry redirects through an external CSP-compatible script', () => {
  assert.match(entryPage, /assets\/entry\.js/);
  assert.doesNotMatch(entryPage, /<script(?![^>]*src=)[^>]*>/i);
  assert.match(entryScript, /airp_onboarded/);
  assert.match(entryScript, /16-onboarding\.html/);
});

test('first-run onboarding uses a dedicated real-backend runtime', () => {
  assert.match(onboardingPage, /id="onboarding-steps"/);
  assert.match(onboardingPage, /id="onboarding-card"/);
  assert.match(onboardingPage, /assets\/api-client\.js/);
  assert.match(onboardingPage, /assets\/onboarding\.js/);
  assert.doesNotMatch(onboardingPage, /assets\/console-runtime\.js/);
});

test('first-run onboarding blocks blind resend after an uncertain commit', () => {
  assert.match(onboardingScript, /\['partially_committed', 'unknown'\]\.includes\(error\.commitState\)/);
  assert.match(onboardingScript, /message\.control\.disabled = true/);
  assert.match(onboardingScript, /打开对话历史确认/);
});

for (const [name, html] of [['role list', rolePage], ['chat space', chatPage]]) {
  test(name + ' has no inline style or script blocked by the Engine CSP', () => {
    assert.doesNotMatch(html, /\sstyle\s*=/i);
    assert.doesNotMatch(html, /<style(?:\s|>)/i);
    assert.doesNotMatch(html, /<script(?![^>]*src=)[^>]*>/i);
  });

  test(name + ' does not include sample-only navigation chrome', () => {
    assert.doesNotMatch(html, /assets\/app\.js/);
    assert.doesNotMatch(html, /data-sample-chrome/);
  });
}

test('role list exposes the live import and data targets', () => {
  for (const id of ['engine-status', 'character-file', 'character-grid', 'stat-characters']) {
    assert.match(rolePage, new RegExp('id="' + id + '"'));
  }
  assert.match(rolePage, /class="pane-main role-main"/);
  assert.match(rolePage, /class="stat-grid"/);
  assert.match(rolePage, /class="char-grid" id="character-grid"/);
  assert.match(rolePage, /assets\/role-list\.js/);
});

test('chat space exposes session, history and streaming controls', () => {
  for (const id of ['session-list', 'message-flow', 'message-input', 'send-message', 'continue-message', 'regen-message', 'refresh-history']) {
    assert.match(chatPage, new RegExp('id="' + id + '"'));
  }
  assert.match(chatPage, /assets\/chat-space\.js/);
});

test('every shipped screen is compatible with the Engine CSP', async () => {
  const directory = new URL('../screens/', import.meta.url);
  const files = (await readdir(directory)).filter(name => name.endsWith('.html'));
  assert.equal(files.length, 31);
  for (const file of files) {
    const html = await readFile(new URL(file, directory), 'utf8');
    assert.doesNotMatch(html, /\sstyle\s*=/i, file + ' contains an inline style');
    assert.doesNotMatch(html, /<style(?:\s|>)/i, file + ' contains an inline style block');
    assert.doesNotMatch(html, /<script(?![^>]*src=)[^>]*>/i, file + ' contains an inline script');
  }
});

test('operational console pages load the shared real-backend runtime', async () => {
  for (const file of ['03-workbench.html', '04-world-book.html', '05-presets-models.html', '06-user-persona.html', '07-agent-runs.html', '08-settings.html', '17-memory-state.html', '18-group-chat.html', '19-branch-tree.html', '20-assembly-preview.html', '21-usage-quota.html', '22-backup-restore.html', '23-diagnostics.html', '24-plugins.html', '25-notes-connections.html']) {
    const html = await readFile(new URL('../screens/' + file, import.meta.url), 'utf8');
    assert.match(html, /assets\/api-client\.js/);
    assert.match(html, /assets\/console-runtime\.js/);
    assert.match(html, /id="engine-status" role="status"/);
    assert.doesNotMatch(html, /assets\/app\.js/);
  }
});
