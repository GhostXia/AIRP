// webui/history-utils.js
//
// #148: 把 history toolbar 的状态计算提取为纯函数，便于单测网络失败后按钮恢复。
// app.js 的 updateHistoryToolbar() 调用 computeHistoryToolbarState() 并应用到 DOM。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.AIRPHistoryUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /**
   * 计算 history toolbar 的 UI 状态。
   *
   * @param {object} historyState - { total, hasMore, loading }
   * @param {number} loadedCount - 已渲染的消息节点数
   * @param {Intl.NumberFormat} countFormatter - 数字格式化器
   * @returns {object} { toolbarHidden, statusText, loadEarlierHidden, loadEarlierDisabled, loadEarlierText }
   */
  function computeHistoryToolbarState(historyState, loadedCount, countFormatter) {
    const loading = Boolean(historyState && historyState.loading);
    const total = Number(historyState && historyState.total) || 0;
    const hasMore = Boolean(historyState && historyState.hasMore);
    // 防御性 fallback：若 countFormatter 为 null/undefined 或无 format 方法，
    // 退化为 String()，避免纯函数抛 TypeError（gemini-code-assist 建议）。
    const format = function (num) {
      return countFormatter && typeof countFormatter.format === 'function'
        ? countFormatter.format(num)
        : String(num);
    };
    return {
      toolbarHidden: total === 0,
      statusText:
        format(loadedCount || 0) +
        ' / ' +
        format(total) +
        ' 条消息',
      loadEarlierHidden: !hasMore,
      // #148: loading 结束后（含网络失败 r.status===0）按钮必须恢复可用
      loadEarlierDisabled: loading,
      loadEarlierText: loading ? '加载中…' : '加载更早',
    };
  }

  return { computeHistoryToolbarState };
});
