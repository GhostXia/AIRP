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
// intent 执行面接 `POST /v1/widget-intents`（C-P3 逐调用强制，合同见
// protocol/widget-intents.json）。
//
// C-P3 base 统一 + consent 异步初始化：
// - catalog / grants / intent 远程调用统一用 engineBase()（airp_engine_url
//   优先，缺省 location.origin），跨源联调时不再因相对路径失败；
// - boot() 先 fetchEngineGrants() 注入 consent.js（engine 权威 grant 缓存），
//   再拉 catalog 与挂载；engine 不可达时降级到 consent.js 的 localStorage
//   UX 层缓存（C-P1 行为保留）。
import { createClockWidget } from './clock.module.js';

const registry = globalThis.AIRPWidgetRegistry;
const manifests = globalThis.AIRPWidgetManifests;
const consent = globalThis.AIRPWidgetConsent;
const slotsApi = globalThis.AIRPWidgetSlots;
const pluginDeps = globalThis.AIRPWidgetPluginDeps;

let booted = false;
let handles = [];

/** 8h bearer 过期等鉴权失败的可见化入口（屏运行时注入）。 */
let onAuthFailure = null;
export function setAuthFailureHandler(fn) {
  onAuthFailure = typeof fn === 'function' ? fn : null;
}

/**
 * C-P3：engine 远程 base 统一。与 console-runtime.js 的 connection.base 同语义：
 * airp_engine_url 优先（跨源联调），缺省 location.origin（同源 local-webui/desktop）。
 * 末尾斜杠归一，确保 `new URL('/v1/...', engineBase())` 拼接正确。
 */
function engineBase() {
  if (typeof sessionStorage !== 'undefined') {
    const url = sessionStorage.getItem('airp_engine_url');
    if (url) return url.replace(/\/+$/, '');
  }
  return typeof location !== 'undefined' ? location.origin : '';
}

/** 拼接 engine 绝对 URL；base 缺省时退化为相对路径（同源场景兼容）。 */
function engineUrl(path) {
  const base = engineBase();
  if (!base) return path;
  try {
    return new URL(path, base + '/').toString();
  } catch {
    return path;
  }
}

/** 取当前 bearer（与 console-runtime.js 同源：sessionStorage airp_bearer）。 */
function bearerToken() {
  return typeof sessionStorage !== 'undefined' ? sessionStorage.getItem('airp_bearer') : null;
}

function authedHeaders(extra) {
  const headers = Object.assign({}, extra || {});
  const bearer = bearerToken();
  if (bearer) headers.Authorization = 'Bearer ' + bearer;
  return headers;
}

function defaultIntentHandler(name, params, instance) {
  // C-P3：intent 执行面逐调用强制——POST /v1/widget-intents。
  // envelope 的 widget_type/instance_id 由宿主从 slot 计划补齐（第三参）。
  // capability 字段由 widget 在 instance.capability 声明（C-P3 boot.js base
  // 统一后，intent handler 不再省略 capability——engine 逐调用强制需要它）。
  // 失败不回传假交互：只留 console 痕迹（合同见 protocol/widget-intents.json）。
  const envelope = {
    name,
    params: params || {},
    widget_type: (instance && instance.type) || 'unknown',
    instance_id: (instance && instance.id) || 'unknown',
  };
  if (instance && instance.capability) envelope.capability = instance.capability;
  fetch(engineUrl('/v1/widget-intents'), {
    method: 'POST',
    headers: authedHeaders({ 'Content-Type': 'application/json' }),
    body: JSON.stringify(envelope),
  })
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
  const resp = await fetch(engineUrl('/v1/extensions/catalog'), {
    headers: authedHeaders({ Accept: 'application/json' }),
  });
  if (!resp.ok) throw new Error('HTTP ' + resp.status);
  return resp.json();
}

/**
 * C-P3：拉取 engine 权威 grant 状态并注入 consent.js。
 * 失败（engine 不可达 / 非 2xx）时静默降级——consent.js 仍用 localStorage
 * UX 层缓存（initGrants 已在 boot() 开头调用），canMount 行为同 C-P1。
 */
async function fetchEngineGrants() {
  try {
    const resp = await fetch(engineUrl('/v1/extensions/grants'), {
      headers: authedHeaders({ Accept: 'application/json' }),
    });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    const grants = await resp.json();
    consent.initGrantsFromEngine(grants);
  } catch (error) {
    console.warn('[widget-boot] engine grants 不可用，降级 consent localStorage 缓存：', error.message || error);
  }
}

/**
 * #498 §7.4：拉取已安装 trusted plugin 状态注入 plugin-deps.js。
 * 失败（engine 不可达 / 非 2xx）保持空缓存 → widget 声明的软依赖
 * 全部提示缺失（fail-closed：宁可多提示，不静默假装插件可用）。
 */
async function fetchEnginePlugins() {
  if (!pluginDeps) {
    console.warn('[widget-boot] plugin-deps.js 未加载，trusted plugin 降级提示不可用');
    return;
  }
  try {
    const resp = await fetch(engineUrl('/v1/plugins'), {
      headers: authedHeaders({ Accept: 'application/json' }),
    });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    const plugins = await resp.json();
    pluginDeps.initFromEngine(plugins);
  } catch (error) {
    console.warn('[widget-boot] engine plugins 不可用，trusted plugin 软依赖按缺失提示：', error.message || error);
  }
}

async function boot() {
  if (booted) return handles;
  booted = true;
  consent.initGrants();

  // builtin 首方 widget 注册（module kind，进程内、无 consent）。
  registry.registerModuleWidget('airp.clock', () => createClockWidget());

  // C-P3：先拉 engine 权威 grant 状态注入 consent（异步），再拉 catalog。
  // 两者都失败时降级为本地 slots.json + localStorage consent（C-P1 行为）。
  await fetchEngineGrants();

  // #498 §7.4：拉 trusted plugin 状态（降级提示的数据源；失败不阻塞）。
  await fetchEnginePlugins();

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
