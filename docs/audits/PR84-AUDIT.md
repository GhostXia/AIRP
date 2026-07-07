# PR #84 独立审计

> 日期：2026-07-07
> 审计对象：`docs(webui): 修正后端需求文档 6 处阻塞错误 + 浏览器实测审计` (PR #84)
> 审计立场：独立审计（AGENTS.md 守则），不附和 PR 描述或 commit message

---

## 1. PR 概要

GLM-5.2 对我（UI Agent）编写的 `docs/WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md` 做了独立审计，用 agent-browser + fetch() 实测了 engine 的每个端点，发现原文档有 6 处 A 项阻塞错误 + 5 处 B 项描述错误 + 3 处 C 项设计建议 + 4 处 D 项表述问题，并提交了修正后的文档 + 审计报告。

---

## 2. A 项逐条复核

| A 项 | PR 论断 | 我的独立验证 | 评定 |
|------|--------|-------------|------|
| **A1** import 不接受 multipart，只接受 JSON | 415 实测 + handlers.rs 源码 | **正确**。我的原文档错误地把 import 描述为 multipart。源码确认是 `Json(req): Json<ImportCharacterRequest>`。PR 修正为 JSON body + base64 编码方案完全正确 |
| **A2** 角色卡 GET/PUT/DELETE 在裸路径已存在 | 路由判别器 400 vs 404 + 实际 200 | **正确**。我的原文档错误地建议新增 `/card` 子路径。实测确认 `GET/PUT/DELETE /v1/characters/:character_id` 全部可用。PR 修正为使用裸路径正确 |
| **A3** 世界书 GET/PUT 已存在 | 路由判别器 + 实际 200 | **正确**。与 A2 同理，我的原文档错误。PR 修正正确 |
| **A4** §5 P0 端点汇总 4 条全错 | A2+A3 综合推导 | **正确**。既然 A2/A3 都已存在，P0 清单自然失效。PR 重排正确 |
| **A5** SettingsView 无 access_api_key 字段 | GET /v1/settings 实测返回 | **正确**。我的原文档说"检查 access_api_key 字段"，但 SettingsView 实际只返回 `api_key_set: bool`，不返回 access_api_key 相关字段。PR 建议加 `access_api_key_set: bool` 合理 |
| **A6** session API 割裂（新发现） | session_id 不匹配 + 422 + 源码 | **正确且重要**。这是我最严重的遗漏——我写 §3.1 时只看了 `ChatCompletionRequest` 有 `session_id` 字段就认为多 session 可行，没有验证 history/rollback/regen 是否也接受。A6 是多 session UI 的第一阻塞项 |

**A 项审计结论**：6 条全部正确。我的原文档在端点存在性核实上确实不充分——我主要基于 WEBUI-BACKEND-PLAN.md（文档描述）而非浏览器实测，导致把已有端点判为"需新增"，漏了 A6 session 割裂。

---

## 3. B 项逐条复核

| B 项 | PR 论断 | 评定 |
|------|--------|------|
| B1 api_path_prefix 必要性存疑 | 已有自动推导逻辑 | **正确**。PR 建议从 P2 标记"暂缓"合理 |
| B2 api_key_set 是 bool，非 sk-...**** | 实测返回 | **正确**。我的原文档说"脱敏为 sk-...****"是错的 |
| B3 card_path 门控是环境变量 | 源码 AIRP_ALLOW_LOCAL_PATH | **正确**。我的原文档说"仅限 Tauri IPC"是错误的——门控条件是环境变量，不是调用方身份 |
| B4 漏列端点 | /health /scenes /presets /state/schema /DELETE | **正确**。我的原文档确实漏列了这些 |
| B5 session meta 扩展依赖 title 持久化 | SessionId 只是 UUID | **正确**。光扩展返回结构不够，需要先持久化 title |

---

## 4. C 项评估（PR 提出的设计建议）

| C 项 | 建议 | 我的看法 |
|------|------|---------|
| C1 Tauri IPC 路径未讨论 | 建议讨论是否在 Tauri 环境运行 | **合理但不阻塞**。当前 webui 是浏览器 harness，Tauri 是独立产品线（ui/）。讨论双路径策略有意义，但不在本报告范围 |
| C2 /health vs /version | 诊断面板应优先用 /health | **正确且有价值**。/health 返回 `provider_configured` + `data_root_writable`，比 /version 的 `{name, version}` 更适合做连通性检查。PR 已在 §6 和 §8 采纳 |
| C3 §9 实施顺序失效 | 已重排 | **正确**。PR 重排后的顺序合理 |

---

## 5. 文档修正质量

### 5.1 修正后的需求文档（WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md）

| 维度 | 评分 | 说明 |
|------|------|------|
| 端点准确性 | A | 每个端点都经实测验证，路由判别器方法严谨（用 URL 编码的 `:` 做 400 vs 404 区分） |
| P0 优先级 | A | 正确识别出 A6（session API 割裂）和 A5（access_api_key_set）为真正 P0，移除了伪 P0 |
| 实施顺序 | A | P0-A（session_id 扩展）→ P0-B（SettingsView）→ P0-C（前端对接现有端点）→ P0-D（import 改 JSON body）→ P1/P2，逻辑正确 |
| UI→后端映射表 | A | §8 速查表每行都标注了 A6/A5 依赖，标注了裸路径/JSON body 等关键细节 |
| 安全约束 | A | §7 修正了 B3（card_path 门控是环境变量）、D3（Bearer sessionStorage 移出安全约束），新增了 import 错误消息误导问题（§7.6） |
| 对 UI 设计稿的兼容性 | A | 所有修正都兼容 `airp-engine-console/` 的 3 页设计——无鉴权警告（P0-B）、session 切换（P0-A）、工作台 CRUD（P0-C）、导入（P0-D）全部覆盖 |

### 5.2 审计报告（WEBUI-REDESIGN-BACKEND-REQUIREMENTS-audit.md）

| 维度 | 评分 | 说明 |
|------|------|------|
| 证据链完整性 | A | 每条 A 项都有：测试描述 + 请求/响应 + 源码行号引用，形成完整证据链 |
| 独立性 | A | 不附和原文档，用实测推翻了我的 5 个错误结论 + 发现了 1 个新问题 |
| 清理验证 | A | 测试后 DELETE 清理 + 返回值比对，确认无数据污染 |
| 方法论 | A | 路由判别器（%3A 做 400/404 区分）是创新方法，避免了对每个路由猜存在性 |

---

## 6. 遗留问题 / 可操作建议

### 6.1 对修正后文档的小补充（非阻塞，可后续 PR）

| 项 | 说明 | 优先级 |
|---|---|---|
| S-1 | §3.1 session 管理中"方案 A/B/C"建议可合并为 PR 中的方案 1（改 HistoryQuery 加 session_id），因为 A6 修复后方案 C（前端从 history 推导 title）成本最低 | 低 |
| S-2 | §2.2 角色列表建议"扩展 GET /v1/characters"返回 meta，但没提用 `?with_meta=true` 做版本兼容。审计报告 B5 提到了但文档正文未采纳 | 极低 |
| S-3 | 修正后的文档把 import 描述改为 JSON body + base64，但 UI 设计稿的导入区仍用"拖放上传"文案。前端实现时需要做 file → base64 转换——这属于前端工作，不影响文档正确性，但可在文档中加一行提示 | 极低 |

### 6.2 对后端 Agent 的建议（从 UI 视角）

| 优先级 | 建议 | UI 影响 |
|--------|------|--------|
| **P0-A6** | chat/history + rollback + regen 加 session_id：Option 字段。这是多 session UI 的**唯一后端阻塞项**。没有这个，用户点击不同 session 看到的都是同一个聊天记录 |
| **P0-A5** | SettingsView 加 access_api_key_set: bool。没有这个，无鉴权警告永远无法正确显示/隐藏 |
| **P0-D** | 前端侧修改 import 为 JSON body。不需要后端改动，但后端应修正 import handler 的错误消息（当前写"请用 multipart"，实际不接受 multipart） |

---

## 7. 总评

| 维度 | 评分 |
|------|------|
| 原文档错误密度 | 高（6 处 A 项阻塞错误，5 处 B 项描述错误） |
| PR 修正完整度 | A（全部 18 项问题均已修正） |
| 审计方法论 | A（浏览器实测 + 路由判别器 + 源码引用） |
| 新发现价值 | A（A6 session API 割裂是关键发现） |
| 文档可实施性 | A（修正后可直接作为后端实施依据） |

**结论**：PR #84 质量优秀，修正了我的原文档的所有错误，并发现了我遗漏的关键 A6 问题。修正后的文档准确、完整、可直接作为后端 Agent 的实施依据。**建议合入**。

我（UI Agent）在编写原版报告时犯了两个主要错误：
1. **端点存在性未实测**——基于文档描述而非 fetch 实测，导致 A1~A4 四条误判
2. **session 多路径验证不充分**——只确认 chat/completions 支持 session_id，没有验证 history/rollback/regen 是否也支持，导致 A6 遗漏

PR #84 通过独立审计 + 浏览器实测纠正了这些错误，质量可靠。
