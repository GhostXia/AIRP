// C-P1：widget 运行时引导（ES module，页面以 <script type="module"> 引入）。
//
// 单点接入：屏页面只需追加一行
//   <script type="module" src="../assets/widgets/boot.js"></script>
// DOM 就绪后自动完成：consent 恢复 → builtin 注册 → 拉取机器可读计划
// → 扫描 [data-slot] 挂载。
//
// 依赖加载顺序（经典脚本，先于本模块执行）：
//   registry.js → manifests.js → consent.js → sandbox-bridge.js →
//   widget-host.js → slots.js
//
// C-P2 注册面切换：计划优先由 engine `GET /v1/extensions/catalog` 权威下发
// （含已安装第三方扩展的 manifest upsert + slot 编入）；engine 不可用
// （网络失败/非 2xx，如纯静态部署或 engine 无配置）时降级为本地静态
// slots.json——双保险，任何失败都不硬失败（slot 保持空占位也可接受）。
// intent 执行面接 `POST /v1/widget-intents`（C-P2 拒绝默认，合同见
// protocol/widget-intents.json）。
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

function defaultIntentHandler(name, params, instance) {
  // C-P2：intent 执行面最小合同——POST /v1/widget-intents（拒绝默认）。
  // envelope 的 widget_type/instance_id 由宿主从 slot 计划补齐（第三参）。
  // 失败不回传假交互：只留 console 痕迹（合同见 protocol/widget-intents.json）。
  const envelope = {
    name,
    params: params || {},
    widget_type: (instance && instance.type) || 'unknown',
    instance_id: (instance && instance.id) || 'unknown',
  };
  const headers = { 'Content-Type': 'application/json' };
  const bearer = typeof sessionStorage !== 'undefined' ? sessionStorage.getItem('airp_bearer') : null;
  if (bearer) headers.Authorization = 'Bearer ' + bearer;
  fetch('/v1/widget-intents', { method: 'POST', headers, body: JSON.stringify(envelope) })
    .then(async (resp) => {
      if (!resp.ok) {
        const body = await resp.json().catch(() => null);
        const code = body && body.error && body.error.code ? body.error.code : resp.status;
        console.warn('[widget-intent] engine 拒绝：' + name + '（' + code + '）');
      }
    })
    .catch((error) => {
      console.warn('[widget-intent] 投递失败：' + name, error);
    });
}

/** engine 权威下发计划（bearer 可选：local-webui 无鉴权模式无 token 也能拉）。 */
async function fetchEngineCatalog() {
  const headers = { Accept: 'application/json' };
  const bearer = typeof sessionStorage !== 'undefined' ? sessionStorage.getItem('airp_bearer') : null;
  if (bearer) headers.Authorization = 'Bearer ' + bearer;
  const resp = await fetch('/v1/extensions/catalog', { headers });
  if (!resp.ok) throw new Error('HTTP ' + resp.status);
  return resp.json();
}

async function boot() {
  if (booted) return handles;
  booted = true;
  consent.initGrants();

  // builtin 首方 widget 注册（module kind，进程内、无 consent）。
  registry.registerModuleWidget('airp.clock', () => createClockWidget());

  // 机器可读计划：优先 engine 权威下发，失败降级本地静态 slots.json；
  // 两者都失败时降级为空计划——slot 保持空占位，不硬失败。
  // manifests（set 语义）+ slots（replace 语义）。
  let plan = null;
  try {
    plan = await fetchEngineCatalog();
  } catch (error) {
    console.warn('[widget-boot] engine catalog 不可用，降级静态 slots.json：', error.message || error);
    try {
      const resp = await fetch(new URL('./slots.json', import.meta.url), { headers: { Accept: 'application/json' } });
      if (resp.ok) {
        plan = await resp.json();
      } else {
        console.warn('[widget-boot] slots.json 不可用：HTTP ' + resp.status);
      }
    } catch (fallbackError) {
      console.warn('[widget-boot] slots.json 加载失败：', fallbackError);
    }
  }
  if (plan) {
    if (Array.isArray(plan.manifests)) manifests.applyManifestMessage('set', plan.manifests);
    slotsApi.applySlotPlan(plan, 'replace');
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
