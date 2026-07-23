// 方案 B 预留：Agent 每次只发一个 {action, target}，执行器返回 DOM 摘要 + 控制台错误 + 截图。
// MVP 不实现执行器，只定义协议契约，供后续 #273 阶段 3 接入。

export const ACTION_PROTOCOL_VERSION = 1;

export const SUPPORTED_ACTIONS = [
  'navigate',     // { action: 'navigate', target: '16-onboarding.html', params?: {} }
  'click',        // { action: 'click', target: '按钮文本' | '#selector' }
  'fill',         // { action: 'fill', target: '#message-input', value: '文本' }
  'wait',         // { action: 'wait', target: predicateId, timeoutMs?: 5000 }
  'snapshot',     // { action: 'snapshot' } → 返回 DOM 快照 + console + requests
  'screenshot',   // { action: 'screenshot' }
];

export function validateAction(action) {
  if (!action || typeof action !== 'object') return { ok: false, error: 'action must be object' };
  if (!SUPPORTED_ACTIONS.includes(action.action)) return { ok: false, error: 'unsupported action: ' + action.action };
  if (!action.target && action.action !== 'snapshot' && action.action !== 'screenshot') {
    return { ok: false, error: 'target required for ' + action.action };
  }
  return { ok: true };
}

// 预留执行器接口；MVP 抛 NotImplemented
export async function executeAction(harnessClient, action) {
  throw new Error('action-protocol executor not implemented in MVP (Plan A only); see #273 阶段 3');
}
