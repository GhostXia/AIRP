// C-P1：widget 契约类型（对译 ui/src/protocol/types.ts widget 切片 +
// ui/src/registry/widget-module.ts）。零构建环境下以 JSDoc 表达，
// 运行时不产生任何代码——本文件是作者与宿主之间的成文合同。
//
// 责任边界（docs/SECURITY.md）：宿主自保（强制 capability、隔离自身密钥、
// 取得 consent、containment 错误），不审计 widget 代码；安装 widget 是
// 用户的选择与风险。
//
// ── Capability（能力令牌，C-P3 engine 权威签发前的宿主侧枚举） ──
//   "read:memory" | "write:memory" | "read:worldbook" |
//   "read:state"  | "write:state"  | "call:tool"
//
// ── WidgetInstance（engine/宿主持有的实例） ──
//   { id: string, type: string, props?: Json, state?: Json, capabilities?: Capability[] }
//
// ── WidgetDef（manifest，机器可读注册面条目） ──
//   {
//     type: string, version: string, title?: string,
//     propsSchema?: Json, stateSchema?: Json,
//     capabilities?: Capability[], intents?: string[],
//     entry?: WidgetEntry, author?: string, description?: string,
//   }
//
// ── WidgetEntry（如何加载） ──
//   { kind: "builtin" }                              // 首方，进程内，无 consent
//   { kind: "esm", source: string, sandbox: true }   // 第三方；BUG-6 门禁：
//                                                    //   sandbox 必须显式 true，
//                                                    //   缺失即拒载（widget-host.js）
//
// ── WidgetContext（宿主在 mount 时交给 widget） ──
//   {
//     instance: WidgetInstance,               // 本实例（id/type/props/请求的 caps）
//     getState(): unknown,                    // 读取本 widget 作用域的当前 state 切片
//     onState(cb: (state) => void): () => void, // 订阅 state 变化；返回退订函数
//     emit(intent: string, params?: Json): void,  // 向上发出 intent（用户动作）
//     capabilities: Capability[],             // 宿主已授予的 capability（强制）
//   }
//
// ── WidgetModule（widget 实现；任何技术均可：vanilla DOM/框架/Web Component） ──
//   { mount(el: HTMLElement, ctx: WidgetContext): void | Promise<void>,
//     unmount?(): void }
//
// ── WidgetFactory（esm widget 模块的 default 导出） ──
//   () => WidgetModule
//
// ── 沙箱消息协议（host ↔ opaque-origin iframe，见 sandbox-bridge.js） ──
//   host → iframe：{ kind: "mount", instance, capabilities } / { kind: "state", state }
//   iframe → host：{ kind: "ready" } / { kind: "intent", name, params } / { kind: "error", message }
export {};
