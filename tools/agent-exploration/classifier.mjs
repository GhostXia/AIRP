// PR diff → 任务集映射
// 命中规则按"文件路径模式 + 内容关键字"组合；只看 +/- 行。

export const DIFF_TASK_MAP = {
  'onboarding-firstchat-refresh': {
    paths: [/webui\/assets\/onboarding\.js/, /webui\/screens\/16-onboarding\.html/, /engine\/src\/daemon\/handlers\/onboarding/],
    keywords: [/onboarding/i, /first.?chat/i, /first_mes/i],
  },
  'regen-swipe-refresh': {
    paths: [/engine\/src\/daemon\/handlers\/chat\.rs/, /webui\/assets\/chat-space\.js/, /webui\/screens\/14-message-swipe\.html/],
    keywords: [/regen/i, /swipe/i, /smooth.?stream/i, /candidate/i],
  },
  'edit-branch-switch-refresh': {
    paths: [/engine\/src\/chat_store\.rs/, /engine\/src\/daemon\/handlers\/chat\.rs/, /webui\/screens\/19-branch-tree\.html/],
    keywords: [/\bedit\b.*message/i, /branch/i, /switch_branch/i, /active_leaf/i, /rollback/i],
  },
  'memory-roundtrip': {
    paths: [/engine\/src\/daemon\/handlers\/memory\.rs/, /engine\/src\/memory/, /webui\/screens\/17-memory-state\.html/],
    keywords: [/resident.?memory/i, /user.?model/i, /memory.?extract/i],
  },
};

export function classifyPrDiff(diff) {
  if (!diff || typeof diff !== 'string') return [];
  // 提取 diff 中变更的文件路径
  const pathMatch = diff.match(/^diff --git a\/(\S+) b\/\S+$/gm) || [];
  const paths = pathMatch.map(l => l.replace(/^diff --git a\//, '').replace(/ b\/\S+$/, ''));

  // 提取 +/- 行内容
  const changedLines = diff.split('\n').filter(l => l.startsWith('+') || l.startsWith('-')).join('\n');

  const hits = new Set();
  for (const [taskName, rule] of Object.entries(DIFF_TASK_MAP)) {
    const pathHit = rule.paths.some(p => paths.some(pp => p.test(pp)));
    const keywordHit = rule.keywords.some(k => k.test(changedLines));
    if (pathHit && keywordHit) hits.add(taskName);
    else if (pathHit) hits.add(taskName); // 路径命中即触发，关键字仅作加权（此处简化为 OR）
  }
  return [...hits];
}
