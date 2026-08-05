// C-P1：capability consent 闸门（对译 ui/src/registry/consent.ts，去 vue reactive）。
//
// 宿主自保：第三方（esm）widget 必须经用户显式批准才加载，且只获得其声明的
// capabilities；首方（builtin）widget 无需同意。我们不审计 widget 代码——
// 安装/批准它是用户的选择与风险（docs/SECURITY.md）。
//
// 同意绑定 widget **身份** {type, version, source} 而非仅 type：manifest 后续
// 换 source 或 bump version，旧批准不延续，必须重新同意。
//
// 持久化：grants 存 localStorage（key `airp:consent-grants`）跨刷新保留；
// 启动时调用一次 initGrants() 恢复。存储可注入（测试）。
// 去 reactive：vue 版用 reactive Set 触发重渲染；vanilla 版由 widget-host
// 在授权动作后自行重挂载。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetConsent = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /** 持久化 grants 的 localStorage key。 */
  const STORAGE_KEY = 'airp:consent-grants';

  const granted = new Set();

  /** 存储后端。默认 localStorage；可经 initGrants() 覆写。 */
  let storage = null;

  /** grant 绑定的身份：type + version + (esm) source。 */
  function grantKey(manifest) {
    const source = manifest.entry && manifest.entry.kind === 'esm' ? (manifest.entry.source || '') : '';
    return manifest.type + '@' + manifest.version + '#' + source;
  }

  /** 把当前 grant 集合写入存储（若已配置）。 */
  function save() {
    if (!storage) return;
    try {
      storage.setItem(STORAGE_KEY, JSON.stringify([...granted]));
    } catch {
      // localStorage 可能已满或不可用；优雅降级。
    }
  }

  /**
   * 初始化 consent 持久化：载入既有 grants 并让后续 grant/revoke/clear 自动落盘。
   * 应用启动时调用一次；从不调用则 consent 仅存内存（向后兼容）。
   * @param {object} [s] 存储后端；缺省用 localStorage；测试传 mock。
   *   缺省且 localStorage 不可用（非 DOM）时为 no-op，绝不抛错。
   */
  function initGrants(s) {
    const backend = s != null ? s : (typeof localStorage !== 'undefined' ? localStorage : null);
    if (!backend) return;
    storage = backend;
    try {
      const raw = storage.getItem(STORAGE_KEY);
      if (raw) {
        const keys = JSON.parse(raw);
        if (Array.isArray(keys)) {
          for (const k of keys) if (typeof k === 'string') granted.add(k);
        }
      }
    } catch {
      // 损坏或不可用；从零开始。
    }
  }

  function isGranted(manifest) {
    return granted.has(grantKey(manifest));
  }
  function grant(manifest) {
    granted.add(grantKey(manifest));
    save();
  }
  function revoke(manifest) {
    granted.delete(grantKey(manifest));
    save();
  }
  function clearGrants() {
    granted.clear();
    // 刻意不落盘：clear 是会话内重置（测试/重新引导），不能把空集合写回存储
    // 覆盖用户已有的批准；撤销单个 widget 用 revoke（它会落盘）。
  }

  /** 第三方（esm）widget 需显式同意；builtin 不需要。 */
  function needsConsent(manifest) {
    return Boolean(manifest.entry && manifest.entry.kind === 'esm');
  }

  /** 该 widget 现在能否挂载？builtin：总是；esm：仅当该精确身份已获批准。 */
  function canMount(manifest) {
    if (!needsConsent(manifest)) return true;
    return isGranted(manifest);
  }

  /** widget 实际可用的 capabilities（不可挂载则为空）。 */
  function effectiveCapabilities(manifest) {
    if (!canMount(manifest)) return [];
    return manifest.capabilities || [];
  }

  return {
    STORAGE_KEY,
    initGrants,
    isGranted,
    grant,
    revoke,
    clearGrants,
    needsConsent,
    canMount,
    effectiveCapabilities,
  };
});
