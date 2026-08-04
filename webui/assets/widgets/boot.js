// C-P1：widget 运行时引导（ES module，页面以 <script type="module"> 引入）。
//
// 单点接入：屏页面只需追加一行
//   <script type="module" src="../assets/widgets/boot.js"></script>
// DOM 就绪后自动完成：consent 恢复 → builtin 注册 → 拉取机器可读计划
// （slots.json：manifests + slots）→ 扫描 [data-slot] 挂载。
//
// 依赖加载顺序（经典脚本，先于本模块执行）：
//   registry.js → manifests.js → consent.js → sandbox-bridge.js →
//   widget-host.js → slots.js
//
// 第三方扩展性（用户硬约束）：注册面全部机器可读——
//   AIRPWidgetSlots.plan() / AIRPWidgetManifests.allManifests() 可 JSON 导出；
//   C-P2 engine 扩展注册面交付后，slots.json 的内容将由
//   GET /v1/extensions + catalog 端点权威下发，本文件只保留首方示范集。
import { createClockWidget } from './clock.module.js';

const registry = globalThis.AIRPWidgetRegistry;
const manifests = globalThis.AIRPWidgetManifests;
const consent = globalThis.AIRPWidgetConsent;
const slotsApi = globalThis.AIRPWidgetSlots;

let booted = false;
let handles = [];

/** 8h bearer 过期等鉴权失败的可见化入口（屏运行时注入）。 */
let onAuthFailure = null;
export function setAuthFailureHandler(fn) {
  onAuthFailure = typeof fn === 'function' ? fn : null;
}

function defaultIntentHandler(name, params) {
  // C-P1：intent 尚未接 engine 执行面（C-P2/P3），先落控制台留痕，不做假交互。
  console.info('[widget-intent]', name, params || {});
}

async function boot() {
  if (booted) return handles;
  booted = true;
  consent.initGrants();

  // builtin 首方 widget 注册（module kind，进程内、无 consent）。
  registry.registerModuleWidget('airp.clock', () => createClockWidget());

  // 机器可读计划：manifests（set 语义）+ slots（replace 语义）。
  // fetch 失败（如静态目录被裁剪）时降级为空计划——slot 保持空占位。
  try {
    const resp = await fetch(new URL('./slots.json', import.meta.url), { headers: { Accept: 'application/json' } });
    if (resp.ok) {
      const plan = await resp.json();
      if (Array.isArray(plan.manifests)) manifests.applyManifestMessage('set', plan.manifests);
      slotsApi.applySlotPlan(plan, 'replace');
    } else {
      console.warn('[widget-boot] slots.json 不可用：HTTP ' + resp.status);
    }
  } catch (error) {
    console.warn('[widget-boot] slots.json 加载失败：', error);
  }

  handles = slotsApi.mountSlots(document, { onIntent: defaultIntentHandler });
  return handles;
}

/** 手动引导入口（页面需要自定义时序时调用；默认 DOM 就绪自动执行）。 */
export function bootWidgetSlots() {
  return boot();
}

/** 向某 slot 的全部已挂载 widget 推送新 state（屏运行时调用）。 */
export function pushSlotState(slotId, state) {
  return slotsApi.pushSlotState(handles, slotId, state);
}

/** 暴露挂载句柄（调试/测试）。 */
export function widgetHandles() {
  return handles;
}

export function notifyAuthFailure(detail) {
  if (onAuthFailure) onAuthFailure(detail);
}

// DOM 就绪即自动引导；module 脚本默认 defer，通常已就绪。
// 把「挂载完成 → 公开 API」的 Promise 挂到 window.__airpWidgetBoot：
// console-runtime / chat-space 等经典脚本先于本模块执行，它们轮询此全局
// 拿到 API 后再接线（401 可见化、slot state 推送）。
const publicApi = { bootWidgetSlots, pushSlotState, setAuthFailureHandler, widgetHandles, notifyAuthFailure };
const booting = document.readyState === 'loading'
  ? new Promise(resolve => { document.addEventListener('DOMContentLoaded', () => resolve(boot()), { once: true }); })
  : boot();
globalThis.__airpWidgetBoot = Promise.resolve(booting)
  .then(() => publicApi)
  .catch(error => {
    console.warn('[widget-boot] 引导失败：', error);
    return null;
  });
