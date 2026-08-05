// C-P1/C-P3：capability consent 闸门（对译 ui/src/registry/consent.ts，去 vue reactive）。
//
// 宿主自保：第三方（esm）widget 必须经用户显式批准才加载，且只获得其声明的
// capabilities；首方（builtin）widget 无需同意。我们不审计 widget 代码——
// 安装/批准它是用户的选择与风险（docs/SECURITY.md）。
//
// 同意绑定 widget **身份** {type, version, source} 而非仅 type：manifest 后续
// 换 source 或 bump version，旧批准不延续，必须重新同意。
//
// C-P3 权威化：engine 是 capability 授权的唯一权威。consent.js 启动时经
// `GET /v1/extensions/grants` 拉取权威 grant 状态（type → granted_capabilities），
// canMount/effectiveCapabilities 优先查 engine grant 缓存；engine 不可达
// （纯静态部署 / 网络失败）时降级到本地 localStorage 的 UX 层缓存。
// 本地 grant/revoke 仍写 localStorage（离线降级 + 旧测试兼容）；线上权威
// 操作由扩展管理 UI 调 `POST /v1/extensions/:id/grants`，consent.js 仅维护
// 内存镜像。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetConsent = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /** 持久化 grants 的 localStorage key（UX 层降级缓存）。 */
  const STORAGE_KEY = 'airp:consent-grants';

  /** 本地 UX 层 grant 身份集合（type@version#source）。 */
  const granted = new Set();

  /**
   * C-P3 engine 权威 grant 缓存：type → granted_capabilities（数组）。
   * 由 initGrantsFromEngine 注入；canMount/effectiveCapabilities 优先查此。
   * 空表示未初始化（降级到本地 granted Set）。
   */
  const engineGrants = new Map();

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

  /**
   * C-P3：从 engine `GET /v1/extensions/grants` 响应注入权威 grant 缓存。
   * 优先级：engine grant > 本地 localStorage UX 缓存。
   * 调用此方法后，canMount/effectiveCapabilities 切换为查 engine 缓存；
   * engine 缓存中无该 type 时降级到本地 granted Set（未安装扩展的 esm
   * widget 仍可经本地同意挂载，用于纯静态部署场景）。
   * @param {Array} grants engine 响应 `{grants: [{id, type, granted_capabilities, granted_at}, ...]}`
   *   或直接数组形态。非数组 / 空 → no-op（保持降级模式）。
   */
  function initGrantsFromEngine(grants) {
    engineGrants.clear();
    const list = Array.isArray(grants) ? grants : (grants && Array.isArray(grants.grants) ? grants.grants : null);
    if (!list) return;
    for (const g of list) {
      if (!g || typeof g.type !== 'string') continue;
      const caps = Array.isArray(g.granted_capabilities) ? g.granted_capabilities.filter(c => typeof c === 'string') : [];
      engineGrants.set(g.type, caps);
    }
  }

  /** engine 权威缓存是否已初始化（即 boot.js 是否成功拉取过 engine grants）。 */
  function hasEngineGrants() {
    return engineGrants.size > 0;
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
    engineGrants.clear();
    // 刻意不落盘：clear 是会话内重置（测试/重新引导），不能把空集合写回存储
    // 覆盖用户已有的批准；撤销单个 widget 用 revoke（它会落盘）。
  }

  /** 第三方（esm）widget 需显式同意；builtin 不需要。 */
  function needsConsent(manifest) {
    return Boolean(manifest.entry && manifest.entry.kind === 'esm');
  }

  /**
   * 该 widget 现在能否挂载？
   * - builtin：总是；
   * - esm + engine grant 缓存已初始化：type 在 engineGrants 中且
   *   granted_capabilities 非空（即 engine 已权威签发 grant）；
   * - esm + engine 缓存未初始化（降级模式）：仅当本地 granted Set 含该身份。
   */
  function canMount(manifest) {
    if (!needsConsent(manifest)) return true;
    if (engineGrants.size > 0) {
      const caps = engineGrants.get(manifest.type);
      return Array.isArray(caps) && caps.length > 0;
    }
    return isGranted(manifest);
  }

  /**
   * widget 实际可用的 capabilities（不可挂载则为空）。
   * - engine 模式：engine grant 的 granted_capabilities（已是 manifest 子集，
   *   engine 侧校验过；与 manifest.capabilities 取交集为防御性冗余）；
   * - 降级模式：manifest.capabilities 全集（本地 consent 仅做闸门，不缩窄）。
   */
  function effectiveCapabilities(manifest) {
    if (!canMount(manifest)) return [];
    if (engineGrants.size > 0) {
      const caps = engineGrants.get(manifest.type) || [];
      const declared = manifest.capabilities || [];
      // 防御性交集：engine grant 应已是 manifest 子集，但取交集避免脏数据。
      return caps.filter(c => declared.includes(c));
    }
    return manifest.capabilities || [];
  }

  return {
    STORAGE_KEY,
    initGrants,
    initGrantsFromEngine,
    hasEngineGrants,
    isGranted,
    grant,
    revoke,
    clearGrants,
    needsConsent,
    canMount,
    effectiveCapabilities,
  };
});
