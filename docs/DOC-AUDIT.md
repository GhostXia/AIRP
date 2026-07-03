# 文档审计待确认项

> 最后更新：2026-07-03
> 目的：把文档整理中发现的矛盾、疑问和需要用户拍板的地方集中列出。事实性过期描述已直接修正；这里保留不能替用户决定的事项。

## 已按当前事实整理

- 根 README：更新为 `engine + protocol + ui` 两盒结构，并说明 `gateway` / `mcp-server` 已退回独立零件来源。
- `ui/README.md`：更新为 engine 客户端，不再把 Gateway/MockBus 当默认后端。
- `engine/README.md`：更新测试基线、无项目级 CI、Docker/脚本缺失等当前事实。
- `docs/DEV-GUIDE.md`：同步 PR #1-#4 状态、D 盘工具链和 npm cache 约束。
- `docs/PLAN.md` / `docs/PARTS.md`：修正四仓 workspace、mock BusRelay、CI 强制等 2026-07-01 旧状态。
- `AGENTS.md`：补充 npm cache 必须显式指向 `D:\npm-global\npm-cache`。
- 补充历史验证事实：AIRP-State-Protocol 原项目打包后的 exe 曾可正常启动并做简单交互，但未进一步深测；这不等于当前 AIRP-Dev 与 engine 集成后的完整 GUI 验收。

## 需要用户审核 / 拍板

1. **Task 1.1 状态怎么写**
   - 已确认：PR #3 已实现 path-first 角色卡导入 UI，PR #4 已加固派生 ID。
   - 未确认：本轮没有做 GUI 端到端手动验收。
   - 建议写法：`已实现，待运行时验收`。若你认可，也可以改成 `Done`，把验收作为单独 QA 项。

2. **下一步优先级**
   - 路线 A：先补 Task 1.1 GUI/runtime 验收和 Perf Spike，再继续功能。
   - 路线 B：先做 Task 1.2 chat 消息 id-keyed 寻址，顺带解决 `chat_lock` 债务。
   - 我倾向 A 后 B，原因是导入链路已经合并，先把验收闭环补上更透明。

3. **“CI”措辞**
   - 当前仓库没有项目级 `.github` CI；门禁事实是本地测试 + 人工 review。
   - 我已把“CI 强制”改成“本地/未来 CI 强制”或“本地门禁”。
   - 需要你确认：是否所有文档都统一避免单独写“CI 绿”，只写“本地测试全绿 / 未来 CI”。

4. **`card_path` 安全边界**
   - 当前 path-first 导入假设 UI 与 engine 是可信本地 sidecar 组合。
   - 若 engine 暴露到非本机或多用户环境，`card_path` 需要更强的约束：localhost 绑定、access key、文件选择 token、允许目录白名单，或改成受控上传/复制。
   - 需要你确认：这是马上进入 Phase 1 安全项，还是等 engine 对外暴露前再做。

5. **`data/` 中跟踪文件的定位**
   - `data/items.md`、`data/world.md`、`data/styles/profiles/default.md` 是已跟踪文件。
   - 需要明确它们是示例 seed、默认模板、还是运行时数据。若是运行时数据，未来多人开发会产生噪声；若是 seed，应在 README/DEV-GUIDE 里明确不可直接写用户私有数据。

6. **旧四仓历史要保留到什么程度**
   - 当前 README 只简述 `gateway` / `mcp-server` 是零件来源，详细历史留在 `docs/PLAN.md` / `docs/PARTS.md`。
   - 需要你确认：根 README 是否应该更彻底，只面向当前 AIRP-Dev，不再提旧四仓细节。

7. **`docs/PLAN.md` 的体量**
   - `PLAN.md` 仍是长文，包含大量 2026-07-01 审计记录。
   - 建议后续拆成：`VISION.md`（长期定位）、`ROADMAP.md`（当前阶段）、`DECISIONS.md`（已拍板）、`DOC-AUDIT.md`（待拍板）。
   - 这属于结构调整，不应在未确认前直接大搬。
