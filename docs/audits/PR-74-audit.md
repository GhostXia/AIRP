# PR #74 审计报告 — webui-pr-h-issue-cleanup

- **审计源 LLM**: GLM-5.2
- **审计日期**: 2026-07-06
- **审计范围**: issue cleanup 第一批 — #67 #5, #67 #9, #68 #5, #73 方案A
- **审计基线**: commit a32a714 (main)
- **审计分支**: webui-pr-h-issue-cleanup
- **审计模式**: 开发者自审（AGENTS.md §11.1 — 原"审计 bot 复核"已下线）

## 审计守则确认

按 AGENTS.md "审计 agent 守则" 三条执行：
1. 独立审计 — 不附和开发侧结论，重新读源码验证每项修复
2. 提出自己的想法 — 见 W 项
3. 质疑历史决策 — 已验证 `CharacterId::new` / `load_or_create` 等历史代码与 issue 描述的差异

## 修复清单

| 编号 | issue | 修复点 | 文件 | 行号 |
|---|---|---|---|---|
| F-1 | #67 #5 | `get_character_lorebook` 返回 `Result<Json<Value>, AirpError>` 统一错误 body | engine/src/daemon/handlers.rs | 845-867 |
| F-2 | #67 #9 | `formatError` 白名单展开 + extras 折叠 raw JSON | webui/app.js | 107-134 |
| F-3 | #68 #5 | `scheduleAutoConnect` / `cancelAutoConnect` 竞态保护 + `connect()` 入口取消 pending | webui/app.js | 136-181, 1524-1526 |
| F-4 | #73 方案A | `loadHistory` 显示会话时间范围（created_at → updated_at） | webui/app.js | 638-694 |
| F-5 | (自审发现) | keydown Enter / btn-click 触发 `connect()` 不取消 pending，300ms 后重复请求 | webui/app.js:136-139 | 自审修复 |

## 验证结果

### 测试

- `cargo test --manifest-path engine/Cargo.toml`: **358 pass / 0 fail**（含 2/2 神圣不变式）
- `node --check webui/app.js webui/serve.js`: 通过
- `node target/test-serve-security.js`: **12 pass / 0 fail**
- `node target/test-md-v2.js`: **24 pass / 0 fail**

### 神圣不变式

```
test agent::tests::subagent_context_has_no_orchestrator_noise ... ok
test agent::tests::subagent_prepared_pipeline_has_no_orchestrator_noise ... ok
```

## 逐项审计

### F-1: `get_character_lorebook` 错误格式统一 (#67 #5)

**修改前**:
```rust
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    // ...
    match fs::read_to_string(&lb_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
```

**修改后**:
```rust
) -> Result<Json<serde_json::Value>, AirpError> {
    let char_id = CharacterId::new(character_id)?;
    let lb_path = data_dir::char_world_lorebook_path(&state.data_root, char_id.as_str());
    match fs::read_to_string(&lb_path) {
        Ok(json) => {
            let value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|e| AirpError::Internal(format!("lorebook parse error: {e}")))?;
            Ok(Json(value))
        }
        Err(_) => Err(AirpError::NotFound(format!(
            "lorebook for character {} not found",
            char_id
        ))),
    }
}
```

**审计结论**: PASS

**验证点**:
1. `CharacterId::new` 失败返回 `AirpError::BadRequest`（types.rs:21-25），`?` 传播后 axum `IntoResponse` 实现 JSON 错误 body — 客户端 `formatError` 可读
2. `fs::read_to_string` 失败 → `AirpError::NotFound`，错误信息含 `char_id`（Display trait 实现见 types.rs:44-48）。`char_id` 已过 `validate_id_segment` 校验（无路径遍历/空字节），可安全写入错误响应
3. parse 失败 → `AirpError::Internal`。语义正确：文件存在但内容损坏属 internal error，不是 not found
4. **新增改动**: 旧代码直接返回 raw string `(headers, json).into_response()`，新代码 `serde_json::from_str` 后用 `Json<Value>` 返回。行为差异：parse 失败时旧代码返回 200 + raw string（content-type=application/json 但 body 非 JSON），新代码返回 500。新行为更严格、更正确

**遗留 W 项**:
- **W-01**: `get_character_lorebook` 未加 HTTP 级回归测试（参照 PR #65 M3 模式）。建议后续 PR 补一个 test。

### F-2: `formatError` 白名单 + extras 折叠 (#67 #9)

**审计结论**: PASS

**验证点**:
1. `KNOWN_FIELDS` 白名单与 engine `AirpError` 当前 variant 字段对齐（code/message/upstream_status/upstream_body/detail）
2. 未知字段折叠为 `extras={...}` raw JSON，engine 错误模型扩展（如 request_id/hint/suggestion）时不丢失
3. `Object.keys(err)` 遍历包含已知字段，但被 `KNOWN_FIELDS.includes(k)` 过滤，不会重复展开

**遗留 W 项**:
- **W-02**: `KNOWN_FIELDS` 是硬编码白名单。engine 错误模型扩展新"已知"字段时仍需手动同步 webui，否则新字段会进入 `extras`（不丢失，但展示位置不理想）。可接受 — extras 仍是可读 JSON。

### F-3: auto-connect 竞态保护 (#68 #5)

**审计结论**: PASS（含自审修复 F-5）

**验证点**:
1. `pendingAutoConnect` 用 `let` 模块级闭包变量，`connect()` / `scheduleAutoConnect()` / `cancelAutoConnect()` 三处访问闭包一致
2. `connect()` 入口调用 `cancelAutoConnect()`（F-5 修复）— 防止 keydown Enter / btn-click 后 300ms 重复触发
3. `engineUrl.addEventListener('input', cancelAutoConnect)` — 用户输入时取消 pending，避免读半截值
4. `scheduleAutoConnect()` 在 IIFE 末尾调用，300ms 后触发 connect

**遗留 W 项**:
- **W-03**: `pendingAutoConnect` 是闭包内变量，无法跨 reload 持久化。可接受 — 页面 reload 后 pending timer 自然清空。

### F-4: 会话时间范围显示 (#73 方案A)

**审计结论**: PASS

**验证点**:
1. `formatSessionTime` 用 `new Date(iso)` 解析 ISO 8601 — 现代浏览器支持。`Number.isNaN(d.getTime())` 防御非法字符串
2. `renderSessionInfo` 空会话（`!hasMsgs`）返回 null，不占用顶部空间
3. 同一天（`created.slice(0, 10) === updated.slice(0, 10)`）只显示一次日期 + 时间范围，避免冗余
4. 时间戳缺失时退化为消息数提示，不阻断渲染
5. `chatLog.appendChild(info)` 在 `msgs.forEach(appendMsg)` 之前 — 信息条在顶部，消息在下，顺序正确
6. CSS `.session-info` 低视觉权重（11px / #8b949e / 浅灰底），不抢消息流焦点

**遗留 W 项**:
- **W-04**: `formatSessionTime` 用本地时区显示。若用户跨时区协作（如远程查看另一时区的会话），时间显示可能与对方不一致。当前 webui 是单用户本地控制台，可接受。

## 不修项（issue 误判，已在 issue 下评论说明）

| issue | 描述 | 不修原因 |
|---|---|---|
| #67 #3 | bearer 截断 panic 风险 | `mod.rs:152` 已用 `token.chars().take(32).collect()`，修复在历史 PR 完成 |
| #67 #4 | chat/history 对不存在 character_id 应返回 404 | 当前 `load_or_create` 自动创建空 log 返回 200（lazy init），比 404 更友好 |
| #68 #6 | 缺 agent max_steps UI input | `index.html:114` 已有 `<input id="agent-max-steps" min="1" max="20" />`，`app.js:774` 已读取并钳位 |

## 综合结论

**推荐合并** — 4 项 issue 修复 + 1 项自审发现修复，全部测试通过，含 2/2 神圣不变式。

遗留 W 项均为低优先级，可后续迭代：
- W-01: lorebook HTTP 回归测试（建议下次 webui 测试加固 PR）
- W-02: formatError KNOWN_FIELDS 与 engine 错误模型同步（可接受当前 extras 折叠行为）
- W-03: pendingAutoConnect 跨 reload 持久化（不修，自然清空是 desired）
- W-04: 跨时区时间显示（当前单用户场景可接受）
