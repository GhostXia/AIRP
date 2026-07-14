# 项目审计与实施历史（2026-07）

> 状态：历史摘要，不提供当前任务排序。
>
> 整理日期：2026-07-14
>
> 原始全文基线：`main@6736755`。

本页合并以下已被当前基线或 GitHub issues 取代的文档：

- `AUDIT-AND-ROADMAP-2026-07.md`
- `PROJECT-AUDIT-2026-07-10.md`
- `issues/issue-pr38-audit-findings.md`
- `superpowers/plans/2026-07-07-decompose-agent-flow.md`

## 演进摘要

1. 仓库从旧的多仓叙事收敛为 `engine + protocol + ui/src-tauri` workspace，并新增独立 `webui/`
   正式产品面；Gateway、MCP-Server、State-Protocol 与 AIRPCLI 按第一方资产来源处理。
2. engine 完成从单回合后端到有界 structured tool-call Agent loop、共享 domain service、持久化
   history/state/lorebook 和 production fail-closed 的演进。
3. WebUI 先作为后端验证面，随后接管当前产品交付主面；桌面 UI 代码保留，但近期开发暂停。
4. 早期 decompose/analysis 拆分计划已由 PR #91/#93 等实现；其旧 checkbox 不再表示待办。
5. PR #38 等审计的未修意见已迁移到 GitHub issue，Markdown 不再维护第二份实时状态。
6. 2026-07-13 完成 P0 同源 HTTPS 生产拓扑；正式 RP 使用面、数据恢复和发布候选门禁仍开放。

## 可复用结论

- 当前事实先看源码、manifest、测试和 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md)。
- 历史审计中的百分比、文件行号、PR 分支和任务排序均不可直接复用。
- 候选能力只有纵向贯通并通过对应验收后，才能从 issue/计划升级为已交付事实。
- “便于维护和未来移植”要求第三方能力模块化接入、边界可替换，并且不能形成平行真相源。

## 原始材料恢复

```powershell
git show 6736755:docs/PROJECT-AUDIT-2026-07-10.md
git show 6736755:docs/AUDIT-AND-ROADMAP-2026-07.md
git show 6736755:docs/superpowers/plans/2026-07-07-decompose-agent-flow.md
```
