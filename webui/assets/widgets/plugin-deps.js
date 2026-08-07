// #498 §7.1/§7.4：trusted plugin 软依赖状态（对译 ui/src/registry 去 Vue 重构版）。
//
// 职责：持有 `GET /v1/plugins` 的权威缓存（id → {version, host_api, status}），
// 提供 `missingDependencies(manifest)` 计算 widget 声明但不可用的 trusted
// plugin 软依赖。加载流程见 boot.js `fetchEnginePlugins()`；消费点见
// widget-host.js render（非阻塞降级提示条——widget 仍加载，提示由用户/
// widget 自行处理，engine 不强制匹配，见 docs/TRUSTED-PLUGINS.md §4.2/§6）。
//
// `status` 仅反映 spawn 结果（`running` / `stopped`，engine 不探活）：
// 未装、已崩、spawn 失败（端口冲突）的插件一律视为「不可用」。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetPluginDeps = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /** id -> {version, host_api, status}（来自 GET /v1/plugins 的 plugins 数组）。 */
  let installed = new Map();

  /** 全量替换权威缓存（boot 时调用；失败则保持空 → 所有依赖都提示缺失）。 */
  function initFromEngine(list) {
    installed = new Map();
    for (const p of (list && list.plugins) || []) {
      if (!p || typeof p.id !== 'string' || !p.id) continue;
      installed.set(p.id, {
        version: typeof p.version === 'string' ? p.version : '',
        host_api: typeof p.host_api === 'string' ? p.host_api : '',
        status: typeof p.status === 'string' ? p.status : 'stopped',
      });
    }
  }

  /**
   * 计算 manifest.trusted_plugins 中不可用的依赖条目。
   * 返回 [{id, min_host_api, reason}]；`reason` 区分 `not-installed` 与
   * `stopped`（spawn 失败/崩溃），供提示文案区分。无声明 → []。
   */
  function missingDependencies(manifest) {
    const deps = (manifest && manifest.trusted_plugins) || [];
    const missing = [];
    for (const dep of deps) {
      if (!dep || typeof dep.id !== 'string' || !dep.id) continue;
      const info = installed.get(dep.id);
      if (!info) {
        missing.push({ id: dep.id, min_host_api: dep.min_host_api || null, reason: 'not-installed' });
      } else if (info.status !== 'running') {
        missing.push({ id: dep.id, min_host_api: dep.min_host_api || null, reason: 'stopped' });
      }
    }
    return missing;
  }

  /** 当前缓存的全部已安装插件（测试/调试用）。 */
  function allInstalled() {
    return [...installed.values()];
  }

  return { initFromEngine, missingDependencies, allInstalled };
});
