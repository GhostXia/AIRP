# AIRP WebUI

`webui/` 是当前 AIRP 产品交付主面：零构建、浏览器可运行的 RP 客户端。它已经具备基本日用闭环，但仍是 preview；正式发布前的 P1–P3 门禁见 [WebUI 正式上线计划](../docs/WEBUI-PRODUCTION-PLAN.md)。当前事实见 [开发基线](../docs/CURRENT-BASELINE.md)，P0 拓扑见 [production architecture](../docs/WEBUI-PRODUCTION-ARCHITECTURE.md)。

> `start.bat`、`serve.js`、`cargo run`、手填 engine URL 和可选 bearer 都是开发路径。不要把 8000/9001 端口或静态开发服务器直接暴露到公网。首方 P0 preview 位于 [deploy/production](../deploy/production/README.md)。

## 本地启动

### Windows 一键开发环境

双击 `webui/start.bat`。脚本会使用当前 `PATH` 中的 Rust/Node 工具链，并在独立窗口启动：

1. 零密钥 mock provider；
2. `cargo run -p airp-core -- daemon --port 8000`；
3. `node webui/serve.js`；
4. 浏览器 `http://127.0.0.1:9001`。

关闭三个服务窗口即可停止。脚本会清理可丢弃的 `target/webui-smoke-data`，不会使用仓库内真实玩家数据。

### 手动启动

终端一：

```powershell
cargo run -p airp-core -- daemon --port 8000
```

终端二：

```powershell
node webui/serve.js
```

打开 `http://127.0.0.1:9001`，连接默认 engine `http://127.0.0.1:8000`。需要本地 bearer 时通过 `AIRP_ACCESS_KEY` 启动 engine，并在当前标签页连接配置中输入；provider secret 不写入 `localStorage`。

跨设备或公网访问必须使用 `deploy/production/` 的同源 HTTPS、私有 engine 和 perimeter auth，不得通过把开发 server/engine 改成 `0.0.0.0` 代替生产部署。

## 页面结构

- **角色列表**：选择和导入角色；
- **对话空间**：session、历史、流式聊天、Agent Run 和诊断；
- **工作台**：角色卡、worldbook 与 decompose 工具，不销毁当前会话上下文。

`airp-engine-console/` 是设计来源，不是运行中的 WebUI。

## 当前能力

### 连接与配置

- `/version`、`/health`、`/v1/settings`、`/v1/models`；
- provider 设置和真实 model validation；
- 一键诊断只检查后端可达性，不消耗 provider 对话配额；
- development engine URL/bearer 只保存在 `sessionStorage`；production 浏览器不接收 engine 私有 URL/bearer。

### 角色与 RP 配置

- 角色列表、avatar、JSON/PNG 内容导入；WebUI 永不发送 `card_path`；
- Preset 选择与 JSON 导入；
- 多 Persona 列表、创建、编辑、删除和本地选择状态；
- Persona 绑定/解绑、聊天请求显式 `persona_id` 切换、完整 Preset 生命周期与有效配置摘要仍待 P1 完成；
- state live/history、worldbook 与 decompose 工作台。

### Session、聊天与 Agent

- 命名 session 创建、选择、删除；切换时清空视图并取消旧 SSE，避免跨 session 回写；
- `/v1/chat/completions` 增量 SSE、停止、regen、rollback；
- durable message ID、50 条首屏窗口、cursor 加载更早、稳定 DOM 复用和 prepend 滚动保持；
- `/v1/agent/run` 的 PLAN/TOOL_CALL/TOOL_RESULT/DELTA/DONE 事件日志；
- `/v1/agent/tools` 运行时 catalog 驱动 allow/destructive-confirm 选择；
- Markdown 先转义再渲染有限语法，用户文本不直接注入 HTML。

## 明确未完成

- P1 的 Persona/Preset/Worldbook 完整产品管理和有效配置；
- 自包含 session revision、migration、备份/恢复、可恢复删除与运维 runbook；
- branch/swipe/edit 的首发取舍；
- 浏览器矩阵、移动端收口、长会话 soak、SBOM/notices、升级和回滚演练；
- plugin/skills/MCP upstream、ChangeInbox、可配置多 Agent；
- Tauri 桌面 UI 变更。

本页不复制完整实时任务列表；开始开发前查询 GitHub issues 和 [当前基线](../docs/CURRENT-BASELINE.md)。

## 验证

### Engine-truth smoke

启动 mock provider 与 engine 后运行：

```powershell
node webui/smoke.mjs
```

该脚本通过 HTTP/SSE 验证持久化 history、Persona/Preset/session ID、三轮流式响应、隔离、rollback/regen/delete 和 typed errors；它不是浏览器自动化。

### UI 与浏览器

```powershell
cd ui
npm ci
npm run typecheck
npm run test -- --run
```

真实浏览器另行验证连接、恢复、交互、渲染、注入安全、SSE 取消和窗口 prepend/滚动。production topology gate 还验证真实 HTTPS、auth、私有 engine、CSP/headers、重启持久化和 secret scan。

每次验证记录 commit、启动命令、URL、provider/model（密钥脱敏）、请求边界、状态/延迟、SSE 事件、数据根和失败证据。历史结果只保留在 [WebUI 历史归档](../docs/archive/WEBUI-HISTORY-2026-07.md)，不要把旧数字当成当前证明。
