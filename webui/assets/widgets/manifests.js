// C-P1：manifest 注册表（对译 ui/src/registry/manifests.ts 行为基准）。
//
// widget `type` → 已发布 manifest（WidgetDef）的映射：entry（如何加载）、
// props/state schema、申请的 capabilities。registerEsmWidgetsFromManifests
// 把 entry.kind === "esm" 的 manifest 接入组件注册面。
//
// 供线上 `manifest` 消息驱动（C-P2 engine 扩展注册面交付后经 SSE/HTTP 送达）：
// op:"set" 全替换（先 clearManifests）；op:"patch" 按 type upsert
// （manifest 的 patch 是 manifests 数组的 upsert，不是 RFC 6902 JSON Patch）。
(function (root, factory) {
  const api = factory(root.AIRPWidgetRegistry);
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetManifests = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function (registryModule) {
  'use strict';

  // node --test 场景下直接 require，registry 未先加载时兜底自取。
  const registry = registryModule || require('./registry.js');

  const manifests = new Map();
  // 本模块注册进组件注册面的 type（仅 esm widget），
  // 使 `set`（全替换）能摘除过期条目而不触碰 builtin。
  const esmRegistered = new Set();

  function registerManifest(manifest) {
    manifests.set(manifest.type, manifest);
  }

  function getManifest(type) {
    return manifests.get(type);
  }

  function allManifests() {
    return [...manifests.values()];
  }

  /**
   * 摘除全部已记录 manifest，并注销它们带入的 esm widget
   * （builtin 在别处注册，不受影响）。`manifest op:"set"` 全量重置用。
   */
  function clearManifests() {
    for (const type of esmRegistered) registry.unregisterWidget(type);
    esmRegistered.clear();
    manifests.clear();
  }

  /** 记录 manifest 并把其中的 esm widget 自动注册进组件注册面。importer 可注入（测试）。 */
  function registerEsmWidgetsFromManifests(list, importer) {
    for (const manifest of list || []) {
      registerManifest(manifest);
      if (manifest.entry && manifest.entry.kind === 'esm' && manifest.entry.source) {
        // 纵深设防（W2）：注册面只接受声明了 sandbox:true 的 esm；
        // 未声明者仍记录 manifest，由 widget-host render 层的 BUG-6
        // fail-closed 拒载展示（行为不变，双点设防）。
        if (manifest.entry.sandbox === true) {
          registry.registerEsmWidget(manifest.type, manifest.entry.source, { sandbox: true, importer });
          esmRegistered.add(manifest.type);
        }
      }
    }
  }

  /**
   * 应用下游 `manifest` 消息：op:"set" 先清空再注册（全替换）；
   * op:"patch" 按 type upsert 子集（增量）。importer 可注入（测试）。
   */
  function applyManifestMessage(op, list, importer) {
    if (op === 'set') clearManifests();
    registerEsmWidgetsFromManifests(list, importer);
  }

  return {
    registerManifest,
    getManifest,
    allManifests,
    clearManifests,
    registerEsmWidgetsFromManifests,
    applyManifestMessage,
  };
});
