# PR #78 独立审计报告

**审计源 LLM**：GLM-5.2
**审计日期**：2026-07-07
**审计范围**：Tauri Settings UI（commit 2453923）
**审计依据**：AGENTS.md 三条守则（独立审计 / 可提出自己的想法 / 质疑历史决策）

## 改动清单

| 文件 | 改动 |
|---|---|
| `ui/src-tauri/src/bus.rs` | +106 行：新增 `settings.get` / `settings.update` intent 分支 + `fetch_settings` / `update_settings` 函数 |
| `ui/src/widgets/SettingsModal.vue` | 新建：endpoint/api_key/model 表单 modal |
| `ui/src/App.vue` | +15 行：集成设置按钮 + modal + `w-settings` scope 初始化 |

## 审计方法

1. 完整读三个改动文件
2. 读 engine `/v1/settings` 端点实现（`engine/src/daemon/handlers.rs:69-130` + `engine/src/daemon/mod.rs:68-90`）确认 GET/POST 行为
3. 对比 webui 老 console 的 settings 处理（`webui/app.js:207-217`）
4. 对比现有 bus.rs 的 characters.list / characters.import 分支结构
5. 跑全量测试（372 engine + 9 Tauri + 97 vitest + 12 security + 24 markdown + 1 神圣不变式 + vue-tsc 0 errors）

## A 项（阻塞）

**无。**

## W 项（非阻塞，可后续）

### W-01：SettingsModal watch(visible) 的 `immediate: true` 导致 App mount 时发无谓请求

**位置**：`ui/src/widgets/SettingsModal.vue:69-75`

```ts
watch(
  () => props.visible,
  (v) => {
    if (v) refresh();
  },
  { immediate: true },  // ← 问题
);
```

**问题**：`App.vue` 中 `<SettingsModal v-if="isTauri" ...>`，组件在 App mount 时就实例化（visible=false）。`immediate: true` 会立刻触发 watch 回调，但 `v=false` 不进 if，所以**不会真的发请求** — 但 still 触发了一次无意义的回调执行。

实际副作用小（不发包），但 `immediate: true` 是不必要的，且让人误以为 mount 时会 refresh。建议移除 `immediate: true`。

**严重度**：低（信息级，无实际发包）

### W-02：bus.rs 新增 settings.get/update 分支无 emit 形状的单元测试

**位置**：`ui/src-tauri/src/bus.rs:305-372`

**问题**：现有 9 个测试中，`chat_turn_open_is_one_ordered_patch_envelope` 验证了 chat.send 的 emit 形状；但 settings.get/update 的 emit 形状（state set vs patch, scope name "w-settings", payload shape `{loaded, settings}` / `{saving}`）没有专门测试。

`dispatch_handles_intent_without_subscriber` 只验证不 panic，不验证 emit 形状。

**建议**：补一个测试，类似 `chat_turn_open_is_one_ordered_patch_envelope`，验证 settings.update 成功时 emit 的 envelope 形状（scope = "w-settings", op = Set, state 包含 `saving: false`）。

**严重度**：低（测试覆盖缺口，与 characters.list/import 同样缺测试，但本 PR 不应扩大范围）

### W-03：SettingsModal.vue 无单元测试

**位置**：`ui/src/widgets/SettingsModal.vue`

**问题**：SettingsModal.vue 没有对应的 vitest 测试文件。关键行为 `save()` 的"只传非空字段"逻辑（line 55-62）值得测：

```ts
function save(): void {
  const params: Record<string, string> = {};
  if (endpoint.value) params.endpoint = endpoint.value;
  if (model.value) params.model = model.value;
  if (apiKey.value) params.api_key = apiKey.value;  // ← 空 apiKey 不传 = 不修改
  emit("intent", "settings.update", params as unknown as Json);
}
```

这是与 engine 端 `update_settings` handler "空字符串视为未设置"逻辑（`engine/src/daemon/handlers.rs:98`）的双重保险，正确性关键。

**对比**：CharactersWidget.vue 也没有专门测试 — 项目惯例不强制 widget 单元测试。但 SettingsModal 的 save() 逻辑更关键（涉及 api_key 修改），建议补。

**严重度**：低（项目惯例问题，本 PR 不应扩大范围）

### W-04：bus.rs 顶部 module doc 未更新

**位置**：`ui/src-tauri/src/bus.rs:1-17`

**问题**：顶部 doc 说 "Phase 0 scope: `chat.send` intents are routed to the engine's `POST /v1/chat/completions` SSE endpoint... Other intents fall back to a minimal ack until later phases wire them."

新增 settings.get/update 后这段 doc 已过时 — 这些 intent 不再 "fall back to a minimal ack"。

**建议**：补充一句 "M4: `settings.get` / `settings.update` intents are routed to `GET/POST /v1/settings`."

**严重度**：低（文档失同步）

### W-05：saving=true 中间状态用 set 替换整个 scope，设计隐式

**位置**：`ui/src-tauri/src/bus.rs:339-344` + `ui/src/widgets/SettingsModal.vue:42-53`

**问题**：`settings.update` 分支先 emit `{ saving: true }`（set，全量替换 w-settings scope），这会覆盖之前的 `{loaded, settings}`。SettingsModal 的 watch 通过"只同步非空字段"避免表单被清空：

```ts
watch(() => state.value.settings, (s) => {
  if (s) {  // ← saving=true emit 后 s 是 undefined，不进 if
    if (s.endpoint) endpoint.value = s.endpoint;
    if (s.model) model.value = s.model;
  }
}, { immediate: true });
```

这个交互是**正确的**，但**隐式** — saving emit 的 set 替换 + watch 的"只同步非空字段"两者配合才能避免表单清空，单独看任一侧都不明显。

**建议**：在 bus.rs saving emit 处加注释说明这个交互，或在 SettingsModal watch 处加注释说明"依赖 set 替换 + 非空同步避免回填清空"。

**严重度**：低（设计正确但隐式，可读性改进）

## 信息级

### I-01：modal 关闭时未取消 inflight 请求

用户点保存 → saving=true → 立刻关 modal → 后台请求仍在跑 → 完成后 emit state set，但 modal 已卸载（v-if visible）。emit 是 best-effort（`let _ = app.emit(...)`），不会 panic。但用户重新打开 modal 时 watch(visible) 触发 refresh()，可能与 inflight update 竞态。影响：可能看到旧数据，不会 panic 或数据损坏。

### I-02：settings.update params 无 schema 验证

bus.rs `settings.update` 分支把 `i.params` 直接 forward 给 engine POST，没有 schema 验证。但 engine 端 `PartialAppConfig` serde 反序列化会忽略未知字段，且 IPC 调用者已是本机用户（能调 IPC 就能直接编辑 settings.json），**不是新增攻击面**。

## 设计审计

### D-01：modal vs blueprint widget 取舍（同意开发决策）

开发文档说"Settings 走 modal 而非 blueprint widget，因为 settings 是全局配置而非常驻 UI"。

**独立判断**：同意。Blueprint widget 适合常驻、与角色/session 关联的状态；Settings 是全局配置，modal 更合适。SettingsModal 不走 registry 注册，是 App.vue 直接 import 的硬编码 — 与 webui 老 console 的"硬编码 settings 面板"模式一致，合理简化。

### D-02：api_key 脱敏策略（同意开发决策）

engine GET 返回 `api_key_set: bool`（不返回真实 key），SettingsModal 不回填，留空 = 不修改。这避免了在 IPC / state store 中暴露真实 key。

**独立验证**：确认 engine `SettingsView::from_config`（`engine/src/daemon/mod.rs:84-85`）只输出 `api_key_set: bool`，不输出 `api_key` 字符串。bus.rs 接收的 settings JSON 不会包含真实 key。POST 时 api_key 是用户输入的明文，通过 IPC → bus.rs → HTTP POST 传给 engine — IPC 是同进程内消息传递，与 webui Bearer token 一样的暴露面。**api_key 不会回灌到 state store**（update 成功后 emit 的是 SettingsView，仍是脱敏的）。

✅ 安全设计正确。

### D-03：w-settings scope 不进 blueprint（同意开发决策）

w-settings scope 没有在 `MINIMAL_BLUEPRINT` 中声明，因为 SettingsModal 不是 blueprint 渲染的 widget，是 App.vue 直接控制的 modal。w-settings 只是数据通道。

**独立判断**：合理。Blueprint patch 机制不会触及 w-settings，只有 bus.rs 的 settings.get/update 会写 — 职责清晰。

## 历史决策质疑（守则 3）

### H-01：emit_state_set vs emit_state_patch 选择

现有所有非流式 state 更新都用 set（characters.list, characters.import, settings.get/update）。set 简单但覆盖并发更新；patch 复杂但并发安全。

**质疑**：settings 是否应该用 patch 避免 saving=true 覆盖 loaded/settings？

**结论**：当前 settings 是低并发场景（用户手动操作，不会同时 get 和 update），set 是合理的。W-05 的隐式交互是 set 模式的副作用，但通过 watch 设计已正确处理。**不修改**。

### H-02：bus.rs 用 RwLock<EngineConnection> 而非 ArcSwap

`engine: RwLock<EngineConnection>` 在每次 dispatch 都要 read lock + clone。`configure_engine` 是写锁，但很少调用。

**质疑**：用 `ArcSwap<EngineConnection>` 更高效（lock-free read）。

**结论**：这是历史决策，与本 PR 无关。RwLock 在当前吞吐下足够，且 `configure_engine` 几乎不调用。**不在本 PR 范围**。

## 测试结果

| Suite | 结果 |
|---|---|
| engine cargo test (workspace) | 372 pass, 1 ignored |
| Tauri ui cargo test | 9 pass |
| 神圣不变式 `subagent_context_has_no_orchestrator_noise` | 1/1 pass |
| webui security tests | 12/12 pass |
| markdown tests | 24/24 pass |
| UI vitest | 97/97 pass (13 files) |
| vue-tsc --noEmit | 0 errors |

## 结论

**推荐合并。**

- A 项（阻塞）：无
- W 项（非阻塞）：5 项，均为可后续改进，本 PR 不应扩大范围
- 设计合理，安全正确，测试覆盖与现有惯例一致
- 与 PLAN.md §0 "可双击运行并简单使用" 目标直接对齐 — 补齐了 Tauri shell 中缺失的 Settings 配置入口

## 后续 issue 建议（按 AGENTS.md 时序，PR 合并后提交）

- W-02 + W-03：合并为一条 "Tauri bus.rs settings intent + SettingsModal 单元测试覆盖" issue
- W-04：bus.rs module doc 更新（可与下次 bus.rs 改动一起做，不必单独 issue）
- W-01 + W-05：信息级，可在 W-02/W-03 issue 中附带处理
