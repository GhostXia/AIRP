// 读 report.md, 截断到 GitHub 评论长度上限 (65536), 通过 gh CLI 发评论
// 用 --body-file 模式（项目 memory: PowerShell/gh 多行特殊字符问题）

import { readFile, access } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawnSync } from 'node:child_process';

const reportPath = process.argv[2];
if (!reportPath) {
  console.error('usage: post-pr-comment.mjs <report.md>');
  process.exit(2);
}

const prNumber = process.env.PR_NUMBER;
const token = process.env.GH_TOKEN;
if (!prNumber || !token) {
  console.error('PR_NUMBER and GH_TOKEN env vars required');
  process.exit(2);
}

// B5 修复: report.md 可能不存在（topology bootstrap 失败 → runner 没跑 → report 未生成）。
// 这种情况下 failure() placeholder step 已经发了占位评论（runner exit 1 触发），
// 本 step 不应再因 ENOENT 抛错失败；改为优雅退出 + 提示已由 placeholder 处理。
try {
  await access(reportPath, constants.R_OK);
} catch {
  console.warn('[post-pr-comment] report not found at ' + reportPath + '; runner likely did not run (topology bootstrap failed?); placeholder comment already posted by if: failure() step');
  process.exit(0);
}

let body = await readFile(reportPath, 'utf8');
const MAX = 65000;  // 留余量
if (body.length > MAX) {
  body = body.slice(0, MAX) + '\n\n... (report truncated; see artifacts for full report)';
}

// 用 gh pr comment --body-file 避免 PowerShell 特殊字符问题
const result = spawnSync('gh', ['pr', 'comment', prNumber, '--body-file', '-'], {
  input: body,
  env: process.env,
  encoding: 'utf8',
});

if (result.status !== 0) {
  console.error('gh pr comment failed:', result.stderr);
  process.exit(result.status || 1);
}
console.log('PR comment posted:', result.stdout.trim());
