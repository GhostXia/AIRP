# Widget 开发指南

> 读者：第三方 widget 作者 / AIRP 贡献者
>
> 最后校准：2026-08-05，C-P4-4（Widget SDK 骨架）
>
> 真理顺序：源码 / manifest / 测试 > 本文档 > widget-contract.js JSDoc > 历史归档。

本文档是 widget 作者的入门与合同参考。widget 系统的安全模型与架构决策见 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)；intent 执行面合同以 [protocol/widget-intents.json](../protocol/widget-intents.json) 为机器可读唯一事实源；扩展注册面合同以 `engine/src/extensions/` 源码为准。

## 1. Widget 模型

AIRP widget 有两种加载形态：

| 形态 | `entry.kind` | 运行环境 | consent | 适用 |
|---|---|---|---|---|
| **builtin** | `"builtin"` | 进程内（与宿主同 origin） | 不需要 | 首方 widget |
| **esm + sandbox** | `"esm"` | opaque-origin sandboxed iframe | 需要 | 第三方 widget |

第三方 widget（esm）的安全边界：

- iframe 以 `sandbox="allow-scripts"` 创建，**不带** `allow-same-origin`，运行于 opaque origin。
- widget 读不到宿主 DOM / `localStorage` / cookie / 同源网络。
- `WidgetContext` 由宿主经 `postMessage` 代理（见 `sandbox-bridge.js` / `sandbox-frame.js`），widget 拿不到宿主对象引用。
- 第三方 esm 必须显式 `entry.sandbox === true`，缺失即拒载（BUG-6 fail-closed，见 `widget-host.js` 的 `sandboxEnforced`）。

## 2. 快速开始

最简第三方 widget（不使用 SDK）：

```js
// my-widget.js
export default function createMyWidget() {
  let unsubscribe;
  return {
    mount(el, ctx) {
      el.textContent = 'hello ' + (ctx.instance.id);
      unsubscribe = ctx.onState((s) => {
        el.textContent = 'state: ' + JSON.stringify(s);
      });
    },
    unmount() {
      if (unsubscribe) unsubscribe();
    },
  };
}
```

对应的 manifest：

```json
{
  "type": "acme.my-widget",
  "version": "1.0.0",
  "host_api": "1",
  "capabilities": ["read:state"],
  "entry": { "kind": "esm", "source": "./my-widget.js", "sandbox": true }
}
```

使用 SDK 骨架（推荐，见第 4 节）可获得错误捕获、生命周期日志、manifest 本地校验与 DOM 辅助。

## 3. Manifest 字段

manifest 是 widget 的机器可读身份声明，engine 在安装时校验并持久化。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `type` | string | 是 | 全局唯一 widget 类型，`ns.name` 两段（如 `acme.clock`）；小写字母数字与 `.-_`，禁路径字符，≤128 字符 |
| `version` | string | 是 | semver，≤64 字符 |
| `title` | string | 否 | 展示名 |
| `author` | string | 否 | 作者 |
| `capabilities` | string[] | 否 | 申请的 capability（见第 5 节）；缺省为空 |
| `host_api` | string | 否 | C-P4-3：宿主合同 major 版本（如 `"1"`、`"1.2.3"`）；缺失或空串视为 `"1"` |
| `trusted_plugins` | object[] | 否 | #498 §7.1：trusted plugin 软依赖声明（见下文）；缺省为空 |
| `entry` | object | 是 | 加载入口 |

### `entry` 字段

| 字段 | builtin | esm | 说明 |
|---|---|---|---|
| `kind` | `"builtin"` | `"esm"` | 加载形态 |
| `source` | — | string | esm 加载源；**安装时被 engine 强制改写为 `/extensions/<digest>/index.js`**（R0 硬门禁：跨源加载路径在安装面即被消灭） |
| `sandbox` | — | `true` | 第三方 esm 必须显式 `true`；缺失即拒载 |

### `host_api` 语义（C-P4-3 前向兼容铁律）

engine 当前支持 `HOST_API_MAJOR = 1`（见 `engine/src/extensions/mod.rs`）。安装时校验 `parse_host_api_major(manifest.host_api) == HOST_API_MAJOR`：

- 接受 `"1"` / `"1.0"` / `"1.2.3"`，取首段为 major。
- 拒绝 `"0"`（major 0 不合法）/ `"01"`（前导零）/ `"abc"` / `"1.x"` / `"1."` / 超长段（单段 >8 字符）→ `invalid_manifest`。
- 跨 major（如 widget 声明 `"2"` 装到 major=1 的 engine）→ `host_api_incompatible`，拒绝安装。

**前向兼容铁律**：engine 不会静默尝试不兼容的 widget。当 engine 升级到 `HOST_API_MAJOR = 2` 时，旧 widget（`host_api = "1"`）安装即被拒，强迫作者显式声明兼容性；反之亦然。这避免“widget 装上去但行为不符”的隐性故障，把不兼容暴露在安装面。

**空值规则（#489 D1 定夺，2026-08-05）**：`host_api` 缺失或空串（`""`）一律视为缺省 `"1"`，向后兼容已有 widget（实现见 `parse_host_api_major`）。空串不属于拒绝列表。

### `trusted_plugins` 软依赖（#498 §7.1）

widget 可声明可选依赖的 trusted plugin（跨进程 HTTP 调用目标，见 [TRUSTED-PLUGINS.md §6.1](TRUSTED-PLUGINS.md)）：

```json
"trusted_plugins": [
  { "id": "com.example.tts", "min_host_api": "1.2" }
]
```

- **软依赖**：声明了但插件缺失/未运行 → widget **仍可加载**并自行降级；engine 不强制匹配，只随 catalog 下发，由 webui 决定怎么提示。
- **engine 安装面校验**（fail-closed：坏条目拒绝整包）：`id` 非空、≤128 字符、无路径分隔符（同插件 id 规则）；`min_host_api` 若出现必须是纯 semver 格式。**显式空串 `""` 也拒绝**（#507 定夺）——缺省语义只能靠 omit 字段表达，空串不沿用 `host_api` 的"空串视为 1"向后兼容规则。
- **webui 四态判定**（挂载前对照 `/v1/plugins` 缓存，`plugin-deps.js`）：`not-installed`（插件不存在）/ `stopped`（已安装未运行）/ `version-too-low`（运行中但 `host_api` 低于 `min_host_api`，逐段数值比较，脏数据 fail-closed）/ 满足（running 且版本足够）。前三态渲染非阻塞降级提示条；engine 不可达或 5s 超时（`AbortSignal.timeout`）时缓存为空 → 全部按缺失提示。

## 4. Widget SDK 骨架

SDK 位于 `webui/assets/widgets/sdk/`，是可选的渐进式辅助。作者可全用、部分用或不用。

| 文件 | 用途 |
|---|---|
| `widget-sdk.js` | 核心：`createWidget` / `defineManifest` / `h` + JSDoc 类型 |
| `example-widget.js` | 完整示例 widget，可复制为起点 |
| `example-manifest.json` | manifest 脚手架 |

### `createWidget(factory, options)`

包装一个 `WidgetFactory`，注入错误捕获与生命周期日志：

- `mount` / `unmount` 抛错被捕获并转 `onError` 回调，不炸宿主（widget-host.js 的 teardown 容错是第二道，SDK 是第一道）。
- `options.debug` 或 `globalThis.__AIRP_WIDGET_DEBUG` 为 true 时打印 `mount`/`unmount` 生命周期日志。
- 返回的仍是标准 `WidgetFactory`，宿主无感知。

```js
import { createWidget } from './widget-sdk.js';

export default createWidget(function createMyWidget() {
  return { mount(el, ctx) { /* ... */ }, unmount() { /* ... */ } };
}, {
  onError: (e) => console.error('[my-widget]', e),
  debug: false,
});
```

### `defineManifest(manifest)`

校验并冻结 manifest。`host_api` 校验与 engine `parse_host_api_major` 同语义，让作者在本地即能发现不兼容问题。esm entry 缺 `sandbox === true` 即抛错（BUG-6 fail-closed）。

```js
import { defineManifest } from './widget-sdk.js';

export const manifest = defineManifest({
  type: 'acme.my-widget',
  version: '1.0.0',
  host_api: '1',
  capabilities: ['read:state'],
  entry: { kind: 'esm', source: './my-widget.js', sandbox: true },
});
```

### `h(tag, props, ...children)`

DOM 构建辅助，简化高频的 `createElement` + 属性 + 事件 + 子节点套路。`onXxx` 识别为事件监听；`className` / `style` / `dataset` 特殊处理；`null`/`undefined` 子节点与属性值被跳过。

```js
import { h } from './widget-sdk.js';

const btn = h('button', {
  className: 'ping-btn',
  onClick: () => ctx.emit('acme.my-widget.ping', { id: ctx.instance.id }),
}, '发出 intent');
```

## 5. Capability 与 consent（C-P3）

第三方 widget 接触敏感数据或触发特权动作前，必须有 engine 侧 capability 强制。仅靠 UI 检查不够（[UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) 必须改写第 3 条）。

### capability 枚举

```text
read:memory | write:memory | read:worldbook | read:state | write:state | call:tool
```

### 授权链路

权限 = `manifest.capabilities` ∩ 用户请求 ∩ engine policy：

1. **声明**：widget 在 manifest 声明所需 capability。
2. **consent**：用户在 webui 授权 UI 同意（grantKey = `type@version#source`）。
3. **grant**：engine `POST /v1/extensions/:id/grants` 签发 grant，持久化到 `extensions.json` 的 `granted_capabilities` 字段。
4. **逐调用强制**：widget 发 intent 时，engine 校验 `envelope.capability ∈ granted_capabilities`（C-P3 逐调用强制）。

宿主启动时先从 `GET /v1/extensions/grants` 读取 engine 快照（合同见
[protocol/widget-grants.json](../protocol/widget-grants.json)），并校验 `type`、
`version`、`source`、`digest` 与 `enabled`。成功空快照表示当前没有授权；请求失败、
快照畸形或身份不匹配都必须 fail-closed，不能用浏览器 `localStorage` 放行。批准/撤销
也只有在对应 POST mutation 成功后才更新 UI 内存镜像。

重装（同 type 不同 digest）清空 grant——consent 不跨身份延续。

### `ctx.capabilities`

`WidgetContext.capabilities` 是 engine 签发的 grant 子集。即使 manifest 声明了 `read:state`，若用户未 consent，`ctx.capabilities` 为空。widget 应据此决定渲染逻辑（如降级展示）。

## 6. WidgetContext API

宿主在 `mount(el, ctx)` 时交给 widget 的上下文：

```ts
interface WidgetContext {
  instance: WidgetInstance;        // 本实例 { id, type, props?, state?, capabilities? }
  getState(): unknown;             // 读取当前 state 切片
  onState(cb: (state) => void): () => void;  // 订阅 state 变化；返回退订函数
  emit(intent: string, params?: unknown): void;  // 向宿主发出 intent
  capabilities: Capability[];      // 宿主已授予的 capability 子集
}
```

- `getState()` / `onState()` 操作的是本 widget 作用域的 state 切片，不是全局 RP 数据真相（engine 拥有真相；widget 只渲染）。
- `emit()` 发出的 intent 经宿主 webui 以 `POST /v1/widget-intents` 送达 engine；intent 名在合同内自由命名（建议 `ns.action`），engine 不以名字白名单拒绝——拒绝语义只来自授权层。

## 7. Intent 合同

intent 执行面合同以 [protocol/widget-intents.json](../protocol/widget-intents.json) 为机器可读唯一事实源。关键点：

- 端点：`POST /v1/widget-intents`，bearer 鉴权。
- envelope 的 `widget_type` / `instance_id` / `capability` 由宿主 webui 从 slot 计划补齐，**widget 代码不得自行伪造**。
- C-P2 为最小合同：engine 无任何已注册执行器，一切 intent 返回 `403 intent_denied`（拒绝默认）。
- C-P3 起按 `capability` 字段做逐调用强制。
- 错误 code 封闭锁定集：`intent_invalid`（400，envelope 形状错误）/ `intent_denied`（403，授权失败或无执行器）。
- 传输层拒收（畸形 JSON / 缺必填字段 / Content-Type 错误）由 axum Json 提取器在 handler 前拒收：4xx 纯文本响应，**无 error.code**（合同见 widget-intents.json 的 `transportRejections`）；宿主不得按 error.code 分支处理此类响应。
- 宿主收到 403/400 不崩溃、不静默吞掉；向 widget 回传错误并留 console 痕迹。

## 8. 打包与安装

第三方 widget 发布为 **digest-pinned 静态包**：

1. 准备 manifest + widget 文件（esm）+ 依赖资产。
2. 计算每个文件的 SHA-256。
3. `POST /v1/extensions/install` 提交 `manifest` + `files[]`（每个文件含 `path` / `content_base64` / `sha256`）。
4. engine 校验：manifest 形状 / `host_api` major / `entry.sandbox` / 文件摘要比对 / slot 合法性。
5. engine 强制改写 `entry.source` 为 `/extensions/<digest>/index.js`（R0 硬门禁）。
6. 落盘到 `data_root/extensions/<digest>/`，记录写入 `extensions.json`。
7. 静态服务：`GET /extensions/<digest>/*` 鉴权层外投放 + `ACAO:*`（opaque-origin 沙箱 CORS import 必需）+ `immutable` 长缓存；服务时复检摘要防篡改。

### 限制

- 单包文件数 ≤ 32；单文件 ≤ 1MB；包总大小 ≤ 4MB（见 `engine/src/extensions/mod.rs` 常量）。
- 同一 `type` 至多一条记录：重装（同 type 不同 digest）即替换，且清空 grant。

## 9. 测试策略

- **SDK 单元测试**：`webui/tests/widget-sdk.test.mjs`，覆盖 `createWidget` / `defineManifest` / `h`。用最小 fake DOM，无需真实浏览器。
- **widget 运行时测试**：`webui/tests/widget-runtime.test.mjs`，覆盖 registry / manifests / consent / sandbox-bridge / widget-host / slots。
- **engine 集成测试**：`engine/src/daemon/tests/extensions.rs`，覆盖 install / catalog / grant / intent / 静态服务 / token 续期。
- **host_api 校验测试**：`engine/src/extensions/mod.rs` 的 `install_validates_host_api_major` 与 `parse_host_api_major_handles_edge_cases`。

作者本地验证：

```bash
# webui 测试（含 SDK）
node --test webui/tests/*.test.mjs

# engine 扩展面测试
cargo test --lib extensions::
```

## 10. 安全边界回顾

- **agent 不得在运行时写 Vue / JavaScript / 任意前端代码**（[UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) 必须保留第 1 条）。widget 是已安装、已审查的模块，不是 agent 生成的代码。
- **widget 不得持 RP 数据真相源**，引擎拥有真相；widget 只渲染并发出 intent。
- **第三方 widget 接触敏感数据或触发特权动作前，必须有 engine 侧 capability 强制**（C-P3 已落地）。
- **运行时验证是功能的一部分**：digest-pinned 安装、sandbox iframe、capability 逐调用强制、catalog fail-closed（C-P4-1）。
- **SDK 不削弱任何安全边界**：它运行在沙箱 iframe 内，不提供任何"突破沙箱"或"绕过 capability"的路径。

## 11. 参考实现

| 参考 | 位置 |
|---|---|
| SDK 核心 | `webui/assets/widgets/sdk/widget-sdk.js` |
| SDK 示例 widget | `webui/assets/widgets/sdk/example-widget.js` |
| SDK manifest 脚手架 | `webui/assets/widgets/sdk/example-manifest.json` |
| 首方 builtin widget | `webui/assets/widgets/clock.module.js` |
| 首方 esm widget | `webui/assets/widgets/status.module.js` |
| 第三方示范 widget | `webui/assets/widgets/third-party-example.js` |
| 沙箱桥（宿主侧） | `webui/assets/widgets/sandbox-bridge.js` |
| 沙箱引导（iframe 侧） | `webui/assets/widgets/sandbox-frame.js` |
| widget 宿主 | `webui/assets/widgets/widget-host.js` |
| manifest 注册表 | `webui/assets/widgets/manifests.js` |
| widget 合同声明 | `webui/assets/widgets/widget-contract.js` |
| engine 扩展注册面 | `engine/src/extensions/mod.rs` |
| engine 扩展 API | `engine/src/extensions/api.rs` |
| intent 合同源 | `protocol/widget-intents.json` |
| catalog slot 计划（降级） | `webui/assets/widgets/slots.json` |
