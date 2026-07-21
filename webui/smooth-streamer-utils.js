// AIRP WebUI SmoothStreamer 纯函数工具集 — #252 §2.H.2 / #275 F-1
//
// 抽取自 app.js IIFE 内 SmoothStreamer.start() 的句子边界检测逻辑，
// 目的：
//   1. 让边界检测可被 node --test 单元测试覆盖（app.js IIFE 不可 import）
//   2. 防止 PR #270 §2.M1 + CR-3 修复的 2-char context 边界 bug 回归
//
// UMD 模式与 persona-utils.js / assembly-utils.js 一致：
//   - 浏览器端通过 <script src> 加载，挂 window.AIRPSmoothStreamerUtils
//   - Node.js 测试通过 require() 加载，走 module.exports
//
// 不变量：
//   - 纯函数，不依赖 this / DOM / localStorage / performance
//   - 输入只读，不修改 queue
//   - 返回值约定：>=1 表示建议在 boundary 处截断；-1 表示无合适边界（调用方走默认 charsToRender 路径）

(function (root, factory) {
  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) root.AIRPSmoothStreamerUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  // 句子边界检测：中文句号/叹号/问号/分号，英文句号/叹号/问号 + 空格或串尾，换行符。
  // 保留原 SENTENCE_END_RE 用于导出（验证正则形状不变），但 findSentenceBoundary
  // 不直接用 .test(twoChar)——见下文注释。
  const SENTENCE_END_RE = /[。！？；\n]|\.(\s|$)|[!?](\s|$)/;

  // 在 queue 的前 charsToRender 字符窗口内，从后向前找最近的句子边界。
  //
  // 背景（PR #270 §2.M1 + CR-3 修复 + #275 F-1 进一步修复）：
  // - SENTENCE_END_RE 含多字符模式 `\.(\s|$)` / `[!?](\s|$)`
  // - M1 修复：从 `SENTENCE_END_RE.test(candidate[i])`（单字符）改为 2-char context
  // - CR-3 修复：用 queue 而非 candidate 取上下文，避免 candidate 末尾 `.` 误匹配 $
  // - #275 F-1 进一步修复：原 2-char context 仍有 bug——当 twoChar = "X."（X 非边界，
  //   '.' 是 twoChar 第二字符）时，正则 `\.(\s|$)` 会在 '.' 上匹配 $，导致 "1.0" /
  //   "Google.com" 中的 '.' 被误判为边界。根因：twoChar 末尾的 $ 不代表 queue 末尾，
  //   只代表 twoChar 末尾。
  //
  // 修复方案：findSentenceBoundary 不用 SENTENCE_END_RE.test(twoChar)，改用显式检查：
  //   - 中文标点 / 换行：单字符即边界（ch ∈ {。！？；\n}）
  //   - 英文句号 / 问号 / 叹号 + 空格：ch ∈ {.!?} 且 next 是 \s
  //   - 不依赖 $：charsToRender < queue.length 保证 i+1 < queue.length，next 存在
  // 这保持原正则语义（去掉 $ 误匹配），且语义更清晰。
  //
  // 返回值：
  //   - >=1：建议在 boundary 处截断（boundary 是 charsToRender 窗口内的截断点）
  //   - -1：无合适边界，调用方应走默认 charsToRender 路径
  //
  // 0.25 阈值：避免在边界处截断导致渲染字符过少（至少渲染 1/4 的 charsToRender）。
  function findSentenceBoundary(queue, charsToRender) {
    if (!queue || charsToRender <= 0 || charsToRender >= queue.length) return -1;
    const candidate = queue.slice(0, charsToRender);
    let boundary = -1;
    for (let i = candidate.length - 1; i >= 0; i--) {
      const ch = queue[i];
      const next = queue[i + 1];
      // 中文标点 / 换行：单字符即边界（SENTENCE_END_RE 的 [。！？；\n] 分支）
      if (ch === '。' || ch === '！' || ch === '？' || ch === '；' || ch === '\n') {
        boundary = i + 1;
        break;
      }
      // 英文句号 / 问号 / 叹号 + 空格：ch ∈ {.!?} 且 next 是 \s
      // （SENTENCE_END_RE 的 \.(\s|$) / [!?](\s|$) 分支，去掉 $ 误匹配）
      // charsToRender < queue.length 保证 next 存在（i+1 <= charsToRender < queue.length）
      if ((ch === '.' || ch === '!' || ch === '?') && next !== undefined && /\s/.test(next)) {
        boundary = i + 1;
        break;
      }
    }
    // 找到边界且不会导致太少字符（至少渲染 1/4 的 charsToRender）。
    if (boundary > charsToRender * 0.25) {
      return boundary;
    }
    return -1;
  }

  return {
    SENTENCE_END_RE,
    findSentenceBoundary,
  };
});
