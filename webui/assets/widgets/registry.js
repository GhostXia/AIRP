// C-P1：widget 注册面（对译 ui/src/registry/registry.ts 行为基准，去 Vue 化）。
//
// widget `type` → 实现的映射。注册面保持机器可读、第三方可扩展：
//  - module —— framework-agnostic WidgetModule（任意技术，见 widget-contract.js）
//  - esm    —— 以 ES module 从 `source` 加载，default 导出为 WidgetFactory
// 渲染端（widget-host.js）从不硬编码 widget 类型，一律在此查找。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetRegistry = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const registry = new Map();

  // esm widget 的默认动态导入器。宿主（或测试）可用 setDefaultEsmImporter
  // 覆写，把 source 映射到本地模块、加缓存或 consent 门控；
  // registerEsmWidget 仍接受逐调用的 importer 覆写（测试友好）。
  let defaultEsmImporter = (source) => import(source);

  function setDefaultEsmImporter(importer) {
    defaultEsmImporter = importer;
  }

  function registerWidget(type, widget) {
    registry.set(type, widget);
  }

  /** 注册 framework-agnostic module widget。 */
  function registerModuleWidget(type, load) {
    registry.set(type, { kind: 'module', load });
  }

  /**
   * 注册第三方 esm widget（manifest entry: { kind: "esm", source }）。
   * 模块 default 导出必须是 WidgetFactory。宿主的加载安全策略
   * （BUG-6：esm 必须 sandbox:true）在 widget-host 层强制，注册面只负责解析。
   */
  function registerEsmWidget(type, source, importer) {
    const doImport = importer || defaultEsmImporter;
    registry.set(type, {
      kind: 'module',
      load: async () => {
        const mod = await doImport(source);
        const factory = typeof mod === 'function' ? mod : mod && mod.default;
        if (typeof factory !== 'function') throw new Error('esm widget default export must be a WidgetFactory');
        return factory();
      },
    });
  }

  /** 移除已注册 widget（manifest `set` 全替换时使用）。 */
  function unregisterWidget(type) {
    registry.delete(type);
  }

  function resolveWidget(type) {
    return registry.get(type);
  }

  function registeredTypes() {
    return [...registry.keys()];
  }

  return {
    registerWidget,
    registerModuleWidget,
    registerEsmWidget,
    unregisterWidget,
    resolveWidget,
    registeredTypes,
    setDefaultEsmImporter,
  };
});
