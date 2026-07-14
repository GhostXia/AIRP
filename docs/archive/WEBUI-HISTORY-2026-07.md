# WebUI 开发历史归档（2026-07）

> 状态：历史实施与验证摘要，不是当前执行入口。
>
> 整理日期：2026-07-14
>
> 原始全文基线：`main@6736755`。可用
> `git show 6736755:<原路径>` 恢复任一原文。

当前 WebUI 产品边界见 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md)，正式上线门禁见
[WEBUI-PRODUCTION-PLAN.md](../WEBUI-PRODUCTION-PLAN.md)。本页合并以下已完成或已被取代的材料：

- `WEBUI-ANALYSIS-AND-OPTIMIZATION.md`
- `WEBUI-AUDIT-v2.md`
- `WEBUI-BACKEND-PLAN.md`
- `WEBUI-BACKEND-VALIDATION.md`
- `WEBUI-MVP-PLAN.md`
- `WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md`
- `smoke-evidence/README.md`

## 阶段摘要

### 1. 后端验证面

早期 WebUI 用来证明 engine 可作为无头 RP 后端工作：配置、模型、角色、会话、聊天 SSE、
Agent Run、history/rollback/regen 和失败路径逐步接通。PR #100 保留一次真实 provider 流式
证据，但该证据不代表所有 provider、timeout 或错误类型都被覆盖。

### 2. 可运行 V2 与工作台

PR #88 的静态设计稿最初落在非运行目录，不能算交付。PR #106 以后运行态迁入 `webui/`，
角色/会话视图、工作台、导入和诊断才成为真实产品路径。此阶段的路由缺口报告多次被源码
和浏览器探针纠正，形成一条长期规则：路由存在性、请求形状和错误语义必须从运行时验证，
不能从旧文档推断。

### 3. 基础 RP 闭环

PR #118/#119/#121 接通基础 Persona、Preset、session 生命周期、恢复和 busy-state；PR #123
完成零密钥 mock-provider 基础验收。PR #124/#125 加入 durable message ID、cursor history、
rollback-by-ID、50 条首屏窗口、加载更早和稳定 DOM 复用。该阶段的完成合同已经被当前基线
吸收，不再保留独立 MVP 计划。

### 4. 生产拓扑

PR #132–#136 建立 engine production fail-closed、首方 OCI/Compose/Caddy artifact 和真实 HTTPS
topology smoke。WebUI 从“临时验证面”转为当前正式产品交付主面；Tauri/Vue 资产保留，但桌面
开发与打包验收暂停。P0 完成不等于正式发布，P1–P3 仍由生产计划约束。

## 仍有效的验证纪律

- engine-truth smoke 只证明其明确断言，不等同于真实浏览器、真实 provider 或发布验收。
- 真实浏览器证据必须说明浏览器、页面入口、交互和持久化结果。
- 失败覆盖要区分 connection refused、timeout、provider 非 2xx、鉴权失败与 client abort。
- 测试数字是 dated snapshot；修改后应重新运行，而不是沿用旧报告数字。
- WebUI 新能力应纵向贯通 shared service、HTTP/SSE、WebUI 与 production tests。

## 原始材料恢复

例如：

```powershell
git show 6736755:docs/WEBUI-MVP-PLAN.md
git show 6736755:docs/WEBUI-BACKEND-VALIDATION.md
git show 6736755:docs/smoke-evidence/README.md
```
