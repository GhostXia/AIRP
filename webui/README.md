# AIRP WebUI

`webui/` 是 AIRP Engine 随包发布的无构建 Web 客户端。1–33 屏的视觉与页面信息架构以仓库内样板 `airp-engine-console/` 为准；34–44 屏是后续产品扩展，仍需按同一 styleguide 和视觉审查门校准。运行时代码在本目录独立维护。本页最后在 2026-08-09 的 `main@affa315`（`v0.0.5-rc.2` prerelease candidate）复核。

当前产品事实与发布边界：`v0.0.5-rc.2` 对应 Actions run `31309894372`，3 个 job 成功、5 个候选资产上传；这只证明候选包链路。正式 `v0.0.5` 仍被 [#130](https://github.com/GhostXia/AIRP/issues/130) 的真实 provider + 真实 browser + production Compose 验收，以及 `release` environment required reviewer 配置阻塞。

## 运行

由 Engine 托管：

```text
airp-core daemon --host 127.0.0.1 --port 8765 --webui-dir webui
```

> `--port 8765` 为显式传参；引擎编译期默认 `daemon_port` 即 8765（见 `engine/src/config.rs`），省略 `--port` 时行为一致。

打开 `http://127.0.0.1:8765/`。首次运行进入 `screens/16-onboarding.html`，完成或明确跳过后进入 `screens/01-role-list.html`。WebUI 默认同源调用 Engine；开发联调也可在 URL 上使用 `?engine=http://127.0.0.1:PORT`，该地址只保存在当前浏览器会话。

## 结构

- `index.html`：CSP 兼容入口。
- `assets/api-client.js`：JSON 请求与 SSE 客户端。
- `assets/role-list.js`、`assets/chat-space.js`：角色/会话核心流程。
- `assets/console-runtime.js`：工作台、世界书、Persona、Agent、设置、记忆、场景、分支、装配预览、配额和诊断的共享运行时。
- `screens/`：44 个页面入口；1–33 对应样板基线，34–44 覆盖关系图、剧情弧、图片/模板、创作工具、Provider 与插件管理。
- `tests/`：无构建 Node 测试与 CSP 静态门禁。

所有 HTML 必须遵守 Engine 的 `script-src 'self'`、`style-src 'self'`：禁止内联脚本、内联样式和内联事件处理器。没有后端契约的能力必须明确显示为不可用，不得提供假交互。
