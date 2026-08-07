// #498 §7.1/§7.4：trusted plugin 软依赖状态（对译 ui/src/registry 去 Vue 重构版）。
//
// 职责：持有 `GET /v1/plugins` 的权威缓存（id → {version, host_api, status}），
// 提供 `missingDependencies(manifest)` 计算 widget 声明但不可用的 trusted
// plugin 软依赖。加载流程见 boot.js `fetchEnginePlugins()`；消费点见
// widget-host.js render（非阻塞降级提示条——widget 仍加载，提示由用户/
// widget 自行处理，engine 不强制匹配，见 docs/TRUSTED-PLUGINS.md §4.2/§6）。
//
// `status` 仅反映 spawn 结果（`running` / `stopped`，engine 不探活）：
// 未装、已崩、spawn 失败（端口冲突）的插件一律视为「不可用」；
// 已运行但 `host_api` 低于声明的 `min_host_api` 视为「版本过低」
// （审计 #507：running 不等于可用，宿主合同版本同样参与判定）。
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
   * installed 版本是否 >= min（逐段数值比较，缺段视为 0）。
   * min 缺省 / 空 → 不比对（true）；任一段非纯数字 → false（fail-closed：
   * 脏数据一律按不满足提示，不静默放行）。
   *
   * 审计 #507 N2 修复：用 `^\d+$` 严格校验每段，拒绝 `Number()` 会误接受
   * 的 hex（`'0x10'` → 16）和科学计数法（`'1e2'` → 100）。semver 段只
   * 允许纯十进制数字。
   */
  function versionAtLeast(installedVersion, min) {
    if (min == null || min === '') return true;
    const a = String(installedVersion).split('.');
    const b = String(min).split('.');
    const len = Math.max(a.length, b.length);
    for (let i = 0; i < len; i++) {
      const segA = a[i] || '0';
      const segB = b[i] || '0';
      if (!/^\d+$/.test(segA) || !/^\d+$/.test(segB)) return false;
      const x = parseInt(segA, 10);
      const y = parseInt(segB, 10);
      if (x < y) return false;
      if (x > y) return true;
    }
    return true;
  }

  /**
   * 计算 manifest.trusted_plugins 中不可用的依赖条目。
   * 返回 [{id, min_host_api, reason}]；`reason` 区分 `not-installed` /
   * `stopped`（spawn 失败/崩溃）/ `version-too-low`（host_api 低于声明），
   * 供提示文案区分。无声明 → []。
   */
  function missingDependencies(manifest) {
    const deps = (manifest && manifest.trusted_plugins) || [];
    const missing = [];
    for (const dep of deps) {
      if (!dep || typeof dep.id !== 'string' || !dep.id) continue;
      const minHostApi = dep.min_host_api || null;
      const info = installed.get(dep.id);
      if (!info) {
        missing.push({ id: dep.id, min_host_api: minHostApi, reason: 'not-installed' });
      } else if (info.status !== 'running') {
        missing.push({ id: dep.id, min_host_api: minHostApi, reason: 'stopped' });
      } else if (!versionAtLeast(info.host_api, minHostApi)) {
        missing.push({ id: dep.id, min_host_api: minHostApi, reason: 'version-too-low' });
      }
    }
    return missing;
  }

  /** 当前缓存的全部已安装插件（测试/调试用）。 */
  function allInstalled() {
    return [...installed.values()];
  }

  return { initFromEngine, missingDependencies, allInstalled, versionAtLeast };
});
