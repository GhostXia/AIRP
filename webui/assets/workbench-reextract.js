(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWorkbench = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  function hasUnknownCommitState(error) {
    return Boolean(
      error
      && (
        ['partially_committed', 'unknown'].includes(error.commitState)
        || error.name === 'AbortError'
        || error.name === 'TimeoutError'
        || error.name === 'TypeError'
      )
    );
  }

  async function reextractCharacterResources(options) {
    const confirmed = await options.confirm(
      '将根据当前角色卡重新生成问候语和世界书附属文件，并覆盖现有提取结果。继续吗？'
    );
    if (!confirmed) {
      options.setStatus('已取消重新提取');
      return { status: 'cancelled' };
    }

    options.button.disabled = true;
    options.setStatus('正在重新提取附属资源…');
    try {
      const result = await options.request(
        'POST',
        '/v1/characters/' + encodeURIComponent(options.characterId) + '/reextract'
      );
      options.setStatus('附属资源重新提取完成');
      return { status: 'completed', result };
    } catch (error) {
      if (hasUnknownCommitState(error)) {
        options.setStatus('重新提取的提交状态未知；请刷新角色卡并核对问候语与世界书后再决定是否重试', true);
        return { status: 'unknown', error };
      }
      options.setStatus('重新提取失败：' + options.errorMessage(error), true);
      return { status: 'failed', error };
    } finally {
      options.button.disabled = false;
    }
  }

  return { reextractCharacterResources };
});
