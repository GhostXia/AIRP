# C-PR1：Persona WebUI 闭环

> 日期：2026-07-15
>
> 方向：#114 Persona/Preset WebUI 闭环（拆分为 C-PR1 + C-PR2，本文仅 C-PR1）
>
> 基线：`main@db4fc12`（PR #179）
>
> 并行策略：C-PR1 与 D（#126 Worldbook）并行；C-PR2（Preset 生命周期 + revision 合同）随后；B（#115 PromptAssemblyTrace 接线）后移至 C-PR2 合并后

## 1. 目标

让用户不编辑 JSON、不打开开发工作台即可完成 Persona 的绑定、聊天时切换与生效可见性，闭合 #114 的 Persona 侧产品缺口。

## 2. 范围

### 包含

- Persona 下拉加「自动」选项，区分 explicit 覆盖与跟随绑定/默认
- persona-section 内嵌绑定/解绑按钮（角色级 + 会话级）
- 新 HTTP 端点查询有效 Persona（解析后实际生效的 persona + 来源）
- WebUI 在选中「自动」时展示有效 Persona
- `buildChatPayload` 按选择传/不传 `persona_id`

### 不包含

- Preset 生命周期、revision 合同、dry-run（属 C-PR2）
- 有效配置摘要的统一 UI（Preset 侧由 C-PR2 做，Persona 侧本文只做生效展示）
- Persona CRUD（已交付）
- Persona 绑定 HTTP 端点（已交付）
- chat_pipeline 内的 persona 激活逻辑（已交付，零改动）

## 3. 背景：已交付地基

| 能力 | 位置 | 状态 |
|---|---|---|
| Persona HTTP CRUD（9 端点） | [personas.rs](../../../engine/src/daemon/handlers/personas.rs) | 已交付 |
| 绑定 HTTP（POST/DELETE bindings） | [personas.rs](../../../engine/src/daemon/handlers/personas.rs#L175) | 已交付 |
| `PersonaBinding` = `{character_id, session_id?}` | [domain.rs](../../../engine/src/domain.rs) | 已交付 |
| `find_for_character` / `bind` / `unbind` | [domain.rs](../../../engine/src/domain.rs#L914) | 已交付；同 scope 多绑定会 fail closed，但 `bind` 目前不会主动阻止冲突 |
| chat_pipeline persona 解析（explicit→binding→default） | [chat_pipeline.rs](../../../engine/src/chat_pipeline.rs#L120) `resolve_request_persona` | 已交付 |
| `ChatCompletionRequest.user_id/persona_id` | [types.rs](../../../engine/src/daemon/types.rs#L49) | 已交付；`persona_id` 仅在同时传 `user_id` 时生效 |
| WebUI Persona 列表/编辑/本地选择 | [app.js](../../../webui/app.js#L460) | 已交付 |
| Persona revision 冲突合同 | [domain.rs](../../../engine/src/domain.rs#L731) `PersonaRevisionConflict` | 已交付 |

## 4. 设计

### 4.1 新 HTTP 端点：有效 Persona 查询

```text
GET /v1/users/:user_id/persona/effective?character_id=<id>&session_id=<id>
```

- `character_id` 必填，`session_id` 可选
- 响应体：

```json
{
  "persona": { "id": "...", "name": "...", "revision": 3, "...": "..." },
  "source": "session_binding | character_binding | default",
  "bindings": {
    "character_persona_id": "writer | null",
    "session_persona_id": "roleplay | null"
  }
}
```

端点放在 legacy singular `persona` 路径下，避免把 `effective` 占作
`/personas/:persona_id` 的保留 ID。本端点只解析 binding→default 两层（explicit
层由 WebUI 本地根据下拉选择判定，不进端点）：

- `source=session_binding`：当前 session scope 恰有一个 owner，返回该 Persona。
- `source=character_binding`：session scope 无 owner，character scope 恰有一个 owner，返回该 Persona。
- `source=default`：两个 scope 都无 owner，返回 default Persona。
- `bindings` 同时返回两个 scope 的 owner，供两个按钮分别决策；没有 session_id 时
  `session_persona_id=null`。
- 任一 scope 有多个 owner 时沿用 fail-closed 原则，返回 typed `400 bad_request`，
  响应中指出冲突 scope 与 Persona IDs；不得挑文件名最靠前者，也不得静默回退 default。

实现方式：在 [domain.rs](../../../engine/src/domain.rs#L964) 增加共享的结构化
binding inspection/resolution 方法，同时产出 effective owner、命中 scope 与两个 scope
owner；现有 `find_for_character` 和 [chat pipeline](../../../engine/src/chat_pipeline.rs#L120)
改为复用该方法，handler 不重新扫描 Persona 文件。在
[personas.rs](../../../engine/src/daemon/handlers/personas.rs) 加
`get_effective_persona_endpoint`，再用 `get_default` 补 default。这样 HTTP 可观察结果与
聊天激活使用同一真相。

### 4.2 Persona 下拉「自动」选项

[index.html](../../../webui/index.html#L55) `persona-select` 在 `refreshPersonaList` 填充时，顶部插入：

```html
<option value="">自动（跟随绑定/默认）</option>
```

- `selectedPersonaId = ""` 表示「自动」；空字符串只存在于 WebUI，不是 Persona ID
- localStorage `airp_persona_id` 明确存空字符串；`rememberWorkspace`、change handler、
  `refreshPersonaList` 与启动恢复都用 `getItem(...) === null` 区分“未设置”和空字符串，
  不得再用 `|| "default"` 抹掉「自动」
- 选中「自动」时 persona 编辑表单显示当前 effective persona 的内容（只读），底部标注「以上为生效 Persona（来自绑定/默认），如需编辑请先在下拉选择具体 Persona」

### 4.3 persona-section 内嵌绑定按钮

在 [index.html](../../../webui/index.html) persona-form 内（`form-actions` 下方或 `persona-status` 上方）加：

```html
<div class="persona-binding-row" id="persona-binding-row">
  <button id="btn-bind-character" type="button" disabled>绑定到角色</button>
  <button id="btn-bind-session" type="button" disabled>绑定到会话</button>
  <span id="persona-effective-hint" class="hint">—</span>
</div>
```

按钮状态规则由 `bindings.character_persona_id` 与
`bindings.session_persona_id` **分别**驱动，不能只看 effective Persona：

| 当前 scope 状态 | 对应按钮行为 |
|---|---|
| 无 character，或下拉为「自动」 | 两个按钮 disabled |
| 无 session | 角色按钮按下列规则；会话按钮 disabled |
| scope 无 owner | 「绑定到角色/会话」，POST 绑定 selected Persona |
| scope owner = selected Persona | 「解绑角色/会话」，DELETE selected Persona 的该 scope 绑定 |
| scope owner ≠ selected Persona | 「先解绑 {owner}」，DELETE owner 的该 scope 绑定；刷新后由用户第二次点击绑定 selected Persona |
| effective 端点报告同 scope 多 owner | 两个按钮 disabled，显示冲突 IDs；不创建更多绑定 |

绑定操作目标 = 当前下拉选中的 persona（「自动」时按钮 disabled，因为「自动」不对应具体 persona）。

绑定/解绑调用现有端点：
- 绑定：`POST /v1/users/:user_id/personas/:persona_id/bindings` body `{character_id, session_id?}`
- 解绑角色：`DELETE /v1/users/:user_id/personas/:persona_id/bindings?character_id=X`
- 解绑会话：`DELETE /v1/users/:user_id/personas/:persona_id/bindings?character_id=X&session_id=Y`

POST 不替换其他 Persona 对同 scope 的绑定；owner 不同时仍须先显式解绑、刷新，再允许
绑定。服务端在持有 per-user Persona 保存锁时检查所有 binding scope owner：若刷新后有
其他客户端抢先绑定同一 scope，POST 返回 typed `400 bad_request`，不得持久化多 owner
状态。每次操作后刷新 effective 端点。

### 4.4 有效 Persona 展示

`persona-effective-hint` span 显示：

- 下拉选中具体 persona：显示「已选择：{name}（explicit）」
- 下拉选中「自动」：
  - `source=session_binding`：「生效：{name}（来自会话绑定）」
  - `source=character_binding`：「生效：{name}（来自角色绑定）」
  - `source=default`：「生效：{default_name}（默认）」

character/session 切换时触发 effective 端点刷新。

### 4.5 buildChatPayload 改动

[app.js](../../../webui/app.js#L1160) `buildChatPayload`：

```js
// 现状：不传 persona_id
// 改为：
const payload = { /* ...existing fields... */ };
payload.user_id = personaUserId.value.trim() || "default";
if (selectedPersonaId) {  // 非空 = 具体 persona
  payload.persona_id = selectedPersonaId;
}
// selectedPersonaId === "" 时不传，pipeline 按 binding→default 解析
```

`persona_id` 在 engine 中只有与 `user_id` 同时出现才生效，因此两者必须一起接线。
同时把 `user_profile` 改为非权威 override（默认 `{name: "", variables: {}}`），避免
WebUI 缓存的表单值覆盖 engine 刚解析出的 Persona；未来显式 override 需要独立 UI 与合同，
本切片不提供。effective 查询、Persona CRUD 与 chat payload 必须读取同一个当前 user ID。

## 5. 数据流

```text
character/session 切换
  → WebUI 调 GET .../persona/effective?character_id&session_id
  → 返回 {persona, source, bindings}
  → 驱动绑定/解绑按钮状态 + effective hint 展示
  → 若下拉为「自动」，表单填入 effective persona（只读）

用户切下拉
  → 具体 persona：表单可编辑，hint 显示「explicit」
  → 「自动」：表单只读填 effective，hint 显示 source

用户点绑定/解绑
  → POST/DELETE .../bindings
  → 刷新 effective 端点

用户发消息
  → buildChatPayload：selectedPersonaId 非空 → 传 persona_id
  → selectedPersonaId 空 → 不传，pipeline 解析
```

## 6. 边界与错误处理

| 场景 | 行为 |
|---|---|
| 无选中 character | 绑定/解绑按钮 disabled；effective 不请求或显示「请先选择角色」 |
| 无选中 session | 「绑定到会话」disabled；effective 用 session_id 缺失请求 |
| 下拉「自动」时点绑定 | 按钮 disabled（无具体 persona 可绑） |
| character_id 语法有效但当前无角色文件 | 与 `find_for_character` 一致：无绑定时返回 default；端点不承担角色存在性校验 |
| character_id/session_id 格式非法 | typed `400 bad_request`；hint 显示可行动错误，按钮 disabled |
| 同 scope 多 Persona 绑定 | typed `400 bad_request`，列出冲突 scope/IDs；不回退 default，按钮 disabled |
| resolve 后并发删除 Persona | 返回 typed `404` 并刷新列表；绑定存于 Persona 文件内，不虚构“悬空绑定回退”状态 |
| 绑定 default persona 到角色 | 允许（让 default 对该角色显式化） |
| revision 冲突 | 复用现有 persona 编辑的 PersonaRevisionConflict 提示路径 |

## 7. 测试

### engine

在 [daemon/tests](../../../engine/src/daemon/tests) persona 分组加 effective 端点路由测试：

1. 无绑定 → `source=default`，返回 default persona
2. character 级绑定 → `source=character_binding`，character owner 正确
3. session 级绑定覆盖 character 级 → `source=session_binding`，返回 session Persona
4. 语法有效但不存在的 character ID、且无绑定 → 返回 default（本端点不查角色文件）
5. 无 session_id 参数 → 只查 character 级绑定
6. character 与 session owner 不同 → 返回 session Persona，同时返回两个 scope owner
7. 同 scope 多 owner → typed `400`，不得静默选取或回退

### WebUI

在 tracked `webui/tests/` 下加 Node 测试，并在 PR gate 运行
`node --test webui/tests/*.test.mjs`。把 persona 选择持久化、按钮决策与 payload
装配提取为可直接测试的纯函数；真实 DOM 接线另加 system-Chrome smoke：

1. 下拉渲染含「自动」选项
2. 选中「自动」→ `buildChatPayload` 不含 `persona_id` 字段
3. 选中具体 persona → `buildChatPayload` 含 `persona_id`
4. character 选中后 → effective 端点被调用，hint 显示生效 persona
5. character/session owner 分别驱动对应按钮；owner 相同显示「解绑」，owner 不同先解绑旧 owner
6. 无 character → 绑定按钮 disabled
7. 自动选择刷新后仍为自动，不回退 default
8. payload 总是含当前 `user_id`；自动不含 `persona_id`；非自动含 `persona_id`

### 不变式

- `subagent_context_has_no_orchestrator_noise` 全绿（C-PR1 不碰 chat_pipeline）
- `subagent_prepared_pipeline_has_no_orchestrator_noise` 全绿
- 当前 workspace 全部 Rust/UI/WebUI/production gates 不退化；不在 spec 固化会漂移的测试数量

## 8. 文件改动清单

### engine

- [domain.rs](../../../engine/src/domain.rs)：加结构化 binding inspection/resolution；`find_for_character` 复用
- [chat_pipeline.rs](../../../engine/src/chat_pipeline.rs)：改用同一结构化 resolver，不改变 explicit → binding → default 优先级
- [personas.rs](../../../engine/src/daemon/handlers/personas.rs)：加 `get_effective_persona_endpoint` + 请求/响应 DTO
- [daemon/mod.rs](../../../engine/src/daemon/mod.rs)：注册 `/persona/effective` 路由
- [daemon/tests](../../../engine/src/daemon/tests) persona 分组：加 effective 端点测试

### WebUI

- [index.html](../../../webui/index.html)：persona-section 加绑定按钮行 + effective hint
- [app.js](../../../webui/app.js)：
  - DOM refs 加绑定按钮 + hint
  - `refreshPersonaList` 插入「自动」option
  - `refreshPersona` 处理「自动」选中的只读表单
  - character/session 切换 handler 加 effective 端点调用
  - 绑定/解绑按钮事件
  - `buildChatPayload` 始终传 `user_id`，并按自动/显式传或不传 `persona_id`
  - 默认不再用缓存 Persona 内容覆盖 engine resolution
- `webui/tests/*.test.mjs`：纯状态/payload 合同测试，并接入 PR gate
- [ui/production-browser-smoke.mjs](../../../ui/production-browser-smoke.mjs)：验证真实下拉、按钮与 hint DOM 接线

## 9. 验收标准

1. 用户选中「自动」+ 发消息 → payload 含 `user_id`、不含 `persona_id`，engine pipeline 用 binding 或 default Persona
2. 用户选中具体 persona + 发消息 → payload 同时含 `user_id`/`persona_id`，engine 用该 Persona（explicit）
3. 用户点「绑定到角色」→ 该角色下次「自动」时生效 persona 变为绑定的 persona
4. 用户点「解绑」→ 该角色回退到 default
5. character/session 切换 → effective hint 正确刷新
6. 无选中 character → 绑定按钮 disabled
7. engine 全套测试 + WebUI DOM 测试全绿
8. 2 个 subagent 不变式全绿
