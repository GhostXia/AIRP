# AIRP WebUI

`webui/` 是 AIRP Engine 随包发布的无构建 Web 客户端。视觉与页面信息架构以仓库内只读样板 `airp-engine-console/` 为准；运行时代码在本目录独立维护。

## 运行

由 Engine 托管：

```text
airp-core daemon --host 127.0.0.1 --port 8765 --webui-dir webui
```

打开 `http://127.0.0.1:8765/`。首次运行进入 `screens/16-onboarding.html`，完成或明确跳过后进入 `screens/01-role-list.html`。WebUI 默认同源调用 Engine；开发联调也可在 URL 上使用 `?engine=http://127.0.0.1:PORT`，该地址只保存在当前浏览器会话。

## 结构

- `index.html`：CSP 兼容入口。
- `assets/api-client.js`：JSON 请求与 SSE 客户端。
- `assets/role-list.js`、`assets/chat-space.js`：角色/会话核心流程。
- `assets/console-runtime.js`：工作台、世界书、Persona、Agent、设置、记忆、场景、分支、装配预览、配额和诊断的共享运行时。
- `screens/`：31 个权威样板入口；子页面和状态页会合并到对应实际工作流。
- `tests/`：无构建 Node 测试与 CSP 静态门禁。

所有 HTML 必须遵守 Engine 的 `script-src 'self'`、`style-src 'self'`：禁止内联脚本、内联样式和内联事件处理器。没有后端契约的能力必须明确显示为不可用，不得提供假交互。
