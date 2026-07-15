# C-PR1：Persona WebUI 闭环

> 日期：2026-07-15
>
> 方向：#114 Persona/Preset WebUI 闭环（拆分为 C-PR1 + C-PR2，本文仅 C-PR1）
>
> 基线：`main@c54428e`（PR #177）
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
| Persona HTTP CRUD（9 端点） | [personas.rs](file:///d:/AIRP-Dev/engine/src/daemon/handlers/personas.rs) | 已交付 |
| 绑定 HTTP（POST/DELETE bindings） | [personas.rs:175-210](file:///d:/AIRP-Dev/engine/src/daemon/handlers/personas.rs) | 已交付 |
| `PersonaBinding` = `{character_id, session_id?}` | [domain.rs](file:///d:/AIRP-Dev/engine/src/domain.rs) | 已交付 |
| `find_for_character` / `bind` / `unbind` | [domain.rs:914-1012](file:///d:/AIRP-Dev/engine/src/domain.rs) | 已交付 |
| chat_pipeline persona 解析（explicit→binding→default） | [chat_pipeline.rs:120-154](file:///d:/AIRP-Dev/engine/src/chat_pipeline.rs) `resolve_request_persona` | 已交付 |
| `ChatCompletionRequest.persona_id` | [types.rs:52-59](file:///d:/AIRP-Dev/engine/src/daemon/types.rs) | 已交付 |
| WebUI Persona 列表/编辑/本地选择 | [app.js:460-512](file:///d:/AIRP-Dev/webui/app.js) | 已交付 |
| Persona revision 冲突合同 | [domain.rs:731-735](file:///d:/AIRP-Dev/engine/src/domain.rs) `PersonaRevisionConflict` | 已交付 |

## 4. 设计

### 4.1 新 HTTP 端点：有效 Persona 查询

```
GET /v1/users/:user_id/personas/effective?character_id=<id>&session_id=<id>
```

- `character_id` 必填，`session_id` 可选
- 响应体：

```json
{
  "persona": { "id": "...", "name": "...", "revision": 3, "...": "..." },
  "source": "binding | default",
  "bound_persona_id": "writer | null"
}
```

本端点只解析 binding→default 两层（explicit 层由 WebUI 本地根据下拉选择判定，不进端点）：

- `source=binding`：`find_for_character(user, character_id, session_id)` 返回 `Some(pid)` → 读该 persona，`bound_persona_id=Some(pid)`
- `source=default`：返回 `None` → 读 default persona，`bound_persona_id=None`

实现方式：在 [personas.rs](file:///d:/AIRP-Dev/engine/src/daemon/handlers/personas.rs) 加 `get_effective_persona_endpoint`。复用 `PersonaService::find_for_character`（character 级 + session 级回退）与 `get_default`。`find_for_character` 已在 [chat_pipeline.rs:120-154](file:///d:/AIRP-Dev/engine/src/chat_pipeline.rs) 消费，逻辑一致，不在 handler 内重新发明解析顺序。

### 4.2 Persona 下拉「自动」选项

[index.html:55](file:///d:/AIRP-Dev/webui/index.html) `persona-select` 在 `refreshPersonaList` 填充时，顶部插入：

```html
<option value="">自动（跟随绑定/默认）</option>
```

- `selectedPersonaId = ""` 表示「自动」
- localStorage `airp_persona_id` 存空字符串
- 选中「自动」时 persona 编辑表单显示当前 effective persona 的内容（只读），底部标注「以上为生效 Persona（来自绑定/默认），如需编辑请先在下拉选择具体 Persona」

### 4.3 persona-section 内嵌绑定按钮

在 [index.html](file:///d:/AIRP-Dev/webui/index.html) persona-form 内（`form-actions` 下方或 `persona-status` 上方）加：

```html
<div class="persona-binding-row" id="persona-binding-row">
  <button id="btn-bind-character" type="button" disabled>绑定到角色</button>
  <button id="btn-bind-session" type="button" disabled>绑定到会话</button>
  <span id="persona-effective-hint" class="hint">—</span>
</div>
```

按钮状态规则（由 effective 端点结果驱动）：

| 条件 | 「绑定到角色」 | 「绑定到会话」 |
|---|---|---|
| 无选中 character | disabled | disabled |
| 有 character，无 session | enabled（文案视情况） | disabled |
| 有 character + session | enabled | enabled |
| `source=binding` 且 `bound_persona_id === selectedPersonaId` 且非「自动」 | 文案变「解绑角色」，点击调 DELETE | 文案变「解绑会话」，点击调 DELETE |

绑定操作目标 = 当前下拉选中的 persona（「自动」时按钮 disabled，因为「自动」不对应具体 persona）。

绑定/解绑调用现有端点：
- 绑定：`POST /v1/users/:user_id/personas/:persona_id/bindings` body `{character_id, session_id?}`
- 解绑角色：`DELETE /v1/users/:user_id/personas/:persona_id/bindings?character_id=X`
- 解绑会话：`DELETE /v1/users/:user_id/personas/:persona_id/bindings?character_id=X&session_id=Y`

操作后刷新 effective 端点。

### 4.4 有效 Persona 展示

`persona-effective-hint` span 显示：

- 下拉选中具体 persona：显示「已选择：{name}（explicit）」
- 下拉选中「自动」：
  - `source=binding`：「生效：{bound_persona_name}（来自{角色/会话}绑定）」
  - `source=default`：「生效：{default_name}（默认）」

character/session 切换时触发 effective 端点刷新。

### 4.5 buildChatPayload 改动

[app.js:1160-1169](file:///d:/AIRP-Dev/webui/app.js) `buildChatPayload`：

```js
// 现状：不传 persona_id
// 改为：
const payload = { /* ...existing fields... */ };
if (selectedPersonaId) {  // 非空 = 具体 persona
  payload.persona_id = selectedPersonaId;
}
// selectedPersonaId === "" 时不传，pipeline 按 binding→default 解析
```

## 5. 数据流

```
character/session 切换
  → WebUI 调 GET .../personas/effective?character_id&session_id
  → 返回 {persona, source, bound_persona_id}
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
| effective 端点 404（character 不存在） | hint 显示「无生效 Persona」；不阻塞聊天 |
| effective 端点 persona 404（绑定的 persona 被删） | hint 显示「绑定的 Persona 已不存在，将使用默认」；建议清理无效绑定（后续改进） |
| 绑定 default persona 到角色 | 允许（让 default 对该角色显式化） |
| revision 冲突 | 复用现有 persona 编辑的 PersonaRevisionConflict 提示路径 |

## 7. 测试

### engine

在 [daemon/tests](file:///d:/AIRP-Dev/engine/src/daemon) persona 分组加 effective 端点路由测试：

1. 无绑定 → `source=default`，返回 default persona
2. character 级绑定 → `source=binding`，`bound_persona_id` 正确
3. session 级绑定覆盖 character 级 → `source=binding`，返回 session 级 persona
4. character 不存在 → 仍返回 default（不 404，因为 default 不依赖 character）
5. 无 session_id 参数 → 只查 character 级绑定

### WebUI

在 `target/` 下加 DOM 测试：

1. 下拉渲染含「自动」选项
2. 选中「自动」→ `buildChatPayload` 不含 `persona_id` 字段
3. 选中具体 persona → `buildChatPayload` 含 `persona_id`
4. character 选中后 → effective 端点被调用，hint 显示生效 persona
5. `source=binding` 且 `bound_persona_id === selectedPersonaId` → 按钮文案变「解绑」
6. 无 character → 绑定按钮 disabled

### 不变式

- `subagent_context_has_no_orchestrator_noise` 全绿（C-PR1 不碰 chat_pipeline）
- `subagent_prepared_pipeline_has_no_orchestrator_noise` 全绿
- 现有 552 lib + 1 ignored + 40 integration 不退化

## 8. 文件改动清单

### engine

- [personas.rs](file:///d:/AIRP-Dev/engine/src/daemon/handlers/personas.rs)：加 `get_effective_persona_endpoint` + 请求/响应 DTO + 路由注册
- [daemon/tests](file:///d:/AIRP-Dev/engine/src/daemon) persona 分组：加 effective 端点测试

### WebUI

- [index.html](file:///d:/AIRP-Dev/webui/index.html)：persona-section 加绑定按钮行 + effective hint
- [app.js](file:///d:/AIRP-Dev/webui/app.js)：
  - DOM refs 加绑定按钮 + hint
  - `refreshPersonaList` 插入「自动」option
  - `refreshPersona` 处理「自动」选中的只读表单
  - character/session 切换 handler 加 effective 端点调用
  - 绑定/解绑按钮事件
  - `buildChatPayload` 传/不传 `persona_id`
- `target/` 下加 WebUI DOM 测试

## 9. 验收标准

1. 用户选中「自动」+ 发消息 → engine 日志/pipeline 用 binding 或 default persona
2. 用户选中具体 persona + 发消息 → engine 用该 persona（explicit）
3. 用户点「绑定到角色」→ 该角色下次「自动」时生效 persona 变为绑定的 persona
4. 用户点「解绑」→ 该角色回退到 default
5. character/session 切换 → effective hint 正确刷新
6. 无选中 character → 绑定按钮 disabled
7. engine 全套测试 + WebUI DOM 测试全绿
8. 2 个 subagent 不变式全绿
