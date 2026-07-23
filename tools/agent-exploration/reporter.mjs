import { writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';

export async function writeReport(reportDir, run) {
  await mkdir(reportDir, { recursive: true });

  // JSON 报告
  const jsonPath = join(reportDir, 'report.json');
  await writeFile(jsonPath, JSON.stringify(run, null, 2));

  // Markdown 报告
  const md = renderMarkdown(run);
  const mdPath = join(reportDir, 'report.md');
  await writeFile(mdPath, md);

  return { jsonPath, mdPath };
}

function renderMarkdown(run) {
  const lines = [];
  lines.push('> ⚠️ **Non-blocking** (阶段 2 MVP): 此报告不阻塞 PR 合并。崩溃/数据损坏/安全问题需人工确认；可用性问题仅记录。');
  lines.push('');
  lines.push('# Agent Browser Exploration Report');
  lines.push('');
  lines.push('**Run ID:** ' + run.runId);
  lines.push('**Trigger:** ' + run.trigger);
  lines.push('**PR:** ' + (run.prNumber || 'N/A'));
  lines.push('**Started:** ' + run.startedAt);
  lines.push('**Duration:** ' + ((run.endedAt ? Date.parse(run.endedAt) : Date.now()) - Date.parse(run.startedAt)) + 'ms');
  lines.push('**LLM:** ' + run.llmModel);
  lines.push('');
  lines.push('## Summary');
  lines.push('- Total tasks: ' + run.tasks.length);
  lines.push('- Passed: ' + run.tasks.filter(t => t.result === 'Passed').length);
  lines.push('- Failed: ' + run.tasks.filter(t => t.result === 'Failed').length);
  lines.push('- Flaky: ' + run.tasks.filter(t => t.result === 'Flaky').length);
  lines.push('');
  for (const task of run.tasks) {
    lines.push('## Task: ' + task.name);
    lines.push('');
    lines.push('**Result:** ' + task.result);
    lines.push('');
    lines.push('### Description');
    lines.push(task.description);
    lines.push('');
    if (task.reproduction && task.reproduction.length) {
      lines.push('### Reproduction');
      for (let i = 0; i < task.reproduction.length; i++) lines.push((i + 1) + '. ' + task.reproduction[i]);
      lines.push('');
    }
    if (task.expected) { lines.push('### Expected'); lines.push(task.expected); lines.push(''); }
    if (task.actual) { lines.push('### Actual'); lines.push(task.actual); lines.push(''); }
    if (task.evidence) {
      lines.push('### Evidence');
      for (const [k, v] of Object.entries(task.evidence)) lines.push('- **' + k + ':** ' + v);
      lines.push('');
    }
    if (task.consoleErrors && task.consoleErrors.length) {
      lines.push('### Console Errors');
      for (const e of task.consoleErrors.slice(0, 10)) lines.push('- ' + JSON.stringify(e));
      lines.push('');
    }
    if (task.failedRequests && task.failedRequests.length) {
      lines.push('### Failed Requests');
      for (const r of task.failedRequests.slice(0, 10)) lines.push('- ' + JSON.stringify(r));
      lines.push('');
    }
    if (task.suspectedArea) { lines.push('### Suspected Area'); lines.push(task.suspectedArea); lines.push(''); }
    if (task.reproducibility) { lines.push('### Reproducibility'); lines.push(task.reproducibility); lines.push(''); }
  }
  return lines.join('\n');
}
