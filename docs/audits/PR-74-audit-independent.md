# PR #74 独立审计报告

- **审计源 LLM**: GLM-5.2
- **审计日期**: 2026-07-06
- **审计范围**: PR #74 issue cleanup batch 1
- **审计基线**: commit a32a714 (main)
- **审计分支**: webui-pr-h-issue-cleanup
- **审计模式**: 独立审计（不沿用 PR 内自审结论）

## 审计守则

按 AGENTS.md "审计 agent 守则" 三条执行：
1. **独立审计** — 不附和 PR 内 self-audit 结论
2. **提出自己的想法** — 见下文 W-05 / A-1 / A-2
3. **质疑历史决策** — 通过实际跑 engine 验证自审未触及的假设

## 测试

```
cargo test --manifest-path engine/Cargo.toml: 358 pass / 0 fail
node --check webui/app.js webui/serve.js: 通过
node target/test-serve-security.js: 12 pass / 0 fail
node target/test-md-v2.js: 24 pass / 0 fail
```

神圣不变式 **2/2 pass**。所有测试通过。

## 关键发现（A-1）

### F-2 `formatError` 白名单是 dead code — 实际跑 engine 验证

**自审结论**（PR 内自审报告 §F-2）：
> "KNOWN_FIELDS 白名单与 engine `AirpError` 当前 variant 字段对齐" — 暗示这些字段会被 webui 看到

**独立审计反驳**：
1. 实际启动 PR #74 编译后的 `airp-core.exe daemon`，curl 一个不存在的角色 lorebook endpoint：
   ```
   HTTP/1.1 404 Not Found
   content-type: text/plain; charset=utf-8
   content-length: 64

   资源不存在: lorebook for character does_not_exist not found
   ```
2. 读 `engine/src/error.rs:108-121` `IntoResponse for AirpError`：
   ```rust
   (status, body).into_response()  // body = self.to_string()，纯字符串
   ```
3. 读 `webui/app.js:96-104` `api()` 解析逻辑：
   ```javascript
   let data;
   try { data = JSON.parse(text); } catch { data = text; }
   return { ok, status, data, text };
   ```
4. 对 AirpError 响应：`text` 是 `"资源不存在: ..."`，`JSON.parse` 失败，`data = text`（字符串）。
5. `formatError` 第 110-134 行的 `if (data && typeof data === 'object' && data.error)` 对字符串永远不成立 → **fall through** 到第 131 行 `if (typeof data === 'string' && data) return data;`
6. 实际 Node.js 模拟跑出（与 PR 编译后 webui 行为一致）：
   ```
   Case 1: Real engine AirpError response
     data type: string
     formatError output: "资源不存在: lorebook for character does_not_exist not found"
     whitelist fired? false

   Case 2: Hypothetical JSON envelope
     data type: object
     formatError output: "NOT_FOUND\nlorebook missing\nextras={\"hint\":\"try creating one\"}"
     whitelist fired? true
   ```

**结论**：F-2 的 `KNOWN_FIELDS` / `extras` 折叠逻辑对当前 engine 的所有响应都是 **dead code**。engine 没有任何 handler 返回 `{error: {code, message, ...}}` JSON envelope（grep 全仓 0 命中）。F-2 提升的是"未来如果 engine 改用 JSON envelope"的前向兼容性，**不是当前行为修复**。

**这是自审的盲点**：自审在 "verification point 2" 写"未知字段折叠为 extras={...} raw JSON, engine 错误模型扩展（如 request_id/hint/suggestion）时不丢失" — 但这要求 engine **先**改成 envelope 格式。当前 PR 没改 engine 的 IntoResponse，所以 F-2 没有真实可观测的修复效果。

**修复方向建议**（A-1 fix）：F-1 应该连带改 `AirpError::IntoResponse` 为 JSON envelope 输出，例如：
```rust
fn into_response(self) -> Response {
    let status = self.status();
    let body = serde_json::json!({
        "error": {
            "code": self.code_str(),  // "not_found" / "bad_request" / "internal" ...
            "message": self.to_string(),
        }
    });
    if status == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::error!(err = %self, "internal error");
        (status, json!({"error":{"code":"internal","message":"internal error"}})).into_response()
    } else {
        (status, body).into_response()
    }
}
```
这样 F-1 + F-2 才形成完整闭环：engine 返回结构化错误 → webui 用白名单展开 + extras 折叠显示。

## 逐项独立审计

### F-1 `get_character_lorebook` 错误格式统一

**自审结论**：PASS

**独立审计结论**：**部分 PASS**（行为确实改善，但有未列出的 W 项）

**自审未提及的 W-05**：
- 旧代码：200 + 任意 content + 任意 body（包括无效 JSON）；400 裸状态无 body；404 裸状态无 body
- 新代码：200 + 合法 JSON（parse 校验）；400 + plain text "非法请求: ..."；404 + plain text "资源不存在: ..."；500（parse 失败）+ "internal error"（细节入 tracing）
- **行为改善成立**（status code 现在正确，error body 不再为空，parse 校验严格化）
- **W-05（新发现）**：F-1 与 F-2 应该一起做，但 F-1 改完 engine 端就停了，webui 端的 F-2 是 dead code（见 A-1）

**W-01（自审已列）**：缺 HTTP-level 回归测试。已建好的 M3 测试 pattern（`m3_import_card_path_rejected_at_http_level` 在 handlers.rs:1317）应复用。建议补 3 个测试：
- 不存在角色 → 404 + plain text body
- character_id 含非法字符 → 400 + BadRequest 文案
- 写入坏 JSON 的 lorebook 文件 → 500 + "internal error" body

### F-2 `formatError` 白名单

**自审结论**：PASS

**独立审计结论**：**仅作前向兼容，无当前可见效果**（见 A-1）

自审写"避免 engine 错误模型扩展时 webui 自动丢失" — 但当前 engine 错误模型**没有** envelope 包装。F-2 的代码是 defensive 但不可观测。

**W-02（自审已列）保持成立**：KNOWN_FIELDS 硬编码。但即便修了 W-02，current engine 响应也不会触发它。

### F-3 `scheduleAutoConnect` / `cancelAutoConnect` 竞态保护

**自审结论**：PASS（含 F-5 自审发现）

**独立审计结论**：**PASS**

逐项验证：
1. `let pendingAutoConnect = null` 在 IIFE 内，非 hoisted（TDZ）
2. `function cancelAutoConnect() { ... }` hoisted 到 IIFE 顶部
3. `function connect() { ... cancelAutoConnect() ... }` hoisted，body 只在调用时执行
4. `engineUrl.addEventListener('input', cancelAutoConnect)` 在 line 180 注册
5. IIFE 末尾 `scheduleAutoConnect()` 注册 300ms 后调 connect

TDZ 风险分析：所有 `cancelAutoConnect` 调用点（line 139、line 180 的 handler）都在 IIFE 同步执行到 line 167（`let pendingAutoConnect` 初始化）之后才可能触发。**安全**。

但有 **W-06（自审未列）**：ordering 脆弱。如果未来有人把 `engineUrl.addEventListener('input', cancelAutoConnect)` 移到 `let pendingAutoConnect` 之前，且在 input 事件触发早于 IIFE 完成（实际不可能但属于代码阅读心智负担），会出 TDZ。**建议**：在 IIFE 顶部把 `let pendingAutoConnect = null` 提到第一个声明处，与 `let abortController = null` 等同区。

**F-5 自审发现**：connect() 入口加 `cancelAutoConnect()`。**正确** — 覆盖 keydown Enter 和 btn-click 两条路径。

### F-4 `formatSessionTime` / `renderSessionInfo`

**自审结论**：PASS

**独立审计结论**：**PASS**（W-04 同意外）

逐项验证：
1. `new Date(iso).getTime()` + `Number.isNaN` 防御非法 ISO ✓
2. 空会话 `!hasMsgs` 返回 null，不占位 ✓
3. 同一天 `created.slice(0, 10) === updated.slice(0, 10)` — `formatSessionTime` 返回 `"YYYY-MM-DD HH:MM"`，前 10 字符是日期。比较的是**本地**日期前缀（因 `getMonth()/getDate()` 是本地时区）。**这是 intended 行为**（"同一天"是用户视角的本地日）✓
4. 时间戳缺失退化消息数提示 ✓
5. `chatLog.appendChild(info)` 在 msgs.forEach 之前 → 顺序正确 ✓
6. CSS `.session-info` 低视觉权重 ✓

**W-07（自审未列）**：边界 — `data?.messages` 为非数组（极少见）时，`Array.isArray(msgs)` 返回 false，return null。会话被当作"空"不显示时间条。合理降级。

## 综合结论

### 必改项（合并前修复）

- **A-1（必改）**：F-2 是 dead code。建议二选一：
  - 方案 A（推荐）：F-1 同时改 `AirpError::IntoResponse` 输出 JSON envelope，让 F-1+F-2 形成完整闭环
  - 方案 B（保守）：revert F-2 改动，承认 forward-compat 不是当前 issue (#67 #9) 的真实修复路径
- **W-01（建议改）**：补 `get_character_lorebook` HTTP-level 回归测试，参照 M3 pattern

### 可后续 issue

- **W-06**：把 `let pendingAutoConnect = null` 提到 IIFE 顶部声明区
- **W-04**：跨时区时间显示（自审已列）保持后续 issue

### PASS 项

- F-3 竞态保护 + F-5 自审发现
- F-4 会话时间范围

### 不修项

- #67 #3 / #67 #4 / #68 #6（自审已列）保持不修

## 评分

| 维度 | 评价 |
|---|---|
| 行为正确性 | **B-**（F-1 改善成立，F-2 不可观测） |
| 内部一致性 | **C**（F-1 + F-2 应联动但实际断开） |
| 测试覆盖 | **B-**（已有测试全过，但 W-01 缺新增） |
| 文档质量 | **B**（自审报告详尽但 A-1 盲点） |
| 总体 | **建议有条件合并**：要么补 A-1 fix，要么 revert F-2 |

**审计 LLM 模型**：GLM-5.2

**审计时长**：实际跑 engine + curl + Node 模拟 + 源码逐行验证 ≈ 8 分钟
