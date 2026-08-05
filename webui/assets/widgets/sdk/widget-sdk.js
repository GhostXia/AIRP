// C-P4-4：Widget SDK 骨架。
//
// 目标：给第三方 widget 作者一个低摩擦、零构建的起点。作者仍可直接写
// `export default function createWidget() { return { mount, unmount } }`，
// SDK 只是把高频套路（错误捕获 / 生命周期日志 / manifest 校验 / DOM 构建）
// 收敛成可选辅助，让作者专注于 widget 逻辑本身。
//
// 设计原则：
// 1. **零构建**：纯 ESM，浏览器与 node --test 直接 import；无打包步骤。
// 2. **渐进式**：作者可只用其中一部分（如只用 defineManifest 校验，不用
//    createWidget 包装）；SDK 不接管 widget 生命周期控制权。
// 3. **不削弱安全边界**：SDK 运行在沙箱 iframe 内（opaque origin），拿不到
//    宿主对象引用；它只是 widget 内部代码的组织辅助，不提供任何"突破沙箱"
//    或"绕过 capability"的路径。
// 4. **与 engine 合同对齐**：defineManifest 的 host_api 校验与 engine
//    `parse_host_api_major`（engine/src/extensions/mod.rs）同语义，让作者
//    在本地即能发现不兼容问题，而不是装到 engine 才被拒。
//
// 与 widget-contract.js 的关系：本文件是可执行辅助，widget-contract.js 是
// 纯 JSDoc 合同声明。两者互补：作者读 widget-contract.js 理解接口，用
// 本文件减少样板代码。

/**
 * @typedef {Object} Capability
 * @description widget 可申请的 capability（C-P3 engine 权威签发）。
 *   枚举见 widget-contract.js：read:memory | write:memory | read:worldbook |
 *   read:state | write:state | call:tool
 * @type {string}
 */

/**
 * @typedef {Object} WidgetInstance
 * @property {string} id 实例 id（宿主分配）
 * @property {string} type widget 类型（与 manifest.type 一致）
 * @property {unknown} [props] 实例属性
 * @property {unknown} [state] 初始 state 切片
 * @property {Capability[]} [capabilities] 宿主已授予的 capability 子集
 */

/**
 * @typedef {Object} WidgetContext
 * @property {WidgetInstance} instance 本实例
 * @property {() => unknown} getState 读取当前 state 切片
 * @property {(cb: (state: unknown) => void) => () => void} onState 订阅 state 变化；返回退订函数
 * @property {(intent: string, params?: unknown) => void} emit 向宿主发出 intent
 * @property {Capability[]} capabilities 宿主已授予的 capability（逐调用强制的输入）
 */

/**
 * @typedef {Object} WidgetModule
 * @property {(el: HTMLElement, ctx: WidgetContext) => void | Promise<void>} mount 挂载到容器
 * @property {() => void} [unmount] 卸载清理
 */

/**
 * @typedef {() => WidgetModule} WidgetFactory esm 默认导出的工厂
 */

/**
 * @typedef {Object} WidgetEntry
 * @property {"builtin" | "esm"} kind builtin=首方进程内；esm=第三方沙箱
 * @property {string} [source] esm 加载源（安装时被 engine 强制改写为 /extensions/<digest>/index.js）
 * @property {boolean} [sandbox] 第三方 esm 必须为 true（BUG-6 fail-closed）
 */

/**
 * @typedef {Object} WidgetDef
 * @property {string} type 全局唯一 widget 类型（ns.name 两段）
 * @property {string} version semver
 * @property {string} [title]
 * @property {string} [author]
 * @property {Capability[]} [capabilities] 申请的 capability
 * @property {string} [host_api] C-P4-3：宿主合同 major 版本（如 "1"、"1.2.3"）；缺省视为 "1"
 * @property {WidgetEntry} [entry] 加载入口
 */

/**
 * 包装一个 WidgetFactory，注入错误捕获与生命周期日志。
 *
 * - mount/unmount 抛错被捕获并转 onError 回调；不炸宿主（widget-host.js
 *   的 teardown 容错是第二道，SDK 是第一道，让 widget 作者自己能看到错误）。
 * - dev 模式（options.debug 或 globalThis.__AIRP_WIDGET_DEBUG）下打印
 *   mount/unmount 生命周期日志，方便作者本地调试。
 * - 返回的仍是标准 WidgetFactory，宿主无感知（SDK 渐进式：作者可选不用）。
 *
 * @param {WidgetFactory} factory 原始工厂
 * @param {{ onError?: (e: unknown) => void, debug?: boolean }} [options]
 * @returns {WidgetFactory}
 */
export function createWidget(factory, options) {
  const opts = options || {};
  const onError = typeof opts.onError === 'function' ? opts.onError : noop;
  const debug = opts.debug === true || (typeof globalThis !== 'undefined' && globalThis.__AIRP_WIDGET_DEBUG);
  // guarded helper：onError 自身抛异常时必须吞掉（审计 #489 W1）。
  // mount/unmount 错误容纳是 SDK 对宿主的合同；若上报回调再抛，会把原始
  // 生命周期失败重新泄漏给宿主（同步路径逃逸 / async 路径变 rejected
  // promise），违背容纳合同。onError 只是作者的可观测窗口，不是控制流。
  const report = (e) => {
    try {
      onError(e);
    } catch (_) {
      // 吞掉上报异常：容纳优先于上报。
    }
  };

  return function wrappedFactory() {
    const mod = factory();
    return {
      mount(el, ctx) {
        try {
          if (debug) log('mount', ctx && ctx.instance && ctx.instance.type);
          const r = mod.mount(el, ctx);
          if (r && typeof r.then === 'function') {
            return r.catch((e) => { report(e); });
          }
          return r;
        } catch (e) {
          report(e);
        }
      },
      unmount() {
        if (typeof mod.unmount !== 'function') return;
        try {
          if (debug) log('unmount');
          mod.unmount();
        } catch (e) {
          report(e);
        }
      },
    };
  };
}

/**
 * 校验并规范化 widget manifest。与 engine `validate_manifest`（engine/src/
 * extensions/mod.rs）的 host_api 校验同语义，让作者在本地即能发现不兼容
 * 问题（如 host_api="2" 装到 major=1 的 engine 会被拒）。
 *
 * @param {WidgetDef} manifest
 * @returns {WidgetDef} 冻结的 manifest（防止作者意外修改）
 * @throws {Error} 校验失败时抛错，message 含失败原因
 */
export function defineManifest(manifest) {
  if (!manifest || typeof manifest !== 'object') {
    throw new Error('defineManifest: manifest must be an object');
  }
  if (!manifest.type || typeof manifest.type !== 'string') {
    throw new Error('defineManifest: manifest.type is required');
  }
  if (!manifest.version || typeof manifest.version !== 'string') {
    throw new Error('defineManifest: manifest.version is required');
  }
  // host_api 校验：与 engine parse_host_api_major 同语义（见 C-P4-3）。
  // 缺省视为 "1"（向后兼容）；非法格式抛错让作者在本地修正。
  if (manifest.host_api != null) {
    parseHostApiMajor(manifest.host_api);
  }
  if (manifest.entry && typeof manifest.entry !== 'object') {
    throw new Error('defineManifest: manifest.entry must be an object');
  }
  // 第三方 esm 必须显式 sandbox:true（BUG-6 fail-closed，与 widget-host.js
  // sandboxEnforced 同语义）。defineManifest 不强制要求 sandbox（builtin
  // 不需要），但 esm 缺 sandbox 即抛错，让作者在本地发现。
  if (manifest.entry && manifest.entry.kind === 'esm' && manifest.entry.sandbox !== true) {
    throw new Error('defineManifest: esm entry must have sandbox === true (BUG-6 fail-closed)');
  }
  // 审计 #489 W2：只冻结外层不够——调用方在校验后仍可 mutate
  // result.entry.sandbox / result.capabilities。clone 并深冻结嵌套的
  // entry 与 capabilities，让 manifest 真正不可变（ESM strict 下赋值抛
  // TypeError，sloppy 下静默失败但 isFrozen 可断言）。
  const result = { ...manifest };
  if (result.entry && typeof result.entry === 'object') {
    result.entry = Object.freeze({ ...result.entry });
  }
  if (Array.isArray(result.capabilities)) {
    result.capabilities = Object.freeze([...result.capabilities]);
  }
  return Object.freeze(result);
}

/**
 * DOM 构建辅助：`h('div', { className: 'x', onClick: fn }, child1, child2)`。
 * 简化作者高频的 createElement + 属性 + 事件 + 子节点套路。不接管模板系统，
 * 作者仍可直接用原生 DOM 或引入自己的框架。
 *
 * @param {string} tag 标签名
 * @param {Record<string, unknown>} [props] 属性 / 事件（onXxx 识别为事件）
 * @param {...(Node | string | null | undefined)} children 子节点
 * @returns {HTMLElement}
 */
export function h(tag, props, ...children) {
  const el = document.createElement(tag);
  if (props) {
    for (const key of Object.keys(props)) {
      const val = props[key];
      if (val == null) continue;
      if (key === 'className') {
        el.className = String(val);
      } else if (key === 'style' && typeof val === 'object') {
        for (const k of Object.keys(val)) el.style[k] = val[k];
      } else if (key.startsWith('on') && typeof val === 'function') {
        el.addEventListener(key.slice(2).toLowerCase(), val);
      } else if (key === 'dataset' && typeof val === 'object') {
        for (const k of Object.keys(val)) el.dataset[k] = String(val[k]);
      } else {
        el.setAttribute(key, String(val));
      }
    }
  }
  for (const child of children) {
    if (child == null) continue;
    el.append(child);
  }
  return el;
}

// ── 内部辅助 ────────────────────────────────────────────────────────────

function noop() {}

function log(phase, type) {
  // 沙箱 iframe 内 console 可用（allow-scripts）；opaque origin 不影响 console。
  try {
    // eslint-disable-next-line no-console
    console.debug('[airp-widget]', phase, type || '');
  } catch (_) {
    // console 不可达时静默（不应发生，但 defensive）。
  }
}

/**
 * 解析 host_api major 版本。与 engine `parse_host_api_major` 同语义：
 * 接受 "1" / "1.0" / "1.2.3"；拒绝 "0" / "01" / "abc" / "1.x" / "" / 超长。
 * 缺省（undefined / null）视为 "1"，由调用方处理；本函数只校验非空值。
 *
 * @param {string} raw
 * @returns {number} major 版本
 * @throws {Error} 非法格式
 */
function parseHostApiMajor(raw) {
  if (typeof raw !== 'string' || raw === '') {
    throw new Error('host_api must be a non-empty string (or omitted for "1")');
  }
  const parts = raw.split('.');
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    const isMajor = i === 0;
    if (
      part === ''
      || part.length > 8
      || (isMajor && part === '0')
      || (part.length > 1 && part[0] === '0')
      || !/^[0-9]+$/.test(part)
    ) {
      throw new Error(
        'host_api segment must be a non-negative integer without leading zeros: ' + raw,
      );
    }
  }
  return Number(parts[0]);
}
