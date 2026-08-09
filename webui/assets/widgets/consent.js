// C-P3/#474：capability consent 闸门（对译 ui/src/registry/consent.ts，去 vue reactive）。
//
// 宿主自保：第三方（esm）widget 必须经用户显式批准才加载，且只获得其声明的
// capabilities；首方（builtin）widget 无需同意。我们不审计 widget 代码——
// 安装/批准它是用户的选择与风险（docs/SECURITY.md）。
//
// 同意绑定 widget **身份** {id, type, version, source, digest}，且必须来自 engine 的已启用
// grant 记录。engine 把 grant 持久化在 extensions.json，并在每次 intent 调用时
// 再校验；本模块只保留可丢弃的内存镜像。
//
// 安全不变式（issue #474）：
// - 成功取得 engine 快照（包括空数组）后进入 engine-authoritative 模式；
// - engine 请求失败、响应畸形或身份字段缺失时进入 unavailable/fail-closed，
//   localStorage 不得让第三方 widget 挂载；
// - grant/revoke 只在 engine mutation 成功后更新镜像。
//
// `initGrants()` / localStorage 仅保留给旧的离线单测与迁移构型；boot.js 不调用它，
// 生产路径始终先配置 engine authority，再调用 initGrantsFromEngine/markUnavailable。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetConsent = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /** 旧测试/迁移缓存 key；不是授权真值，engine 模式完全不读取它。 */
  const STORAGE_KEY = 'airp:consent-grants';

  /** 未接入 engine 的旧测试构型才使用的内存/存储镜像。 */
  const granted = new Set();

  /** type → engine grant views（同 type 的不同身份不得互相覆盖）。 */
  const engineGrants = new Map();
  let engineState = 'uninitialized'; // uninitialized | ready | unavailable
  let engineClient = null;
  let storage = null;

  /** grant 绑定的身份：type + version + (esm) source。 */
  function grantKey(manifest) {
    const source = manifest.entry && manifest.entry.kind === 'esm' ? (manifest.entry.source || '') : '';
    return manifest.type + '@' + manifest.version + '#' + source;
  }

  function sourceOf(manifest) {
    return manifest.entry && manifest.entry.kind === 'esm' ? (manifest.entry.source || '') : '';
  }

  function digestOf(manifest) {
    const match = /^\/extensions\/([0-9a-f]{64})\/index\.js$/.exec(sourceOf(manifest));
    return match ? match[1] : null;
  }

  /** 把旧的本地 grant 集合写入存储（engine 模式不会调用）。 */
  function save() {
    if (!storage || engineState !== 'uninitialized') return;
    try {
      storage.setItem(STORAGE_KEY, JSON.stringify([...granted]));
    } catch {
      // localStorage 可能已满或不可用；旧测试构型仍应不抛错。
    }
  }

  /**
   * 仅供旧测试/迁移构型初始化本地镜像。生产 boot 不调用；一旦 engine
   * 快照成功或失败，本地镜像都不再参与 canMount。
   */
  function initGrants(s) {
    if (engineState !== 'uninitialized') return;
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

  function normaliseGrant(grant) {
    if (!grant || typeof grant !== 'object') return null;
    if (typeof grant.id !== 'string' || grant.id.length === 0) return null;
    if (typeof grant.type !== 'string' || grant.type.length === 0) return null;
    if (typeof grant.version !== 'string' || grant.version.length === 0) return null;
    if (typeof grant.digest !== 'string' || !/^[0-9a-f]{64}$/.test(grant.digest)) return null;
    if (typeof grant.enabled !== 'boolean') return null;
    if (!(grant.source === null || typeof grant.source === 'string')) return null;
    if (!Array.isArray(grant.granted_capabilities)
      || grant.granted_capabilities.some(cap => typeof cap !== 'string')) return null;
    return {
      id: grant.id,
      type: grant.type,
      version: grant.version,
      source: grant.source || '',
      digest: grant.digest,
      enabled: grant.enabled,
      granted_capabilities: [...grant.granted_capabilities],
      granted_at: grant.granted_at == null ? null : grant.granted_at,
    };
  }

  /**
   * 从 engine `/v1/grants`（或兼容 `/v1/extensions/grants`）响应建立权威镜像。
   * 空数组是有效快照，不能被解释为「未初始化」而回退到 localStorage。
   * @returns {boolean} 是否接受了完整、可校验的 engine 快照。
   */
  function initGrantsFromEngine(payload) {
    const list = Array.isArray(payload)
      ? payload
      : (payload && Array.isArray(payload.grants) ? payload.grants : null);
    if (!list) {
      markEngineUnavailable();
      return false;
    }
    const parsed = [];
    for (const item of list) {
      const record = normaliseGrant(item);
      if (!record) {
        markEngineUnavailable();
        return false;
      }
      parsed.push(record);
    }
    const ids = new Set();
    engineGrants.clear();
    for (const record of parsed) {
      if (ids.has(record.id)) {
        markEngineUnavailable();
        return false;
      }
      ids.add(record.id);
      const records = engineGrants.get(record.type) || [];
      records.push(record);
      engineGrants.set(record.type, records);
    }
    granted.clear();
    engineState = 'ready';
    return true;
  }

  /** engine 请求失败/响应畸形后的 fail-closed 状态。 */
  function markEngineUnavailable() {
    engineGrants.clear();
    granted.clear();
    engineState = 'unavailable';
  }

  /** 为批准/撤销 mutation 注入 engine client；boot.js 与 standalone UI 各自提供。 */
  function configureEngineAuthority(client) {
    engineClient = client && typeof client.updateGrant === 'function' ? client : null;
  }

  function hasEngineGrants() {
    return engineState === 'ready';
  }

  function engineGrantFor(manifest) {
    if (engineState !== 'ready') return null;
    const records = engineGrants.get(manifest.type) || [];
    const digest = digestOf(manifest);
    if (!digest) return null;
    return records.find(record => record.enabled
      && record.version === manifest.version
      && record.source === sourceOf(manifest)
      && record.digest === digest) || null;
  }

  /**
   * 第三方（esm）widget 需显式同意；builtin 不需要。
   */
  function needsConsent(manifest) {
    return Boolean(manifest.entry && manifest.entry.kind === 'esm');
  }

  /** engine-authoritative 模式只认 exact identity + 非空 grant；旧测试构型认内存镜像。 */
  function isGranted(manifest) {
    if (!needsConsent(manifest)) return false;
    if (engineState === 'ready') {
      const record = engineGrantFor(manifest);
      return Boolean(record && record.granted_capabilities.length > 0);
    }
    if (engineState === 'unavailable') return false;
    return granted.has(grantKey(manifest));
  }

  /**
   * 请求 engine 签发 grant。成功响应才更新内存镜像；engine 未初始化时只
   * 为旧测试构型更新本地镜像，生产 boot 在请求失败前会先 mark unavailable。
   */
  function grant(manifest) {
    if (!needsConsent(manifest)) return Promise.resolve(null);
    if (engineState === 'ready') {
      const current = engineGrantFor(manifest);
      if (!current || !engineClient) {
        return Promise.reject(new Error('engine grant authority unavailable for widget identity'));
      }
      const capabilities = Array.isArray(manifest.capabilities) ? manifest.capabilities : [];
      return Promise.resolve(engineClient.updateGrant(current.id, 'grant', capabilities))
        .then((updated) => {
          const record = normaliseGrant(updated);
          if (!record || record.id !== current.id || record.type !== manifest.type
            || record.version !== manifest.version
            || record.source !== sourceOf(manifest)
            || record.digest !== digestOf(manifest)) {
            throw new Error('engine returned an invalid grant record');
          }
          const records = engineGrants.get(record.type) || [];
          const index = records.findIndex(item => item.id === record.id);
          if (index >= 0) records[index] = record;
          else records.push(record);
          engineGrants.set(record.type, records);
          return record;
        });
    }
    if (engineState === 'unavailable') {
      return Promise.reject(new Error('engine grant authority unavailable'));
    }
    granted.add(grantKey(manifest));
    save();
    return Promise.resolve(null);
  }

  /** 请求 engine 撤销 grant；同样禁止乐观更新。 */
  function revoke(manifest) {
    if (engineState === 'ready') {
      const current = engineGrantFor(manifest);
      if (!current || !engineClient) {
        return Promise.reject(new Error('engine grant authority unavailable for widget identity'));
      }
      return Promise.resolve(engineClient.updateGrant(current.id, 'revoke'))
        .then((updated) => {
          const record = normaliseGrant(updated);
          if (!record || record.id !== current.id || record.type !== manifest.type
            || record.version !== manifest.version
            || record.source !== sourceOf(manifest)
            || record.digest !== digestOf(manifest)) {
            throw new Error('engine returned an invalid grant record');
          }
          const records = engineGrants.get(record.type) || [];
          const index = records.findIndex(item => item.id === record.id);
          if (index >= 0) records[index] = record;
          else records.push(record);
          engineGrants.set(record.type, records);
          return record;
        });
    }
    if (engineState === 'unavailable') {
      return Promise.reject(new Error('engine grant authority unavailable'));
    }
    granted.delete(grantKey(manifest));
    save();
    return Promise.resolve(null);
  }

  function clearGrants() {
    granted.clear();
    engineGrants.clear();
    engineState = 'uninitialized';
    engineClient = null;
    storage = null;
  }

  /** builtin 无 grant；esm 只有 engine exact identity + grant 可挂载。 */
  function canMount(manifest) {
    return !needsConsent(manifest) || isGranted(manifest);
  }

  /** widget 实际可用 capabilities；engine 镜像与 manifest 取防御性交集。 */
  function effectiveCapabilities(manifest) {
    if (!canMount(manifest)) return [];
    if (engineState === 'ready') {
      const record = engineGrantFor(manifest);
      const declared = manifest.capabilities || [];
      return record ? record.granted_capabilities.filter(cap => declared.includes(cap)) : [];
    }
    return manifest.capabilities || [];
  }

  return {
    STORAGE_KEY,
    initGrants,
    initGrantsFromEngine,
    markEngineUnavailable,
    configureEngineAuthority,
    hasEngineGrants,
    engineAuthorityState: () => engineState,
    isGranted,
    grant,
    revoke,
    clearGrants,
    needsConsent,
    canMount,
    effectiveCapabilities,
  };
});
