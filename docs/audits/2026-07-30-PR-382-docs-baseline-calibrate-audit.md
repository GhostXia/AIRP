# PR #382 独立审计（docs baseline calibrate）

> 审计日期：2026-07-30  
> 审计对象：`codex/docs-baseline-calibrate-2026-07-30` @ 修复后 tip（含 banner 修正 commit）  
> 基线对照：`origin/main@4f3f792`  
> 性质：docs-only；独立复核，不附和开发过程叙述  

## 1. 范围与方法

- `git diff origin/main...HEAD` 全文件列表（30 paths）  
- 自动化：损坏标记扫描、相对 `.md` 链接解析、归档路径存在性、RR-014 结构、基线测试措辞、BOM  
- 人工：关键 diff 抽查（LONG-HISTORY / WORLDBOOK / 研究文档头 / RISK / SECURITY / PLAN / DEV-GUIDE / README / CONVERSATION / archive）  

## 2. 阻塞结论

**首轮发现 1 个应修项，已在同 PR 修复：**

| ID | 严重度 | 问题 | 状态 |
|---|---|---|---|
| B1 | Medium（文档正确性） | `docs/archive/2026-07-persona-http-api-plan.md` 归档横幅写「原活文档 `docs/2026-07-persona-http-api-plan.md`」，实际原路径为 `docs/PERSONA-HTTP-API-PLAN.md`（批量 relink 残留） | **已修** |

修复后复扫：**无剩余 blocking 项**。

## 3. 通过项

- 无 `fiead` / `0006-` / `main@000` / `后后校准` 等编码或错误替换损坏  
- `LONG-HISTORY-CONTRACT` 仅校准头一行；PR 号 #124/#125/#122 完整  
- 研究文档保留研究日期，仅更新「状态复核」锚点  
- Persona 计划已 rename 到 archive；活路径 `docs/PERSONA-HTTP-API-PLAN.md` 不存在  
- 桌面草案 + 计划审计已入库；onboarding archive 引用已改到 archive 路径  
- 相对 markdown 链接可解析到存在文件  
- RR-014 仅一条 Current control / Residual；含 fail-open 与 #381 E-P0-3  
- 基线 §6 诚实写明 Rust workspace 本 pass 未干净复跑，且禁止外推 1199  
- PLAN §2.1/§2.2 结构正确  
- docs 地图归档表原路径正确  

## 4. 非阻塞意见（不阻断合并）

| ID | 说明 |
|---|---|
| N1 | 多个研究/合同文件仅多了文末空行（无语义影响） |
| N2 | 研究文档「状态复核：2026-07-30」主要是锚点对齐，并非对本轮重新精读全文的证明；可接受，因正文已声明以 CURRENT-BASELINE 为准 |
| N3 | `SECURITY.md` 插件段仍写 DNS failure allowed（与代码一致），另增 residual 小节指向 #381；略有分层重复，但不矛盾 |
| N4 | 本 PR 不关闭 #381；合并后风险仍开放——文档已正确表述 |

## 5. 裁决

**通过（在 B1 已合入后）**。docs-only，无 runtime 回归面。建议人工快速确认基线 §2.1/§5/§6 与 archive 横幅后合并。

## 6. 验证命令（本审计）

```text
node .tmp/audit-pr382-local.mjs   # issues=[], warnings=[]
git diff origin/main...HEAD --stat
```
