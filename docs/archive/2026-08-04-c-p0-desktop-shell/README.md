# C-P0 桌面壳归档说明（2026-08-04）

本目录归档 C-P0「Tauri 壳承载 webui 与 bearer 注入通道」实施中被替换下线的桌面壳构件。
分支：`desktop/c-p0-tauri-webui-shell`。上位决策见 `docs/UI-PROTOCOL-DECISION.md`。

## 归档物

### `bus.rs`（原 `ui/src-tauri/src/bus.rs`）

旧壳的 intent relay 总线：在 Vue 面与 engine 之间转发 State Protocol intent、
维护双份会话/状态机投影。C-P0 将壳内容面切换为 engine 同源 webui 后，
壳不再持有 UI 状态，bus.rs 整体退役。随之消除的已知缺陷：

- **BUG-3**：intent relay 与 engine 侧会话状态机的双事实源漂移；
- **BUG-4**：bus 重连后 intent 重放导致的重复副作用；
- **BUG-5**：bus 背压缺失下的事件丢失。

行为基准（intent 语义、事件排序约束）已由 engine 侧 SSE 事件合同
（Task #4 产出）与 webui 消费端一致性测试承接，无需以运行代码形式保留。

## 未归档但已断开运行时加载的资产：Vue 主面

`ui/src/`（App.vue、widgets/、state/、protocol/、registry/）在 C-P0 后
**不再被壳加载**：`tauri.conf.json` 的 `frontendDist` 指向 `ui/splash/`
静态占位页，窗口启动后由 Rust 侧导航到 engine 同源 webui。

按 C-P0 计划，`registry/` 与 `protocol/` 的 TS 源码**保留为行为基准**：

- widget registry 的 slot/能力语义是 C-P1「widget 运行时移植进 webui」的
  移植规格来源；
- State Protocol 型别（protocol/）继续由 `protocol` crate 权威持有，
  TS 侧仅作对照基准；
- CI 的 `ui` job（typecheck + vitest）继续覆盖这些资产，防止基准腐化。

因此 Vue 主面不做物理归档；待 C-P1 移植完成后另行处置。

## 壳职责保留清单（C-P0 后由 `ui/src-tauri/src/main.rs` 承担）

1. sidecar（engine）生命周期：spawn / 就绪探针 / 退出 kill；
2. access key 管理：进程级随机生成，仅内存传递（不落盘、不入日志）；
3. bearer 注入通道：`POST /v1/desktop-session` 换短时效 token，
   经 URL fragment 注入首屏（详见 engine `daemon::desktop_session` 模块注释）；
4. 原生错误对话框（`tauri-plugin-dialog`，不依赖 webview 内容面）；
5. 打包 smoke：webui 经 `ui/bundle-webui.ps1` 暂存进
   `ui/src-tauri/webui-bundle/`，由 `bundle.resources` 打入安装包。
