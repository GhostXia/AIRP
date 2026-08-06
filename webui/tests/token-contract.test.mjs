// WebUI 设计令牌契约测试（Task #12 token 风格一致性门禁）。
//
// 纯静态、无浏览器依赖，风格对齐 runtime-pages.test.mjs；pr-gate 的
// `node --test webui/tests/*.test.mjs` 自动覆盖本文件。
//
// 两层防线：
//   1. 核心令牌清单快照：tokens.css 是唯一事实源，任何令牌改名/改值/增删
//      都会使快照断言失败，必须显式更新快照（即走 PR 审阅）。
//   2. 硬编码色值扫描：webui/assets/*.css（tokens.css 自身除外）与
//      webui/screens/*.html 中禁止出现 #xxx / rgb() / hsl() 硬编码色值。
//      截至 2026-08-04 存量违规 112 处（集中在 9 个历史 CSS 文件），已全部
//      登记进 EXEMPTED_BASELINE 豁免清单。收敛方向：豁免只允许减少、不允许
//      增加——新增任何硬编码色值（含豁免文件中未登记过的新值）都会失败；
//      修复存量违规时同步从豁免清单删除对应条目即可。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

const COLOR_PATTERN = /#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)/gi;

function stripComments(source, kind) {
  if (kind === 'css') return source.replace(/\/\*[\s\S]*?\*\//g, '');
  return source.replace(/<!--[\s\S]*?-->/g, '');
}

function extractColors(source) {
  const matches = source.match(COLOR_PATTERN) || [];
  return matches.map(value => value.replace(/\s+/g, ' ').toLowerCase()).sort();
}

async function parseTokensCss() {
  const css = stripComments(await readFile(new URL('../assets/tokens.css', import.meta.url), 'utf8'), 'css');
  const rootBlock = css.match(/:root\s*\{([\s\S]*?)\}/);
  assert.ok(rootBlock, 'tokens.css must define a :root block');
  const tokens = {};
  for (const match of rootBlock[1].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
    tokens[match[1]] = match[2].replace(/\s+/g, ' ').trim();
  }
  return tokens;
}

// 核心令牌清单快照：由 tokens.css 于 2026-08-04 生成。更新规则：仅当设计令牌
// 确实变更时整体重新生成，并在 PR 中说明原因；禁止逐条静默修改。
const TOKEN_SNAPSHOT = {
  '--bg-base': '#FAFAF7',
  '--bg-surface': '#FFFFFF',
  '--bg-subtle': '#F5F2F0',
  '--border-default': '#E0DBD9',
  '--text-primary': '#1A1A1F',
  '--text-secondary': '#73706E',
  '--text-tertiary': '#9E998F',
  '--text-inverse': '#FFFFFF',
  '--primary': '#C4663B',
  '--primary-strong': '#A85430',
  '--primary-action': '#A85430',
  '--primary-action-hover': '#8E4528',
  '--primary-tint': '#FAEDE6',
  '--primary-muted-text': '#F3D9CC',
  '--success': '#3D9E70',
  '--success-tint': '#EBF7F0',
  '--warning': '#D98C21',
  '--warning-tint': '#FCF5E6',
  '--danger': '#CC4559',
  '--danger-tint': '#FCEDF0',
  '--rel-family': '#4A6FA5',
  '--rel-lover': '#C2597A',
  '--rel-rival': '#D98C21',
  '--avatar-violet-bg': '#F0EBFA',
  '--avatar-violet-text': '#6B5CA5',
  '--avatar-blue-bg': '#EAF1F8',
  '--avatar-blue-text': '#4A6FA5',
  '--ink': '#2A2927',
  '--radius-input': '6px',
  '--radius-card': '10px',
  '--radius-modal': '14px',
  '--radius-pill': '9999px',
  '--space-1': '4px',
  '--space-2': '8px',
  '--space-3': '12px',
  '--space-4': '16px',
  '--space-6': '24px',
  '--font-body': '"Inter", "PingFang SC", "Microsoft YaHei", -apple-system, "Segoe UI", sans-serif',
  '--font-mono': '"JetBrains Mono", ui-monospace, "Cascadia Mono", consolas, monospace',
  '--shadow-card': '0 4px 12px rgba(0, 0, 0, 0.08)',
  '--shadow-pop': '0 6px 16px -2px rgba(0, 0, 0, 0.16)',
  '--shadow-control': '0 1px 3px rgba(0, 0, 0, 0.2)',
  '--overlay-modal': 'rgba(26, 23, 20, 0.45)',
  '--canvas-w': '1440px',
  '--canvas-h': '900px',
  '--topbar-h': '54px',
  '--shadow-modal': '0 12px 40px -8px rgba(0, 0, 0, 0.18)',
};

// 存量硬编码色值豁免清单（2026-08-04 抽查登记，共 112 处 / 9 个文件）。
// 语义为「多重集上限」：文件当前违规必须是登记值的多重集子集——
//   * 新增任何未登记的硬编码色值 → 失败（拦住新违规）；
//   * 修复存量后必须同步删除对应条目（豁免只减不增，收敛到空）。
// 注意：screens/*.html 当前零违规，不享有任何豁免。
const EXEMPTED_BASELINE = {
  'card-diff.css': ['#1a7f37', '#1a7f37', '#1a7f37', '#1a7f37', '#1a7f37', '#bf3989', '#bf3989', '#bf3989', '#bf3989', '#cf222e', '#cf222e', '#cf222e', '#cf222e', '#cf222e', 'rgba(191, 57, 137, 0.1)', 'rgba(207, 34, 46, 0.1)', 'rgba(26, 127, 55, 0.1)'],
  'chat-space.css': ['#22c55e', '#8b5cf6', '#e67e22'],
  'console.css': ['#b42318', '#b42318', '#dfb56b', '#fff8ea'],
  'group-chat.css': ['#06b6d4', '#14b8a6', '#22c55e', '#6366f1', '#8b5cf6', '#ec4899', '#ef4444', '#f59e0b', '#fff', '#fff'],
  'plot-arc.css': ['#22c55e', '#fff', '#fff'],
  'plugin-tools.css': ['rgb(180, 100, 30)', 'rgb(197, 48, 48)', 'rgb(197, 48, 48)', 'rgb(34, 139, 75)', 'rgb(50, 110, 170)', 'rgb(56, 161, 105)', 'rgba(0, 0, 0, 0.45)', 'rgba(127, 127, 127, 0.06)', 'rgba(127, 127, 127, 0.12)', 'rgba(127, 127, 127, 0.12)', 'rgba(127, 127, 127, 0.15)', 'rgba(127, 127, 127, 0.18)', 'rgba(127, 127, 127, 0.18)', 'rgba(127, 127, 127, 0.18)', 'rgba(127, 127, 127, 0.2)', 'rgba(127, 127, 127, 0.2)', 'rgba(127, 127, 127, 0.2)', 'rgba(127, 127, 127, 0.25)', 'rgba(127, 127, 127, 0.3)', 'rgba(127, 127, 127, 0.3)', 'rgba(127, 127, 127, 0.5)', 'rgba(255, 255, 255, 0.03)', 'rgba(56, 161, 105, 0.12)', 'rgba(56, 161, 105, 0.2)', 'rgba(56, 161, 105, 0.25)', 'rgba(56, 161, 105, 0.4)'],
  'provider-management.css': ['#111', '#166534', '#166534', '#166534', '#1e40af', '#1e40af', '#3b82f6', '#4b5563', '#666', '#666', '#666', '#6b7280', '#888', '#888', '#92400e', '#92400e', '#991b1b', '#d8dee4', '#d8dee4', '#d8dee4', '#dbeafe', '#dbeafe', '#dcfce7', '#dcfce7', '#e5e7eb', '#e5e7eb', '#e5e7eb', '#f1f3f5', '#f6f8fa', '#f6f8fa', '#fafbfc', '#fef3c7', '#fef3c7', '#fff', '#fff', '#fff', 'rgba(0, 0, 0, 0.4)'],
  'timeline-export.css': ['#0969da', '#0969da', '#1a7f37', '#1a7f37', '#57606a', '#57606a', '#bf3989', '#bf3989', '#bf3989'],
  'worldbook-graph.css': ['#b07000', '#e0a800', '#fff8e1'],
};

test('tokens.css matches the core token snapshot exactly', async () => {
  const tokens = await parseTokensCss();
  assert.deepEqual(Object.keys(tokens).sort(), Object.keys(TOKEN_SNAPSHOT).sort(),
    'token 清单发生变化：新增/删除/改名令牌必须同步更新 TOKEN_SNAPSHOT 并在 PR 中说明');
  for (const [name, expected] of Object.entries(TOKEN_SNAPSHOT)) {
    assert.equal(tokens[name], expected, `令牌 ${name} 的值被修改：${tokens[name]} ≠ ${expected}`);
  }
});

test('screens/*.html contain no hardcoded color values', async () => {
  const directory = new URL('../screens/', import.meta.url);
  const files = (await readdir(directory)).filter(name => name.endsWith('.html'));
  assert.equal(files.length, 44);
  for (const file of files) {
    const html = stripComments(await readFile(new URL(file, directory), 'utf8'), 'html');
    const violations = extractColors(html);
    assert.deepEqual(violations, [], `${file} 出现硬编码色值：${violations.join(', ')}（屏页面不享有任何豁免）`);
  }
});

test('assets CSS hardcoded colors stay within the shrinking exemption baseline', async () => {
  const directory = new URL('../assets/', import.meta.url);
  const files = (await readdir(directory)).filter(name => name.endsWith('.css') && name !== 'tokens.css');
  let exemptedTotal = 0;
  for (const file of files) {
    const css = stripComments(await readFile(new URL(file, directory), 'utf8'), 'css');
    const violations = extractColors(css);
    const baseline = [...(EXEMPTED_BASELINE[file] || [])].sort();
    exemptedTotal += baseline.length;
    // 多重集子集判定：逐个消费豁免额度，任何未登记的新值都会剩余。
    const remaining = [...baseline];
    const fresh = [];
    for (const value of violations) {
      const index = remaining.indexOf(value);
      if (index === -1) fresh.push(value);
      else remaining.splice(index, 1);
    }
    if (baseline.length === 0) {
      assert.deepEqual(violations, [], `${file} 不在豁免清单内，禁止任何硬编码色值：${violations.join(', ')}`);
    } else {
      assert.deepEqual(fresh, [], `${file} 出现豁免清单之外的新硬编码色值：${fresh.join(', ')}（请改用 var(--*) 令牌）`);
    }
  }
  // 豁免总量只减不增：清单本身被篡改（偷偷加条目）也会被快照和此处拦下。
  assert.ok(exemptedTotal <= 112, `豁免清单总量 ${exemptedTotal} 超过基线 112：豁免只允许收敛，不允许扩张`);
});
