# PR #38 审计报告

> **审计源 LLM**: `MiniMax-M3`（开发：MiniMax，2026 年初；本文档由其派生实例于 2026-07-05 生成）
> **审计对象**: PR #38 `feat(webui): expose provider models smoke`（分支 `codex/webui-p0-usability`，已合并入 `main`）
> **审计员立场**: 独立审计（参 [AGENTS.md §审计 agent 守则](../../AGENTS.md)）。不附和开发文档与既有代码的结论；以"我会不会这样写"为准。

---

## 0. 总结

PR #38 的明示范围是"暴露 `/v1/models` provider smoke 给 WebUI"，实际改动落在两个面：

1. **engine 侧**：[engine/src/daemon/handlers.rs](../../engine/src/daemon/handlers.rs) 新增 `list_models` 处理 + `MODELS_PROXY_TIMEOUT` 5s 上限 + `redact_endpoint_for_error` / `models_url_from_endpoint` 工具；[engine/tests/openai_compat.rs](../../engine/tests/openai_compat.rs) 新增 4 个 `models_proxy_*` 集成测试。
2. **WebUI 侧**：[webui/app.js](../../webui/app.js) 新增 `refreshModels`、`<section>Models</section>` 渲染、`buildChatPayload` 复用、并发流测试按钮；[docs/WEBUI-BACKEND-VALIDATION.md](../../docs/WEBUI-BACKEND-VALIDATION.md) 记录真实 provider 127.0.0.1:8889 + 模型 `gemini-3.1-pro-preview` 的 smoke 证据。

**总体判断**：PR 兑现了它的核心承诺——/v1/models 在 engine 端有 typed 错误、redact 凭据、显式 timeout；WebUI 能在 502/200 之间切换显示。但仍有 3 类问题超出文档限定的"可用性 P0"范围，被本次合并放过：

- **A 类（真 bug，需修）**：URL 派生函数对无 `/v1/` + 无路径的 endpoint 返回**错位 host**；WebUI 的 `doSend` SSE 累积三元表达式**退化为 noop**；import 错误显示路径**未走 `formatError`**。
- **B 类（设计/验证缺口）**：并发流测试的断言与注释**不一致**；engine 端没有覆盖 `models_url_from_endpoint` 边界用例的测试；MODELS_PROXY_TIMEOUT 写死 5s，不可配置。
- **C 类（harness 风险记号，文档未提）**：bearer token 长期驻内存、bearer 截断可能 panic、文件上传无客户端 size gate、自动 connect 300ms 延迟与用户编辑 URL 的竞态、agent run 的 `max_steps: 3` 硬编码、aborted 流会让最后一次 `done` SSE 帧丢失。

下文逐条展开。A 类给出最小修复 patch（仅描述，不直接落代码，留给 PR #39+）。

---

## 1. A 类：真 bug（应在合并前修）

### A1. `models_url_from_endpoint` 对"裸 host"endpoint 返回错位 host（中等严重度）

**位置**：[engine/src/daemon/handlers.rs:934-941](../../engine/src/daemon/handlers.rs#L934-L941)

```rust
fn models_url_from_endpoint(endpoint: &str) -> Option<String> {
    if let Some(pos) = endpoint.find("/v1/") {
        return Some(format!("{}/v1/models", &endpoint[..pos]));
    }
    let base = endpoint.trim_end_matches('/');
    base.rfind('/')
        .map(|pos| format!("{}/models", &base[..pos]))
}
```

**复现**（独立写了一个 Rust 片段验证，与引擎函数体一致）：

| 输入 | 当前输出 | 期望 |
|---|---|---|
| `https://api.openai.com` | `https://models`（host=`models`） | `https://api.openai.com/models` |
| `https://api.openai.com/` | `https://models`（host=`models`） | `https://api.openai.com/models` |
| `https://api.openai.com/v1` | `https://api.openai.com/models` | 一致 |
| `https://api.openai.com/v1/chat/completions` | `https://api.openai.com/v1/models` | 一致 |
| `http://host:port` | `http://models` | `http://host:port/models` |

`endpoint.find("/v1/")` 在裸 host（`https://api.openai.com`）上找不到，回落到 `base.rfind('/')`；该 `rfind` 命中 `https://` 中第二个 `/`（位置 7），`&base[..7]` 截出 `https:/`，再 append `/models` 拼出 `https://models`——新 host 字面意义是 `models`（顶级 TLD），不是原 `api.openai.com`。

**实际影响**：
- 用户在 settings.json 把 endpoint 误配成 `https://api.openai.com`（无尾斜杠无 `/v1/`）→ WebUI 点 reload → engine 实际 GET 的是 `https://models/v1/models`（如果 reqwest 把它当绝对 URL），**所有 provider 都连错 host**，但只看到一层 `502 invalid_endpoint` 链路被掩盖——因为 5s timeout 之后会先撞 `upstream_timeout`，诊断方向被误导。
- 同时也**绕过 typed error 路径**（不会触发 `invalid_endpoint` 502 + 详细 detail），用户看不到自己错配了。

**最小修复**（描述，不落代码）：用 `reqwest::Url::parse(endpoint).ok().and_then(|u| u.origin().ascii_serialization().into())` 拿 scheme+host+port，再 format `"{}/v1/models"`（含 `/v1/` 探测）或 `"{}/models"`（兜底）。或换 `url::Url` 库保持零依赖就够。

**测试缺口**（与 A1 同源）：[engine/tests/openai_compat.rs:217-247](../../engine/tests/openai_compat.rs#L217-L247) 三个 `test_models_proxy_invalid_endpoint_*` 都用 `setup_with_endpoint("not-a-url...")`，全部命中 `reqwest::Url::parse` 失败的分支——**从未覆盖 `models_url_from_endpoint` 的"能找到 host 但 host 是错的"分支**。这是 A1 漏到合并的主因。

### A2. WebUI `doSend` 的 SSE chunk 三元表达式退化为 noop（小严重度，但属于明显 dead code / 误改）

**位置**：[webui/app.js:275](../../webui/app.js#L275)（chat 路径）和 [webui/app.js:486](../../webui/app.js#L486)（`doSendText` 路径）

```js
const body = chunk.type === 'body_chunk' ? chunk.text : chunk.text;
```

两个分支**返回完全相同的 `chunk.text`**。意图几乎肯定是"区分 `body_chunk` 和 `think_chunk`/其他类型做不同渲染"（看 engine [xml_unpacker.rs:22-35](../../engine/src/xml_unpacker.rs#L22-L35) 实际有三类：`think_chunk` / `body_chunk` / `action_options`），但目前 `think_chunk` 会被无差别当 body 渲染——RP 场景下 think 块本应折叠或淡化显示，结果直接糊在角色台词里。

**最小修复**：`if (chunk.type === 'body_chunk') acc += chunk.text; else if (chunk.type === 'think_chunk') /* 走隐藏的 think 累积，不显示 */;`，并暴露一个可折叠的 think 块 DOM（harness 级别不必做交互，但至少要正确分流）。

### A3. WebUI import 失败时不走 `formatError`（小严重度，UX 直接降级）

**位置**：[webui/app.js:424-430](../../webui/app.js#L424-L430)

```js
const r = await api('POST', '/v1/characters/import', body);
if (r.ok) {
  importResult.textContent = '✓ 导入成功: ' + (r.data?.character_id || '?');
  refreshChars();
} else {
  importResult.textContent = '✗ ' + (r.status || 'err') + ': ' + (r.data || r.text);
}
```

而其他错误路径（`connect` / `refreshSettings` / `refreshModels` / `doSend` / `btnAgentRun`）都走 `formatError(r.data, r.text)`：能展开 `error.code` / `error.message` / `upstream_status` / `upstream_body` / `detail` 这五段。

这里 `(r.data || r.text)` 在 structured error 上会显示 `[object Object]`（因为 `r.data` 是 JSON 对象，`+` 字符串拼接会触发 `toString()`），把 engine 已经写好的中文诊断信息（"card_path 任意路径读已禁用（AIRP_ALLOW_LOCAL_PATH 未设）"等）整个丢掉。**审计 2026-07-04 RR-001 收口**定下的错误可读性，在这一行被自己废掉了。

**最小修复**：改成 `importResult.textContent = '✗ ' + (r.status || 'err') + ': ' + formatError(r.data, r.text);`。

### A4. `UserOrIpKeyExtractor` 的 bearer 截断可能 panic（极小概率但后果严重）

**位置**：[engine/src/daemon/mod.rs:148-152](../../engine/src/daemon/mod.rs#L148-L152)

```rust
let key = if token.len() > 32 {
    &token[..32]
} else {
    token
};
```

`token.len()` 是**字节**长度，`&token[..32]` 是字节切片。如果未来某次有人把非 ASCII token（比如带 emoji 或多字节字符的本地开发 token）放进 `access_api_key` 触发"超过 32 字节"分支，且 32 不在 char boundary 上，会**panic on `index 32 is not a char boundary`**。panic 抛在 tower_governor 限流 key 提取路径上 → 整个请求 500。

实际触发概率：低（access_api_key 几乎都是 base64/hex/ASCII）。但 `key_extractor` 是**每个请求都过**的热路径，任意一次 panic = 单次 5xx。成本极低的修复：`&token[..token.len().min(32)]` 改成在 char boundary 切，或更稳的 `let n = token.len().min(32); &token[..token.floor_char_boundary(n)]`（`floor_char_boundary` nightly）或干脆在切之前 `let safe_n = (0..=n).rev().find(|&i| token.is_char_boundary(i)).unwrap_or(0);`。

### A5. 修 A1 顺带：typed-error 测试覆盖空集

与 A1 同源，单列强调：[engine/tests/openai_compat.rs:217-247](../../engine/tests/openai_compat.rs#L217-L247) 三个 invalid_endpoint 测试**只覆盖 URL 完全无法 parse 的情况**。开发 agent 给出的"覆盖行为"清单（[WEBUI-BACKEND-VALIDATION.md:155-163](../../docs/WEBUI-BACKEND-VALIDATION.md#L155-L163)）说"invalid endpoint detail 会脱敏 URL query/userinfo；`not-a-url?api_key=secret` 只返回 `not-a-url?redacted`"——但这是 `redact_endpoint_for_error` 的脱敏行为，**与 `models_url_from_endpoint` 能否正确派生 URL 是两件事**。前者覆盖到了，后者完全没覆盖。

---

## 2. B 类：设计/验证缺口（应单独 PR 修，不要混进 P0 收尾）

### B1. 并发流测试的断言与注释不符

**位置**：[webui/app.js:450-463](../../webui/app.js#L450-L463)

注释说"启动两条并发 chat.send，验证 id-keyed chat state 不串扰（PR #6 修的 race）"。注释**继续**说：

> 期望序列：`u-A → a-A → u-B → a-B 应基本交替，无串扰`

但 `Promise.all([doSendText('A'), doSendText('B')])` 里两个 `doSendText` 内部都是同步 `appendMsg('user', text, false)` 然后 `await fetch(...)`——A 同步追加 u-A 后 `await fetch` 挂起，B 立即同步追加 u-B 后 `await fetch` 挂起。**DOM 上**已经稳定是 `u-A, u-B`（按调用顺序），与注释的 `u-A → u-B` 序列一致；但**注释里的 "u-A → a-A" 不对**：第一条流 SSE 第一个 chunk 落 DOM 时，第二条流的 `appendMsg('user', 'B')` 早就完成了，所以稳态序列是 `u-A, u-B, a-?, a-?`——而哪条 assistant 先回，取决于上游两条流哪个先回，与 id-keyed 串扰无关。

**实际测试断言**（[webui/app.js:460](../../webui/app.js#L460)）：
```js
const ok = results.every(r => r.ok);
```
只校验"两条都 `ok: true`"，**从不校验 chat log 顺序、不校验 message index 唯一、不校验 chat_store 持久化的 message 顺序**。

**影响**：本测试是 §11.1 标"PR #6 race regression test"——但实际只验证"两条流都能回来"，对"不串扰"没有断言。M_AGENT-1 骨架里如果 race 退化（比如未来某次重构把 chat state 改回全局），此测试**不会红**。这是 PR #6 修复的回归保护被悄悄废掉。

**修复方向**：测试结束 `await refreshSessions()` 后读 `/v1/chat/history`，断言 message 数 = 4，user/assistant 交替，user message 内容分别匹配 A 和 B。或者更稳：在 server side 注入 race（同时起 + 同时收），并断言落库 messages 数组的相对顺序与 SSE 接收顺序一致。**但这超出 P0 范围**，建议搬到 P2 #30。

### B2. MODELS_PROXY_TIMEOUT 写死 5s，不可配置

**位置**：[engine/src/daemon/handlers.rs:21](../../engine/src/daemon/handlers.rs#L21)

```rust
const MODELS_PROXY_TIMEOUT: Duration = Duration::from_secs(5);
```

[WEBUI-BACKEND-VALIDATION.md:177](../../docs/WEBUI-BACKEND-VALIDATION.md#L177) 真实验证里只到 5s 内完成的 provider：超时是"够用"但不是"普适"。开发者文档/issue 列表（[docs/WEBUI-BACKEND-PLAN.md §7.4](../../docs/WEBUI-BACKEND-PLAN.md)）当前不把"models proxy timeout 配置"列入 P0/P1 风险。但 audit 角度看：
- 5s 对一个**第一次**请求（provider 冷启）经常不够。WebUI 用户看到的是 `upstream_timeout` 错，但实际 provider 慢只是冷启。
- 5s 对**始终**慢的 provider 永远不够，错误信息会让用户误以为 key 错。

**建议**：要么改成配置项（`MutableConfig` 加 `models_proxy_timeout_secs`），要么干脆直接用现有 `reqwest::Client` 默认 timeout 的 30s（chat 路径已经是 30s 默认——见 [adapter.rs](../../engine/src/adapter.rs)）。**单独 PR 修，不混进 #38 后续**。

### B3. WebUI 的 `appendMsg` 不区分 role 与 sender

**位置**：[webui/app.js:227-236](../../webui/app.js#L227-L236)

`appendMsg` 接收 `role: 'user' | 'assistant'`，但 `user_profile.name`（[webui/app.js:299](../../webui/app.js#L299)）写死 `'User'`。多用户/多角色场景下用户侧永远显示 "User"，无法区分是哪个真人发的。

不是 P0 阻塞，但记号在此。

### B4. `formatError` 漏掉 `error.request_id` / `error.hint` / `error.suggestion` 等

**位置**：[webui/app.js:101-114](../../webui/app.js#L101-L114)

只展开 `code / message / upstream_status / upstream_body / detail` 五段。如果未来 engine 错误模型扩展（比如加 `request_id` 便于用户贴 issue），WebUI 自动丢失。建议改成 `formatError` 接受"已知字段白名单" + "其余字段折叠为 raw JSON 显示"。

---

## 3. C 类：harness 风险记号（不在 P0 范围，但合并前应该留 TODO 标）

| 编号 | 位置 | 风险 | 当前态 | 建议处理时机 |
|---|---|---|---|---|
| C1 | [webui/app.js:502](../../webui/app.js#L502) `setTimeout(connect, 300)` 自动连接 | 用户正在编辑 URL 时 300ms 触发的 connect 会读半截 URL，发请求到不存在的主机，污染 event log | 已知，未文档化 | P1 改"自动连接受用户操作抑制"或加 disable 开关 |
| C2 | [webui/app.js:36-39](../../webui/app.js#L36-L39) bearer token 存在闭包变量里 | XSS 注入可读（`sessionStorage` 计划里写过但未实现）；页面刷新即丢 | 接受（harness） | P1+#33 决策（文档原话"避免默认把新 key 持久写入明文"） |
| C3 | [webui/app.js:411-413](../../webui/app.js#L411-L413) import 直接 `file.arrayBuffer()` 再 base64 | 50MB PNG 读进内存 + base64 编码 + JSON 发送，浏览器可能 OOM；engine 端 10MB limit 拒载，用户只看到 413 | 接受（harness） | M3 之前临时在 WebUI 加客户端 size gate（>10MB 直接拒） |
| C4 | [webui/app.js:253-254](../../webui/app.js#L253-L254) `abortController` 全局共享 | `doSend` 与 `doSendText`（并发测试用）不共享 abortController，但 `doSend` 自身在 `if (abortController) abortController.abort();` 时**不会清空** `abortController`，下次 `new AbortController()` 之前引用还指向旧 AC | 实际无 bug（fetch 自身的 signal 引用独立） | 不必改 |
| C5 | [webui/app.js:375](../../webui/app.js#L375) agent run `max_steps: 3` 硬编码 | 用户无法在 UI 调；agent run 因此常在第 1 步就 finish | 已知 | P1 暴露 UI input |
| C6 | [webui/app.js:288-291](../../webui/app.js#L288-L291) `e.name === 'AbortError'` | 若上游在 `[DONE]` 之前发 abort 事件，streamSse 已退出 `while`，但 `logEvent` 仍记 `SSE done/N chunks`——与"aborted"语义冲突 | 实际无害（log 仅诊断用） | 不必改 |
| C7 | [webui/app.js:36-39](../../webui/app.js#L36-L39) bearer 长度截断 `&token[..32]` panic 风险 | 见 A4 | 同 A4 | 一起修 |

---

## 4. 我的判断（不附和文档）

参 [AGENTS.md §审计 agent 守则 §2](../../AGENTS.md) "可以提出自己的想法，不必拘泥于开发文档"——以下是脱离文档的独立看法：

### 4.1 文档"成就清单"里漏写的事实

[WEBUI-BACKEND-VALIDATION.md:148-163](../../docs/WEBUI-BACKEND-VALIDATION.md#L148-L163) 写：

> 本轮代码断言：
> - cargo test -p airp-core --test openai_compat models_proxy -- --nocapture：3 passed。
> - cargo test -p airp-core --test openai_compat -- --nocapture：9 passed。

但**这 3 个 models_proxy 测试用例不覆盖 `models_url_from_endpoint` 在"host-only endpoint"上的行为**。也就是说，文档给出的"覆盖行为"清单（"覆盖行为"段落里没有"裸 host endpoint"这一行）是诚实的——但读者容易误以为 "3 passed" 即"全场景已覆盖"。这是**测试覆盖与文档承诺之间的微妙错位**，应在 P0 收尾文档补一句"已知未覆盖：裸 host endpoint 派生边界"，让未来审计/开发者少走弯路。

### 4.2 PR 标题"feat(webui): expose provider models smoke"的命名误导

PR 标题与文档重心都把"暴露 /v1/models"作为**核心交付**。但实质上 engine 侧 + 测试侧占 PR 的 80% LOC（[engine/src/daemon/handlers.rs:849-1013](../../engine/src/daemon/handlers.rs#L849-L1013) ≈ 165 行 vs [webui/app.js](../../webui/app.js) 中 models 相关 16 行）。这是个**engine 后端硬化 PR**，副产物才是 WebUI 暴露。把标题叫 "feat(engine): harden /v1/models typed errors + webui smoke" 更贴切。

不改也行；但下次类似 PR 标题请贴近 LOC 重心，免得审计/复习时看错重心。

### 4.3 对"timeout = 5s"的反对意见

开发 agent 给的"5s 上限"看似合理，但 audit 我觉得**对 harness 而言太短**：

- 5s 是给"快 provider"的最佳体验阈值；不是给"首次冷启 provider"的容错阈值。
- WebUI 用户第一次 reload models 时，provider 极可能冷启（同步 token 校验、模型列表 lazy load），5s 撞穿是常态。
- 5s 撞穿后用户看到 `upstream_timeout` 但**不知道是冷启还是 key 错**——错误可读性目标没达到（[WEBUI-BACKEND-PLAN.md §3.1.5](../../docs/WEBUI-BACKEND-PLAN.md) 明确写"失败时知道该改 key、model、endpoint 还是 bearer"）。

我**不**主张 5s 改大。**我**主张：保留 5s 但同时给"第一次请求 + 已知 cold-start"加分段超时（例如首次 15s、后续 5s），或干脆把 5s 暴露在 `/v1/settings` 里让用户自己调。

### 4.4 `concurrent_status.textContent` 的"应基本交替"语义

见 B1。这是注释与代码不同步的小事，**但**因为这条注释在 [WEBUI-BACKEND-PLAN.md §3.1.3](../../docs/WEBUI-BACKEND-PLAN.md) 也有"两个并发 chat stream"的验收标准，我倾向把"基本交替"从验收标准降级为"无串扰 + 4 条消息都在 + 顺序确定"，**这样测试断言才写得出来**。

### 4.5 我会怎么写

如果我来做 #38：

- A1 修了再合并；测试 4 个变 6 个（加 `test_models_proxy_host_only_endpoint_returns_typed_error` / `test_models_proxy_host_only_endpoint_with_trailing_slash`）。
- A2 在 webui 加 `if (chunk.type === 'think_chunk')` 分支，至少把 think 块进 hidden span 不混入 body。
- A3 改一行就好。
- B1 把 `concurrent_status.textContent` 文案改成"两条流已完成。请打开 history 验证消息数=4 且 user/assistant 交替"，把断言写在 README 或 dev-only 注释里。
- B2 留 P1。
- C 全部留 P1+。

P0 的"可用闭环"目标**可以不变**——A1/A2/A3 都是低成本、零行为变化、零 API 变化的小修，**应当修完再合并**。这次合并不是"先把 P0 跑通"，是"把 P0 跑通 + 把小债一起承担"——区别在于下次回归成本。

---

## 5. 与既有约束的兼容性检查

| 约束 | 来源 | 本 PR 状态 |
|---|---|---|
| RR-001 card_path 任意路径读禁 | [docs/RISK-REGISTER.md:1-10](../../docs/RISK-REGISTER.md) | ✅ WebUI 走 `card_png_base64` / `card_json`，从未发 `card_path`；[webui/app.js:404-431](../../webui/app.js#L404-L431) 注释明确 |
| 神圣不变式 `subagent_context_has_no_orchestrator_noise` | [docs/PLAN.md:43](../../docs/PLAN.md) | ✅ 未触及 orchestrator / subagent；本 PR 只动 daemon handlers + tests + webui |
| 浏览器 WebUI 不走 card_path | [docs/WEBUI-BACKEND-PLAN.md §4.3](../../docs/WEBUI-BACKEND-PLAN.md) | ✅ 同上 |
| CORS Any / 鉴权默认关闭 | [docs/WEBUI-BACKEND-PLAN.md §2.2](../../docs/WEBUI-BACKEND-PLAN.md) | ✅ 未变 |
| Governor 覆盖 /v1/* | [engine/src/daemon/mod.rs:178-189](../../engine/src/daemon/mod.rs#L178-L189) | ✅ /v1/models 在覆盖范围内（line 231） |
| 5.0a 改用 `CharacterId` newtype | — | ✅ 不在本 PR 范围（未动 import） |

无兼容性冲突。

---

## 6. 给 PR #39+ 的建议清单（按优先级）

1. **A1 必修**：URL 派生用 `reqwest::Url::origin()` 重写；新增 2 个 boundary test。
2. **A3 必修**：1 行修改。
3. **A2 必修**：加 `think_chunk` 分流，至少保证不混入 body 渲染。
4. **A4 必修**：truncate 改 char-boundary-safe。
5. **B1 单独 PR**：把并发流测试的"无串扰"断言真正写出来。
6. **B2 单独 PR**：暴露 `models_proxy_timeout_secs` 配置（或改 30s）。
7. **C3 单独 PR**：WebUI 客户端 import size gate。
8. **C1 单独 PR**：auto-connect 用户操作抑制。

> 1-4 应在下一次 WebUI 收尾 PR 一起修（< 50 LOC 改动，零行为/契约影响）。
> 5-8 各自独立 PR，不要堆回 P0 收尾。

---

## 7. 审计元数据

- 审计源 LLM: `MiniMax-M3`
- 审计时间: 2026-07-05
- 审计员立场: 独立（不附开发文档与既有代码结论）
- 阅读材料: PR #38 全部 diff（已合并） + [docs/WEBUI-BACKEND-PLAN.md](../../docs/WEBUI-BACKEND-PLAN.md) + [docs/WEBUI-BACKEND-VALIDATION.md](../../docs/WEBUI-BACKEND-VALIDATION.md) + [engine/src/daemon/handlers.rs](../../engine/src/daemon/handlers.rs) + [engine/src/daemon/mod.rs](../../engine/src/daemon/mod.rs) + [engine/tests/openai_compat.rs](../../engine/tests/openai_compat.rs) + [engine/src/agent/mod.rs](../../engine/src/agent/mod.rs) + [webui/app.js](../../webui/app.js) + [webui/index.html](../../webui/index.html) + [webui/style.css](../../webui/style.css) + [AGENTS.md](../../AGENTS.md)
- 复现实验: Rust 离线片段（已在 [docs/audits/PR-38-audit.md](../../docs/audits/PR-38-audit.md) 附录 A 记录命令），验证 A1 的 host-only endpoint 边界行为
- 未跑 cargo test（沙箱无 D 盘、无 Rust 工具链；本仓规则禁止把工具链装 C 盘）
