// C-P1：slot 注册面与 data-slot 挂载机制。
//
// slot = 页面上的 widget 挂载点（不是屏）。注册面机器可读、第三方可扩展：
//   - AIRPWidgetSlots.plan()          —— 导出当前 slot 计划的 JSON 快照
//   - AIRPWidgetSlots.registerSlot()  —— 运行期注册/替换一个 slot 条目
//   - AIRPWidgetSlots.applySlotPlan() —— 应用机器可读计划（slots.json / engine 下发）
//   - AIRPWidgetSlots.mountSlots()    —— 扫描 [data-slot] 占位并挂载
//
// 占位约定：屏 HTML 中 <div data-slot="chat.sidebar" aria-label="..."></div>；
// 引导脚本扫描 document 内全部 data-slot 元素，按注册计划挂载 widget。
// slot 命名规范：<screen>.<region>（如 chat.sidebar / workbench.grid）。
//
// 首批 slot（C-P1）：
//   chat.sidebar / chat.panel-right / settings.context /
//   diagnostics.context / workbench.grid
//
// 8h bearer 生命周期备忘：桌面壳注入的 desktop session token 会过期；
// 本阶段只做失败可见化（console-runtime 的 onRequest 钩子在 401 时置
// #runtime-status 提示），长驻 widget 会话的 token 续期留给 C-P2。
(function (root, factory) {
  const api = factory(root);
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetSlots = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function (root) {
  'use strict';

  /** slotId → { id, screen, region, description?, widgets: [{ instance, state }] } */
  const slots = new Map();

  /**
   * 注册/替换一个 slot 条目。widget 条目形如
   * { instance: { id, type, props? }, state? }；同一 slot 按数组顺序挂载。
   */
  function registerSlot(slot) {
    if (!slot || typeof slot.id !== 'string' || !slot.id) throw new Error('slot id is required');
    const parts = slot.id.split('.');
    slots.set(slot.id, {
      id: slot.id,
      screen: slot.screen || parts[0] || '',
      region: slot.region || parts.slice(1).join('.') || '',
      description: slot.description || '',
      widgets: Array.isArray(slot.widgets) ? slot.widgets : [],
    });
  }

  function getSlot(id) {
    return slots.get(id);
  }

  function slotNames() {
    return [...slots.keys()];
  }

  /** 机器可读快照：第三方扩展面（C-P2 catalog）可直接消费此形状。 */
  function plan() {
    return { version: 1, slots: [...slots.values()] };
  }

  /**
   * 应用机器可读 slot 计划：{ slots: [...] }。mode 缺省为 merge（按 id upsert），
   * "replace" 先清空再注册（对齐 manifest 的 set/patch 语义）。
   */
  function applySlotPlan(payload, mode) {
    if (!payload || !Array.isArray(payload.slots)) throw new Error('slot plan must be { slots: [...] }');
    if (mode === 'replace') slots.clear();
    for (const slot of payload.slots) registerSlot(slot);
  }

  /** 移除一个 slot 注册（不影响已挂载 DOM；挂载句柄随宿主销毁）。 */
  function unregisterSlot(id) {
    slots.delete(id);
  }

  /**
   * 扫描 doc 内全部 [data-slot] 元素并挂载已注册 widget。
   * 返回挂载句柄数组；重复调用只清空 slot DOM 占位并按当前计划重新挂载，
   * 不会销毁此前返回的旧句柄（首次挂载语义，非幂等重挂）——当前调用路径
   * 每页仅挂一次；动态重挂场景需调用方自行持有并销毁旧句柄。
   */
    function mountSlots(doc, options) {
    const d = doc || (typeof document !== 'undefined' ? document : null);
    if (!d) return [];
    const host = (options && options.host) || root.AIRPWidgetHost;
    const onIntent = options && options.onIntent;
    const handles = [];
    for (const node of Array.from(d.querySelectorAll('[data-slot]'))) {
      const id = node.getAttribute('data-slot');
      const slot = slots.get(id);
      node.replaceChildren();
      node.setAttribute('data-slot-state', slot && slot.widgets.length ? 'mounted' : 'empty');
      if (!slot) continue;
      for (const entry of slot.widgets) {
        const container = d.createElement('div');
        container.className = 'widget-slot-instance';
        node.appendChild(container);
                // 透传 doc：宿主创建 slot 容器用的 document（node 单测无全局 document）。
                // C-P2：onIntent 附加第三参 instance（{id,type}）——intent envelope
                // 的 widget_type/instance_id 由宿主从 slot 计划补齐，widget 不得伪造。
                const wrappedIntent = typeof onIntent === 'function'
                  ? (name, params) => onIntent(name, params, entry.instance)
                  : onIntent;
                const handle = host.mountWidget(container, entry.instance, entry.state, { onIntent: wrappedIntent, doc: d });
        handle.slotId = id;
        handles.push(handle);
      }
    }
    return handles;
  }

  /** 向某 slot 的全部已挂载 widget 推送新 state（按句柄上的 slotId 过滤）。 */
  function pushSlotState(handles, slotId, state) {
    let count = 0;
    for (const h of handles || []) {
      if (h && h.slotId === slotId && h.pushState) { h.pushState(state); count += 1; }
    }
    return count;
  }

  function clearSlots() {
    slots.clear();
  }

  return {
    registerSlot,
    unregisterSlot,
    getSlot,
    slotNames,
    plan,
    applySlotPlan,
    mountSlots,
    pushSlotState,
    clearSlots,
  };
});
