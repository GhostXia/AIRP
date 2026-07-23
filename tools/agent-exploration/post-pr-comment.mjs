// 读 report.md, 截断到 GitHub 评论长度上限 (65536), 通过 gh CLI 发评论
// 用 --body-file 模式（项目 memory: PowerShell/gh 多行特殊字符问题）

import { readFile } from 'node:fs/promises';
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
