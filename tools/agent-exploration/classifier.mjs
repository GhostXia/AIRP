// PR diff → 任务集映射
// 命中规则：文件路径模式 AND 内容关键字同时命中才触发任务集；只看 +/- 行。
// 单独路径命中（如 chat.rs 改一行无关代码）不触发，避免高频文件引发不可控成本。

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
    // 注意：原 /\bedit\b.*message/i 的尾部 \b 不会匹配 "edit_message"（_ 是 word char，无边界），
    // 导致纯路径命中下 keyword 永远失效。改用 \bedit.*message/i 让 "edit_message"、"edit message"
    // 等都能匹配，同时保留起始 \b 避免误匹配 "credit_message" 等无关词。
    keywords: [/\bedit.*message/i, /branch/i, /switch_branch/i, /active_leaf/i, /rollback/i],
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
    // path AND keyword：两者必须同时命中。
    // 单独 path 命中（例如改 chat.rs 但内容与 swipe/edit/memory 无关）不触发，
    // 否则 engine 最高频文件每次改动都会启动 2+ 任务集的 LLM+Chrome 探索，CI 成本不可控。
    if (pathHit && keywordHit) hits.add(taskName);
  }
  return [...hits];
}
