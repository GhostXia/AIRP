(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPAgentRun = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  function toolName(tool) {
    return tool && typeof tool.name === 'string' ? tool.name : '';
  }

  function selectedCatalog(tools, selectedNames) {
    const selected = new Set(Array.isArray(selectedNames) ? selectedNames : []);
    return (Array.isArray(tools) ? tools : [])
      .filter(tool => toolName(tool) && selected.has(tool.name));
  }

  function buildRequest(input) {
    const options = input || {};
    const selected = options.toolAuthorityEnabled
      ? selectedCatalog(options.tools, options.selectedTools)
      : [];
    const allowedTools = selected.map(tool => tool.name);
    const confirmed = new Set(Array.isArray(options.confirmedTools) ? options.confirmedTools : []);
    const destructive = new Set(
      selected.filter(tool => tool.side_effect === 'destructive').map(tool => tool.name),
    );

    return {
      character_id: options.characterId,
      session_id: options.sessionId,
      user_id: options.userId,
      user_profile: { name: options.userId, variables: {} },
      message: options.message,
      max_steps: Number(options.maxSteps) || 1,
      capabilities: allowedTools.length ? ['call:tool'] : [],
      allowed_tools: allowedTools,
      confirm_tools: [...confirmed].filter(name => destructive.has(name)),
    };
  }

  function describeEvent(item) {
    if (!item || typeof item !== 'object') return String(item || '');
    switch (item.type) {
      case 'plan':
        return '步骤 ' + item.step + ' · 计划：' +
          (typeof item.action === 'string' ? item.action : JSON.stringify(item.action));
      case 'tool_call':
        return '步骤 ' + item.step + ' · 调用工具 ' + item.tool +
          '\n参数：' + JSON.stringify(item.params || {});
      case 'tool_result':
        return '步骤 ' + item.step + ' · 工具 ' + item.tool +
          (item.dry_run ? '（仅演练，尚未执行）' : '（已执行）') +
          '\n结果：' + JSON.stringify(item.output);
      case 'delta':
        return typeof item.chunk === 'string' ? item.chunk : '';
      case 'done':
        return '\n完成 · ' + (item.stop_reason || 'unknown') +
          ' · ' + (item.steps_taken || 0) + ' 步 · 约 ' +
          (item.tokens_estimated || 0) + ' tokens';
      default:
        return JSON.stringify(item);
    }
  }

  async function run(client, input, handlers) {
    const callbacks = handlers || {};
    const request = buildRequest(input);
    return client.stream('/v1/agent/run', request, {
      signal: callbacks.signal,
      onChunk(item) {
        if (callbacks.onEvent) callbacks.onEvent(item, describeEvent(item));
      },
      onDone(item) {
        if (item && callbacks.onEvent) callbacks.onEvent(item, describeEvent(item));
        if (callbacks.onDone) callbacks.onDone(item);
      },
    });
  }

  return { buildRequest, describeEvent, run };
});
