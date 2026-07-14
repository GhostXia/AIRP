# PR 审计归档（2026-07）

> 状态：历史索引，不是当前开发基线。
>
> 整理日期：2026-07-14
>
> 原始全文基线：`main@6736755`。需要复核原文时使用
> `git show 6736755:docs/audits/<原文件名>`。

本页把 2026-07 的逐 PR 审计压缩为一份可检索索引。当前能力、风险和任务顺序
以 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md)、GitHub issues、源码和测试为准。
审计中未随 PR 修复的意见已经按仓库流程进入 GitHub issue；本页不再复制实时待办。

| PR / 原文件 | 审计主题 | 保留结论 |
|---|---|---|
| #38 `PR-38-audit.md` | models proxy 与 WebUI 错误面 | 暴露了裸 host、SSE 处理、错误展示和并发证据缺口；后续由修复与 issue 接管。 |
| #59 `PR-59-audit.md` | 真实 provider / SSE 证据 | 关键教训是不得把期望事件序列写成实测；缺失证据与缺失覆盖必须分开。 |
| #60 `PR-60-audit.md` | WebUI M1 usability | 审计发现并修复进程级 DoS、HTML 与交互问题；结论只适用于当时分支。 |
| #62 `PR-62-audit.md` | WebUI 工作台 | 复核角色卡/世界书编辑、dirty state 和拖拽生命周期。 |
| #63 `PR-63-audit.md` | WebUI M2 polish | 修复异步竞态、样式回归和注释问题后通过。 |
| #65 `PR-65-audit.md` | import safety | HTTP 回归、multipart 边界和审计产物独立性。 |
| #66 `PR-66-audit.md` | 历史草稿与本地产物清理 | 删除过时审计草稿合理；本地 `target/` 不属于仓库事实。 |
| #72 `PR-72-audit.md` | WebUI usability polish | 评估输入、交互与错误面；非阻塞项后续进入 issue。 |
| #74 `PR-74-audit.md`、`PR-74-audit-independent.md` | issue cleanup | 同一 PR 的开发者自审与独立审计并存；独立审计不继承自审结论。 |
| #75 `PR-75-audit.md`、`PR-75-audit-v2.md` | 消息级时间戳 | 验证消息、ID、时间戳等长与 legacy 兼容；二审识别 dead fallback。 |
| #76 `PR-76-audit.md` | #75 follow-up | 复核 dead code、等长不变式和 WebUI 可观察性修正。 |
| #77 `PR-77-audit.md` | `/health` | 就绪探针、可写性检查和锁释放通过。 |
| #78 `PR-78-audit.md` | Tauri settings | 复核 settings IPC、无谓请求与错误处理。桌面路线现已暂停。 |
| #80 `PR-80-audit.md` | Tauri history | 复核 history intent 和潜在流式竞态。桌面路线现已暂停。 |
| #82 `PR-82-audit.md` | 审计遗留小修 | 文档、watch 行为和交互注释的低风险收口。 |
| #83 `PR-83-audit.md` | 时间戳传递 | Tauri wire/UI 时间戳增量改动通过。 |
| #84 `PR84-AUDIT.md`、`WEBUI-REDESIGN-BACKEND-REQUIREMENTS-audit.md` | 后端需求文档纠错 | 通过真实路由/浏览器探针纠正不存在的 P0 缺口和请求形状误判。 |
| #85 `PR-85-secondary-audit.md` | session API / access key view | 复核 A5/A6 修复、导入错误和多 session 行为。 |
| #88 `PR-88-audit.md` | WebUI V2 设计 | 设计稿落在非运行目录，不能视为产品交付；后续由 #105/#106 与正式 WebUI 接管。 |
| #123 `PR-123-audit.md` | WebUI 基础验收 | 修复 mock provider SSE 结束条件与 rate-limit 误读后，基础闭环可合并。 |

## 归档原则

- 此索引保留审计对象、主题和可复用教训，不保留已失效 line number。
- 原报告中的严重度和“建议合并”只描述当时提交，不能覆盖当前源码。
- 未修意见以 GitHub issue 为唯一实时追踪面；关闭 issue 后不回写历史审计。
- 新审计不再默认新增一份永久 Markdown。PR 内 review 留在 GitHub；只有跨 PR、可复用的
  架构结论才进入活文档，批量历史再定期并入本归档。
