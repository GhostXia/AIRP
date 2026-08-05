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
  // registerEsmWidget 仍接受逐调用的 options.importer 覆写（测试友好）。
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
   * 模块 default 导出必须是 WidgetFactory。
   *
   * 纵深设防（二轮审查 W2）：BUG-6 门禁不能只靠 widget-host render 层
   * 单点强制——注册时即要求携带 manifest 引用（options.manifest）或显式
   * sandbox:true 标记（options.sandbox === true），否则在注册面即拒绝，
   * 关闭「无 manifest 的 esm 进程内加载」路径（C-P2 catalog 新增注册
   * 入口也无法绕过）。首方 builtin 走 registerModuleWidget，不受此门禁影响。
   *
   * @param {object} [options] { sandbox?: true, manifest?: object, importer?: fn }
   */
  function registerEsmWidget(type, source, options) {
    const opts = (options && typeof options === 'object' && typeof options !== 'function') ? options : {};
    if (opts.sandbox !== true && !opts.manifest) {
      throw new Error(
        'registerEsmWidget: esm widget requires a manifest reference or an explicit sandbox:true marker (BUG-6 defense in depth)',
      );
    }
    const doImport = opts.importer || defaultEsmImporter;
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
