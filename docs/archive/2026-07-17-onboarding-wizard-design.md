# Onboarding Wizard Design Spec（决策摘要）

> 日期：2026-07-17
>
> 交付状态：Design approved（sections ①-⑤）
>
> Issue：#209 — webui: implement first-run onboarding wizard（解阻 #207 目标 1 首聊黄金路径）
>
> 方案：方案 4（裁剪版）= 函数注入契约 + 动态 import 边界 + fail-open 降级
>
> 原始全文恢复：`git show 4ac03e3:docs/archive/2026-07-17-onboarding-wizard-design.md`

## 目标

不读开发文档、不打开 dev tools，完成部署健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮对话全闭环。目标用户：自带 provider key 的 RP 重度玩家。

## 关键决策

1. **架构：函数注入 Port 契约**：`onboarding.js` 零 import 宿主模块，通过 `mountOnboarding(container, hostPort)` 注入 6 成员 Port（version/mode/fetcher/formatError/onComplete/onSkip）。动态 import 边界替代 iframe，已完成引导的用户永远不加载向导代码。
2. **砍除项**：Shadow DOM 隔离 → #210；完整 Port 版本协商 → #211。
3. **6 阶段状态机**：健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮对话。
4. **first-run 检测**：标志 + 脱同步重触发（C 方案）。`airp_onboarded` + `airp_onboarding_skipped` 双标志区分"显式跳过"与"character 失效"。检测只用同步信号（localStorage），不发 HTTP。
5. **fail-open 降级**：import 失败（F1-F3）→ 退回手动流程；运行时崩溃（F4）→ 崩溃面板；HTTP/SSE 失败（F5/F6）→ 向导内重试，不降级。
6. **安全不变式**：api_key 永不作为 Port 成员、永不写浏览器存储、永不出现在 URL；GET settings 只返回 `api_key_set` 布尔。

## 不包含

- 不教 RP 基础概念
- 不替代开发工作台
- 不强制 Agent tool 调用
- 不引入真实浏览器自动化测试（首版用 node:test + HTTP 烟雾）
# Onboarding Wizard Design Spec

**Issue**: [#209](https://github.com/GhostXia/AIRP/issues/209) — webui: implement first-run onboarding wizard (解阻 #207 目标 1 首聊黄金路径)
**Date**: 2026-07-17
**Status**: Design approved (sections ①-⑤), pending spec review
**Approach**: 方案 4（裁剪版）= 函数注入契约 + 动态 import 边界 + fail-open 降级，砍除 Shadow DOM（#210）与完整版本协商（#211）

---

## 1. 目标与非目标

### 1.1 目标

解阻 [#207](https://github.com/GhostXia/AIRP/issues/207) 目标 1（首聊黄金路径）：在不读开发文档、不打开 dev tools 前提下，完成部署健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮对话全闭环。

目标用户画像：自带 provider key、带 PNG 角色卡 / `character_book` / preset JSON 的 RP 重度玩家，不是首次接触 RP 的新人。

### 1.2 非目标

- 不教 RP 基础概念（目标用户是重度玩家）；
- 不替代开发工作台（dev tools / workbench 仍保留给开发者，向导只走日用路径）；
- 不在向导中强制 Agent tool 调用（#207 目标 4 L1 价值证明通过场景证据库，不扭曲 onboarding）；
- 不引入 Shadow DOM 隔离（#210 推迟）；
- 不引入完整 Port 版本协商（#211 推迟）；
- 不引入真实浏览器自动化测试（首版用 node:test + HTTP 烟雾测试）。

---

## 2. 架构：方案 4（裁剪版）

### 2.1 核心结构

```text
app.js (宿主)
  ├─ first-run 检测（标志 + 脱同步重触发）← 留在宿主，小而稳定
  ├─ 命中时: const { mountOnboarding } = await import('./onboarding.js')
  ├─ 构造 hostPort 并 Object.freeze()
  └─ topbar effective config 指示器 ← 留在宿主（向导卸载后仍需存在）

onboarding.js (向导，零 import 自宿主代码)
  └─ export function mountOnboarding(container, hostPort)
       ├─ 挂载时校验 hostPort.version === 1，不匹配 fail fast
       ├─ 渲染进宿主给的 container
       └─ 出口只有两个: hostPort.onComplete(config) / hostPort.onSkip()
```

### 2.2 为什么选方案 4

| 优点 | 方案 4 如何拿到 |
|---|---|
| 可测、可 JSDoc 类型化的契约 | 函数注入，Port 是纯对象，单测 mock 成本最低 |
| auth 逻辑单一实现 | 注入 fetcher，向导侧零复制 |
| 零共享函数引用式解耦 | 模块边界替代事件总线：向导不 import 宿主任何东西，但调用是显式函数——拼写错误在挂载时炸掉而非静默失败 |
| 整块可替换 / 故障隔离 | 动态 import 边界替代 iframe：`onboarding.js` 语法错误或文件损坏只会让 `import()` reject，宿主 try/catch 后降级到现有手动配置路径 |
| 性能与爆炸半径 | 已完成引导的用户**永远不加载**向导代码 |

### 2.3 技术可行性查证

- `webui/serve.js:13` `.js` → `application/javascript; charset=utf-8`，合法 ESM MIME；生产用 Caddy `file_server` 同样映射 `.js` 为合法 ESM MIME。✅
- `webui/index.html:215-219` 所有脚本以 classic script 加载（无 `type="module"`），但 ES2020+ `import()` 动态导入在 classic script 中可用，目标浏览器为 evergreen。✅

### 2.4 砍除项追踪

- **Shadow DOM 隔离** → [#210](https://github.com/GhostXia/AIRP/issues/210)，触发条件：真实样式冲突 / 第三方组件库 / 独立设计系统 / 安全审计要求。
- **完整 Port 版本协商** → [#211](https://github.com/GhostXia/AIRP/issues/211)，触发条件：多消费者 / 热更新 A-B / 独立发版 / 审计要求矩阵。

---

## 3. Port 契约（Section ①）

### 3.1 Port 形状

```js
const hostPort = Object.freeze({
  version: 1,                    // 单整数断言；mountOnboarding 入口 if (hostPort.version !== 1) throw
  mode: 'production' | 'dev',    // 决定 Stage 1 是否渲染 engine URL/bearer 输入
  fetcher,                       // (path, opts) => Promise<Response>
  formatError,                   // (err) => { code, message, detail }
  onComplete,                    // (config) => void
  onSkip,                        // () => void
});
```

### 3.2 fetcher 行为（dev/prod 差异，auth 单一实现）

```js
// shared.js —— 宿主与 onboarding.js 共享，但 onboarding.js 通过 Port 注入拿到，不直接 import
function makeFetcher(mode) {
  return async function fetcher(path, opts = {}) {
    let base, bearer;
    if (mode === 'production') {
      base = window.location.origin;     // 同源，网关注入 Authorization
      bearer = '';                        // 浏览器永不持有 access key
    } else {
      // dev: 每次调用读 sessionStorage（向导 Stage 1 写入后立即生效）
      base = sessionStorage.getItem('airp_engine_url') || 'http://127.0.0.1:8000';
      bearer = sessionStorage.getItem('airp_bearer') || '';
    }
    const headers = { ...(opts.headers || {}) };
    if (bearer) headers['Authorization'] = `Bearer ${bearer}`;
    const res = await fetch(base + path, { ...opts, headers });
    return res;
  };
}
```

dev 模式下向导 Stage 1 收集 engine URL + bearer 后，**直接写 sessionStorage**（key 名 `airp_engine_url`/`airp_bearer`，与现有 `app.js:211-216` 约定一致）。fetcher 每次调用读 sessionStorage，无需 Port 提供 setter——**Port 维持 6 成员**。这是 dev-only 便利路径；生产模式向导永不触碰 sessionStorage。

### 3.3 formatError

提取现有 `app.js:176-200` 到 shared.js，签名 `(err) => { code, message, detail }`。向导用它把 engine 返回的类型化错误（`invalid_endpoint`/`upstream_timeout`/`upstream_status` 等）转成可行动 UI 文案。

### 3.4 onComplete(config) config 形状

```js
{
  provider: 'openai' | 'anthropic' | ...,   // 来自 Stage 2 保存的 settings
  model: string,                              // 来自 Stage 3 picker
  character_id: string,                       // 来自 Stage 4 导入
  persona_id: string,                         // 来自 Stage 5（'default' 或新建）
  preset_id: string | null,                   // 来自 Stage 5（null = 不使用预设）
  user_id: string,                            // 固定 'default'（首发单用户）
  firstChatCompleted: boolean,                // Stage 6 是否完成首聊（区分"完成向导"与"完成首聊"）
}
```

宿主 `onComplete` 职责：写 `localStorage.airp_onboarded=true` + `airp_character_id`/`airp_persona_id`/`airp_preset_id` + 卸载向导 + 渲染 topbar effective config 指示器 + 调用 `refreshAll()`。

### 3.5 onSkip

宿主 `onSkip` 职责：写 `localStorage.airp_onboarded=true`（**跳过也标记**，避免每次加载打扰；脱同步重触发仍生效）+ 卸载向导 + 渲染 topbar 指示器（显示当前已知配置，可能部分为空）+ `refreshAll()`。

### 3.6 Port 不变量（审计可校验）

1. `onboarding.js` 文件内零 `import` 语句指向宿主模块（`app.js`/`shared.js`）；仅依赖浏览器 API + Port 注入。
2. 向导对宿主的全部知识 = Port 对象；宿主对向导的全部知识 = `mountOnboarding(container, hostPort)` 签名。
3. Port 成员新增需 PR 论证 + 必须为 stateless/纯函数（fetcher 闭包例外）。
4. `api_key` 永不作为 Port 成员、永不写入浏览器存储、永不出现在 URL/query。

---

## 4. 6 阶段状态机与数据流（Section ②）

### 4.1 状态机总览

```text
[Stage 1: 部署健康检查]
   ├─ dev 模式: 收集 engine URL + bearer → 写 sessionStorage → GET /version + /health
   ├─ prod 模式: GET /version + /health（同源，网关注入 auth）
   ├─ 失败 → 显示可行动错误（formatError）+ 重试按钮；不进 Stage 2
   └─ 成功（health.engine=ok）→ Stage 2
        ↓
[Stage 2: provider 配置]
   ├─ GET /v1/settings → 回填 provider/endpoint/model（api_key 字段空，仅显示 api_key_set 布尔）
   ├─ 用户填 provider + endpoint + api_key + 可选 model（自由文本，可留空待 Stage 3）
   ├─ POST /v1/settings（api_key 仅在用户填了非空值时携带；空字符串=不修改）
   ├─ 失败（如 prod 模式改 access_api_key → BadRequest）→ formatError + 停留 Stage 2
   └─ 成功 → Stage 3
        ↓
[Stage 3: 模型验证]
   ├─ GET /v1/models（拉取上游模型列表）
   ├─ 失败（invalid_endpoint / upstream_timeout / upstream_status 等）→ formatError + 回 Stage 2 修 endpoint
   ├─ 成功 → 渲染 model picker（<select> 从 /v1/models 的 {id} 数组构建）
   ├─ 用户选 model → POST /v1/settings { model } 保存
   └─ 成功 → Stage 4
        ↓
[Stage 4: 角色导入]
   ├─ 文件选择（PNG 角色卡 / character_book JSON / preset JSON）
   ├─ 客户端校验：PNG magic bytes / JSON 解析 / 10 MiB 字节计数（复用 app.js:1807-1817 逻辑）
   ├─ POST /v1/characters/import { card_png_base64 | card_json }（生产模式 card_path 被拒）
   ├─ 失败 → formatError + 停留 Stage 4
   ├─ 成功 → 返回 character_id
   └─ GET /v1/characters 确认导入可见 → Stage 5
        ↓
[Stage 5: Persona/Preset 选择]
   ├─ GET /v1/users/default/personas → 渲染 picker（含 'default' 选项 + 新建按钮）
   ├─ GET /v1/presets → 渲染 picker（含 '不使用预设' 选项 + JSON 导入按钮）
   ├─ 可选：POST /v1/chat/preview（传 character_id + persona_id + preset_id）→ 显示 effective config 摘要（card/persona/lorebook/state/preset/scene/memory/history/user 来源可见，对齐 §2.4 L0）
   ├─ 用户选 persona + preset（或保持 default + 不使用）
   └─ 成功 → Stage 6
        ↓
[Stage 6: 首轮对话]
   ├─ POST /v1/sessions/:character_id（懒创建 session）
   ├─ POST /v1/chat/completions（SSE 流式）→ 显示流式回复
   ├─ 失败 → formatError + 重试；session 已创建则保留
   └─ 成功（收到 done chunk）→ onComplete(config)
```

### 4.2 数据流表

| Stage | fetcher 调用 | 向导内部 state | 出口条件 |
|---|---|---|---|
| 1 | `GET /version` + `GET /health` | `{ mode, engineVersion, providerConfigured, dataRootWritable }` | `health.engine === 'ok'` |
| 2 | `GET /v1/settings` → `POST /v1/settings` | `{ provider, endpoint, model?, apiKeySet }` | POST 200 |
| 3 | `GET /v1/models` → `POST /v1/settings {model}` | `{ models: string[], selectedModel }` | POST 200 + model 非空 |
| 4 | `POST /v1/characters/import` → `GET /v1/characters` | `{ characterId, characterName }` | import 200 + list 包含该 ID |
| 5 | `GET /v1/users/default/personas` + `GET /v1/presets` + 可选 `POST /v1/chat/preview` | `{ personaId, presetId, effectiveConfig? }` | 用户确认选择 |
| 6 | `POST /v1/sessions/:cid` → `POST /v1/chat/completions`（SSE） | `{ sessionId, firstMessage, replyReceived }` | 收到 `done` chunk |

### 4.3 关键设计决策

**Stage 2 的 api_key 处理**（安全敏感）：
- 向导表单有 `api_key` 输入框，但**永不预填**（GET /v1/settings 只返回 `api_key_set` 布尔，不返回值）。
- 用户填了非空值 → POST 携带 `api_key`；用户留空 → POST 不携带 `api_key` 字段（空字符串=不修改）。
- POST 成功后**立即清空输入框值**（复用 `app.js:405` 现有行为），api_key 永不进入向导内部 state、永不写 sessionStorage/localStorage、永不出现在 URL。
- 输入框 `type="password"` + `autocomplete="off"`。
- **编码格式**（gemini id=3602733458）：`api_key` 以 UTF-8 字符串形式放入 JSON body 的 `api_key` 字段，由 `JSON.stringify` 自动处理；不做 base64 / form-encoding。Engine 侧 `PartialAppConfig::api_key: Option<String>` 直接反序列化。这与 engine `application/json` Content-Type 约定一致，无需额外的编码层。

**Stage 3 model picker**：
- 从 `/v1/models` 的 `{id}` 数组构建 `<select>`，用户点选而非自由输入。
- 保留"手动输入"回退选项（上游 `/models` 端点异常或返回空时降级为自由文本）——避免 picker 成为 Stage 3 单点故障。

**Stage 4 文件类型**：
- PNG 角色卡：检测 magic bytes（`89 50 4E 47 0D 0A 1A 0A`），base64 编码，POST `{ card_png_base64 }` 到 `/v1/characters/import`。成功响应 `{ character_id }`，向导写入 `state.characterId` / `state.characterName`，进入 Stage 5。
- character_book JSON：解析后含 `spec: 'chara_card_v2'` 或 `data.book`/`character_book` 字段 → POST `{ card_json }` 到 `/v1/characters/import`。响应同上。
- preset JSON（CodeRabbit id=3602743407 修正）：解析后含 `prompts` 数组（SillyTimizer preset 形状）→ POST `{ preset_id, preset_json }` 到 `/v1/presets/import`。响应 `{ preset_id }`，向导写入 `state.presetId`，**跳过角色导入逻辑**直接进入 Stage 5（用户可能在 Stage 5 选择已有角色或导入角色卡）。若 preset JSON 解析成功但 `prompts` 字段缺失，按 character_book JSON 处理；都不匹配则显示 "无法识别的 JSON 文件" 错误并停留 Stage 4。
- 单次只导入一个资产；多资产导入是未来增强，不在首版。

**Stage 5 effective config 预览**：
- `POST /v1/chat/preview` 是只读、不创建 session、不推进 timeline、不返回 prompt body 或 secrets——安全用于向导展示。
- 预览显示来源标签（card/persona/lorebook/state/preset/scene/memory/history/user），让用户在首聊前确认 effective config 符合预期。直接满足 #209"effective config 必须保持可见"在向导内的体现。

**Stage 6 SSE 流式**：
- 复用现有 SSE 解析逻辑（`app.js:1278-1362` 的 `doSend`），但走 Port.fetcher（dev 模式 auth 注入）。
- 流式 chunk 类型：`body_chunk`/`think_chunk`/`plan`/`tool_call`/`tool_result`/`done`。向导显示 `body_chunk` 内容，`done` 即出口。
- **不强制 Agent tool 调用**（对齐 §2.4 L1"不强制嵌入首轮对话"）——若 SSE 流中自然出现 `tool_call`/`tool_result` 则展示，但不作为通过条件。
- **SSE 终止标记**（CodeRabbit id=3602743427 修正）：`reader.read()` done=true 不等于流正常完成；只有 `[DONE]` sentinel 或 `done` chunk 才算正常出口。提前 EOF（reader done 但未见 sentinel）→ 显示中断错误，保留 sessionId 供重试复用。
- **单飞保护**（spec §4.3 Stage 6 修正）：`sendFirstMessage` 用 `sendInFlight` 标志位防止双击/重试并发；进入即置位，finally 释放。重试时复用 `state.sessionId`，避免孤儿会话。
- **生产模式 SSE 反代头**（gemini id=3602733448）：Caddy / nginx 反代必须禁用缓冲并显式传递 `Accept: text/event-stream`。Caddy 配置示例：`reverse_proxy localhost:8000 { flush_interval -1 }`；nginx 配置示例：`proxy_buffering off; proxy_cache off; chunked_transfer_encoding on;`。否则流式 chunk 会被反代缓冲到流结束才一次性下发，破坏 UX。dev 模式直连 engine 不受此影响。

### 4.4 向导内 navigation

- 每阶段有"上一步"按钮（除 Stage 1）+ "跳过向导"按钮（任意阶段可跳，触发 `onSkip`）。
- Stage 4 角色导入可跳过（用户可能已有角色）——但跳过后 Stage 6 首聊需要选已有角色，向导内补一步"选择已有角色"picker。
- Stage 5 persona/preset 可跳过（保持 default + 不使用预设）。
- Stage 6 不可通过 `onSkip` 跳过（首聊是 #207 黄金路径的最终阶段）；但用户可“完成向导，稍后聊天”——这算 `onComplete` 出口（config 中 `firstChatCompleted: false`），验收记录区分“完成向导”与“完成首聊”。

---

## 5. first-run 检测与脱同步重触发（Section ③）

### 5.1 检测入口

宿主 `app.js` 在现有 `scheduleAutoConnect()`（`app.js:2836`，300ms 延迟）**之前**插入 onboarding 检测门：

```text
页面加载
  ↓
读取 window.AIRP_WEBUI_CONFIG.mode
  ↓
调用 shouldShowOnboarding()        ← 新增，宿主侧纯函数
  ├─ 返回 false → 现有 scheduleAutoConnect() + refreshAll()（不变）
  └─ 返回 true  → 调用 mountOnboarding()，跳过 auto-connect（向导 Stage 1 自己接管连接）
```

### 5.2 shouldShowOnboarding() 逻辑（C 方案：标志 + 脱同步重触发）

```js
function shouldShowOnboarding() {
  const onboarded = localStorage.getItem('airp_onboarded') === 'true';
  const skipped = localStorage.getItem('airp_onboarding_skipped') === 'true';

  if (!onboarded) {
    return true;                    // 从未完成向导 → 触发
  }

  // 已标记 onboarded —— 脱同步检测
  // 用同步可得的信号快速判断；异步 /health 检查在 mountOnboarding 内做（避免阻塞检测）
  // 用户显式跳过向导（skipped=true）→ 不再因 character_id 缺失重触发
  //   （"skip 也写 onboarded=true" 是"别再烦我"的偏好，不是"配置完整"的断言）
  if (skipped) return false;

  // 完成向导但 character_id 失效（外部删除）→ 重新触发
  const hasCharacter = localStorage.getItem('airp_character_id') !== null;
  if (!hasCharacter) {
    return true;                    // 标志说已 onboard 但无 character → 重新触发
  }
  return false;
}
```

**关键设计**：脱同步检测**只用同步信号**（localStorage），不发 HTTP 请求。原因：
1. 检测函数在页面加载关键路径，必须快；
2. 异步 `/health` 检查会让 UI 闪烁（先显示主界面再弹向导）；
3. 真正的 provider 状态脱同步在向导 Stage 1 内处理——向导渲染后第一件事就是 `GET /health`，若 `provider_configured=false` 则提示用户"检测到配置丢失，重新配置 provider"。

**skip 标志的语义**（CodeRabbit id=3602743411 修正）：`airp_onboarding_skipped=true` 表示用户**显式跳过向导**（onSkip 写入，onComplete 清除）。这与"完成向导但 character 失效"区分——后者应重新触发以恢复，前者不应再烦用户。这是对原 spec 的修正：原 spec §5.5 的 `onSkip` 只写 `airp_onboarded=true`，导致跳过后下次加载因 character_id 缺失而重触发，违反"skip 也标记 onboarded 以避免每次加载打扰"的设计意图。

### 5.3 脱同步场景覆盖

| 场景 | 检测点 | 行为 |
|---|---|---|
| 用户清 localStorage | `airp_onboarded` 不存在 | 重新触发向导 |
| 用户手动删 settings.json | 向导 Stage 1 `GET /health` 返回 `provider_configured=false` | 向导内提示"provider 配置丢失"，从 Stage 2 恢复 |
| 用户手动删 character 目录 | `shouldShowOnboarding` 看 `airp_character_id` 仍存在 → 不触发；但 Stage 6 首聊时 `GET /v1/characters/:id` 404 | 主界面用现有错误处理（非向导职责） |
| 用户手动改 settings.json 加 provider | `airp_onboarded=true` + `airp_character_id` 存在 → 不触发 | 主界面正常加载（合理：用户已知道在做什么） |
| 向导未完成用户关页面 | `airp_onboarded` 未写 | 下次加载重新触发向导 |

**故意不覆盖的场景**：character 目录被外部删除但 `airp_character_id` 仍指向失效 ID。这是主界面运行时问题，不应由向导检测——向导的职责是首配置，不是运行时完整性守护（那是 P2 §2.3 恢复判据的职责）。

### 5.4 mountOnboarding 调用点（宿主侧）

```js
// app.js，替换原 scheduleAutoConnect() 调用
async function bootstrapApp() {
  if (shouldShowOnboarding()) {
    const container = document.getElementById('onboarding-root');
    try {
      const { mountOnboarding } = await import('./onboarding.js');
      const hostPort = Object.freeze({
        version: 1,
        mode: productionMode ? 'production' : 'dev',
        fetcher: makeFetcher(productionMode ? 'production' : 'dev'),
        formatError,
        onComplete,
        onSkip,
      });
      // CodeRabbit id=3602743414 修正：必须保存 cleanup 返回值，
      // 否则 onComplete/onSkip 无法调用 unmountOnboarding，导致 listener 泄漏。
      onboardingCleanup = mountOnboarding(container, hostPort);
      // 不调用 scheduleAutoConnect —— 向导 Stage 1 接管
    } catch (err) {
      console.error('[onboarding] load failed, falling back to manual flow:', err);
      container.hidden = true;
      scheduleAutoConnect();        // 降级：向导加载失败 → 现有手动流程
    }
  } else {
    // CodeRabbit id=3602747746 修正：已 onboard 用户刷新后指示器消失，
    // 必须显式调用 renderEffectiveConfigIndicator 恢复 topbar 配置摘要。
    renderEffectiveConfigIndicator(loadKnownConfig());
    scheduleAutoConnect();          // 已 onboard → 现有流程
  }
}
bootstrapApp();
```

### 5.5 onComplete / onSkip 宿主实现

```js
function onComplete(config) {
  localStorage.setItem('airp_onboarded', 'true');
  // 完成向导后清除 skipped 标志（之前可能跳过过，现在已正式完成）
  localStorage.removeItem('airp_onboarding_skipped');
  localStorage.setItem('airp_character_id', config.character_id);
  localStorage.setItem('airp_persona_id', config.persona_id);
  // CodeRabbit id=3602743418 修正：preset_id null/空 → 移除旧值，避免残留
  if (config.preset_id) {
    localStorage.setItem('airp_preset_id', config.preset_id);
  } else {
    localStorage.removeItem('airp_preset_id');
  }
  // user_id 固定 'default'（首发单用户），与现有 app.js:456-468 约定一致
  unmountOnboarding();
  renderEffectiveConfigIndicator(config);   // topbar 常驻指示器
  scheduleAutoConnect();                    // 向导已完成连接，auto-connect 直接 refreshAll
}

function onSkip() {
  localStorage.setItem('airp_onboarded', 'true');
  // skipped 标志（spec §5.3 修正）：区分"显式跳过"与"完成向导但 character 失效"
  localStorage.setItem('airp_onboarding_skipped', 'true');
  unmountOnboarding();
  renderEffectiveConfigIndicator(loadKnownConfig());  // 显示当前已知配置（可能部分为空）
  scheduleAutoConnect();
}
```

**skip 也写 `airp_onboarded=true`**：避免每次加载打扰已配置用户。脱同步检测仍保护——但需通过 `airp_onboarding_skipped` 标志区分"显式跳过"与"完成向导但 character 失效"（spec §5.2 修正）。这是 C 方案的核心——标志是"别再烦我"的偏好，不是"配置完整"的断言。

### 5.6 unmountOnboarding

向导 `mountOnboarding` 返回一个 `cleanup()` 函数（Port 之外的第二返回值）：

```js
// onboarding.js
export function mountOnboarding(container, hostPort) {
  if (hostPort.version !== 1) throw new Error('onboarding: hostPort.version must be 1');
  // ... 渲染逻辑 ...
  return function cleanup() {
    container.innerHTML = '';       // 普通 container，无 shadow root（#210 砍除项）
    // 移除向导注册的事件监听（向导内记录注册的 listeners，cleanup 时遍历移除）
  };
}
```

宿主 `unmountOnboarding()` 调用保存的 cleanup 引用。

### 5.7 与现有 scheduleAutoConnect 的协调

| 路径 | scheduleAutoConnect 是否调用 | 说明 |
|---|---|---|
| 首次/脱同步触发向导 | 否（向导 Stage 1 接管） | 向导内完成 GET /version + /health |
| onComplete | 是 | 向导已完成连接，auto-connect 跑 refreshAll 同步 UI |
| onSkip | 是 | 用户跳过，主界面需要自己连接 |
| 向导 import 失败降级 | 是 | 退回现有手动流程 |
| 已 onboarded | 是 | 现有行为不变 |

### 5.8 topbar effective config 指示器

向导完成后常驻 topbar（与 `#production-connection` 同区，但功能不同）：

```text
[provider: openai] [model: gpt-4o] [character: Alice] [persona: default] [preset: none]  [⚙ 重新配置]
```

- 点击任一标签 → 弹出该资源的简要详情（provider endpoint host、model id、character name、persona name、preset name）。
- `[⚙ 重新配置]` → 清 `airp_onboarded` 标志并刷新页面，重新触发向导（用户主动要求重走，非脱同步）。
- **不显示 api_key / access_api_key / engine URL / bearer**（安全：这些信息不应在 UI 常驻）。
- 生产模式下与 `#production-connection` 并存（后者是"同源安全连接"静态标签，前者是配置摘要）。

---

## 6. import 失败降级路径（Section ④）

### 6.1 设计原则

向导是**纯增强层（progressive enhancement）**，不是启动路径上的单点故障。`import('./onboarding.js')` 失败时，宿主必须 fail-open 到现有手动配置流程，主应用启动不受影响。这与项目"fail-closed 只用于安全边界、可用性路径应可降级"哲学一致。

### 6.2 失败分类与降级矩阵

| 失败类型 | 触发 | 降级行为 | 用户可见 |
|---|---|---|---|
| **F1: 模块加载失败** | `import()` reject（404 / 网络错误 / CSP 拒绝） | catch → `scheduleAutoConnect()` + `refreshAll()` | topbar 显示 toast "向导加载失败，已退回手动配置（详见控制台）" |
| **F2: 模块语法错误** | `import()` resolve 但模块顶层 throw（语法错误 / 依赖缺失） | 同 F1 | 同 F1 |
| **F3: Port 版本不匹配** | `mountOnboarding` 内 `if (hostPort.version !== 1) throw` | catch → 同 F1（错误信息含 "hostPort.version must be 1"） | 同 F1，控制台日志含具体版本号 |
| **F4: 向导运行时崩溃** | 向导渲染后某阶段 throw（未预期错误） | 向导内 try/catch 捕获 → 显示"向导遇到问题"+ 两个按钮：[重试向导] / [退回手动配置] | 向导内错误面板，不弹全局 toast |
| **F5: 向导内 fetcher 调用失败** | Stage 1-6 的 HTTP 请求失败 | **不降级到手动流程**——向导内 formatError 显示可行动错误 + 重试按钮 | 向导内阶段错误，停留当前 Stage |
| **F6: 向导内 SSE 流中断** | Stage 6 流式回复中断 | 向导内提示"回复中断"+ [重试] / [完成向导（首聊未完成）] | 向导内 Stage 6 错误 |

### 6.3 降级路径伪代码（宿主侧）

```js
async function bootstrapApp() {
  if (!shouldShowOnboarding()) {
    scheduleAutoConnect();
    return;
  }

  try {
    const { mountOnboarding } = await import('./onboarding.js');   // F1/F2 在此 reject
    const cleanup = mountOnboarding(
      document.getElementById('onboarding-root'),
      Object.freeze({ version: 1, mode, fetcher, formatError, onComplete, onSkip })  // F3 在此 throw
    );
    pendingCleanup = cleanup;                                       // 保存以便 onComplete/onSkip 调用
  } catch (err) {
    console.error('[onboarding] load failed, falling back to manual flow:', err);
    scheduleAutoConnect();                                          // F1/F2/F3 降级
    showToast('向导加载失败，已退回手动配置（详见控制台）', { type: 'warning' });
  }
}
```

### 6.4 向导内 F4 运行时崩溃处理

向导 `mountOnboarding` 用两层保护：

1. **顶层 `render()` try/catch**：捕获 `renderStage()` 同步渲染异常 → `renderCrashFallback`。
2. **`safeSync(fn, label)` / `safeAsync(fn, label)` 包装**（CodeRabbit id=3602743427 修正）：所有 event handler 与 async continuation 都被包装，异常统一路由到 `renderCrashFallback`，而不是变成浏览器 uncaught error。

```js
function safeSync(fn, label) {
  return function (...args) {
    try { return fn.apply(this, args); }
    catch (err) {
      console.error('[onboarding] handler crashed (' + label + '):', err);
      try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
    }
  };
}
async function safeAsync(fn, label) {
  try { return await fn(); }
  catch (err) {
    console.error('[onboarding] async crashed (' + label + '):', err);
    try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
  }
}

function render() {
  try { renderStage(); }
  catch (err) {
    console.error('[onboarding] runtime crash:', err);
    renderCrashFallback(err);
  }
}

function renderCrashFallback(err) {
  // CodeRabbit id=3602743432 修正：retry 不可残留旧 listener。
  // 先 removeListeners() 再清 DOM，否则 retry 后旧 handler 仍在 + 重复注册新 handler。
  removeListeners();
  if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
  sendInFlight = false;
  container.innerHTML = '';
  // 渲染崩溃面板：[重试向导] → 重置 state + render()；[退回手动配置] → hostPort.onSkip()
  // ...
}
```

**关键约束**：
- **retry 必须先清 listener**（CodeRabbit id=3602743432）：原 spec 的 retry 路径只清了 `container.innerHTML`，但旧 addEventListener 仍持有闭包引用，retry 后会重复注册。修正后的 `renderCrashFallback` 在清 DOM 前先 `removeListeners()`。
- **F4 的"退回手动配置"通过 `hostPort.onSkip()` 实现**——宿主 `onSkip` 会写 `airp_onboarded=true` + `airp_onboarding_skipped=true`（spec §5.5 修正）+ `scheduleAutoConnect()`。这意味着用户从崩溃退出后**不会下次加载又被向导拦截**（即使 character_id 缺失，skipped 标志也会阻止重触发）。这是有意的：崩溃的向导不应无限循环拦截用户。
- **safeSync 不向调用方抛错**：DOM addEventListener 注册的 handler 是 safeSync 包装后的函数，内部 fn 抛错由 safeSync catch，不会传播到浏览器 uncaught error 通道。

### 6.5 F5/F6 不降级的设计理由

Stage 1-6 的 HTTP 失败（F5）和 SSE 中断（F6）**不触发降级到手动流程**，而是向导内重试。理由：
1. 用户已在向导中投入配置工作（填了 provider、导入了角色），降级会丢失上下文；
2. 向导内错误信息更精准（知道在哪个 Stage 失败），手动流程的错误信息更泛；
3. 降级应 reserved for "向导本身坏了"，不是"上游服务暂时不可用"。

### 6.6 F1-F4 vs F5-F6 的边界

```text
向导能否加载并渲染？
├─ 否 (F1/F2/F3) → 降级到手动流程（宿主 catch）
└─ 是
    └─ 向导运行中是否崩溃？
       ├─ 是 (F4) → 向导内崩溃面板 + [重试向导] / [退回手动配置]
       └─ 否
          └─ HTTP/SSE 请求是否失败？
             ├─ 是 (F5/F6) → 向导内阶段错误 + 重试（不降级）
             └─ 否 → 正常流程
```

### 6.7 降级后的状态一致性

降级到手动流程时，宿主**不写 `airp_onboarded` 标志**（只在 `onComplete`/`onSkip` 写）。这意味着：
- F1/F2/F3 降级后，下次页面加载会再次尝试向导（如果向导文件已修复）。
- F4 用户选"退回手动配置"会走 `onSkip` → 写标志 → 下次不触发向导。
- 这个区分是合理的：F1-F3 是"向导根本没起来"，下次应该重试；F4 是"用户主动放弃向导"，下次应该尊重。

---

## 7. 测试策略（Section ⑤）

### 7.1 测试分层总览

| 层 | 范围 | 工具 | 数量目标 |
|---|---|---|---|
| **L1 单元测试** | `onboarding.js` 纯函数 + mock Port | 现有 `webui/tests/*.test.mjs`（node:test + assert） | ~25-35 用例 |
| **L2 集成测试** | 向导 + mock fetcher 走完 6 阶段 | node:test + jsdom 或纯 DOM mock | ~8-12 用例 |
| **L3 烟雾测试** | 真实 engine + 真实 HTTP | 扩展现有 `webui/smoke.mjs` | ~6 用例（每阶段 1 个 happy path） |
| **L4 现有回归** | app.js 主流程不被向导破坏 | 现有 `assembly.test.mjs`/`lorebook.test.mjs`/`persona.test.mjs` 全绿 | 不新增，确保不退化 |

### 7.2 L1 单元测试（onboarding.js 内部）

放在 `webui/tests/onboarding.test.mjs`，复用现有测试约定（参考 `assembly.test.mjs` 结构）。

**mock Port 策略**：
```js
function makeMockPort(overrides = {}) {
  const calls = { fetch: [], onComplete: [], onSkip: [] };
  return {
    port: Object.freeze({
      version: 1,
      mode: 'dev',
      fetcher: async (path, opts) => {
        calls.fetch.push({ path, opts });
        return mockRouter(path, opts);          // 按 path 返回预设 Response
      },
      formatError: (err) => ({ code: err.code || 'unknown', message: err.message, detail: err.detail }),
      onComplete: (config) => calls.onComplete.push(config),
      onSkip: () => calls.onSkip.push(null),
      ...overrides,
    }),
    calls,
  };
}
```

**测试用例分组**：

| 组 | 用例 | 验证点 |
|---|---|---|
| **Port 契约** | `version !== 1 throws` | F3 降级入口 |
| | `missing required member throws` | Port 不变量 1-3 |
| **Stage 1** | `dev 模式渲染 engine URL+bearer 输入` | mode 分支 |
| | `prod 模式隐藏 engine URL+bearer 输入` | mode 分支 |
| | `health.engine !== 'ok' 阻塞进 Stage 2` | 出口条件 |
| | `engine URL 写入 sessionStorage` | dev fetcher 契约 |
| **Stage 2** | `api_key 空字符串不携带` | 安全不变量 |
| | `api_key 非空携带且输入框清空` | 安全不变量 |
| | `POST 失败停留 Stage 2` | F5 不降级 |
| **Stage 3** | `/v1/models 返回空 → 降级自由文本输入` | picker 降级 |
| | `model 选中后 POST /v1/settings` | 出口条件 |
| **Stage 4** | `PNG magic bytes 检测` | 客户端校验 |
| | `JSON 解析失败显示错误` | 客户端校验 |
| | `10 MiB 字节计数拒绝` | body limit |
| | `生产模式 card_path 被拒（不发送）` | 生产安全 |
| **Stage 5** | `persona picker 含 'default'` | PERSONA-API 约定 |
| | `preset '不使用预设' = null` | config 形状 |
| | `chat/preview 显示来源标签` | §2.4 L0 trace |
| **Stage 6** | `SSE done chunk 触发 onComplete` | 出口条件 |
| | `SSE 中断显示重试` | F6 |
| | `tool_call/tool_result 展示但不作通过条件` | §2.4 L1 不扭曲 |
| **navigation** | `每阶段'上一步'可回退` | UX |
| | `Stage 4 跳过后补'选择已有角色'` | 跳过语义 |
| | `任意阶段'跳过向导'触发 onSkip` | 跳过语义 |
| **cleanup** | `cleanup 移除所有事件监听` | 无泄漏 |
| | `cleanup 清空 container.innerHTML` | 无残留 |

### 7.3 L2 集成测试（向导 + mock fetcher 走完 6 阶段）

不走真实 engine，但走真实 `mountOnboarding` 渲染逻辑。DOM 环境用最小 mock（不引入完整 jsdom，保持零依赖约定——参考现有 `assembly.test.mjs` 是否用 jsdom）。

**用例**：
1. `happy_path_dev`：dev 模式从 Stage 1 走到 Stage 6 onComplete，断言 config 形状 + sessionStorage 写入 + calls.onComplete 调用一次。
2. `happy_path_prod`：prod 模式从 Stage 1（无 engine URL 输入）走到 Stage 6，断言无 sessionStorage 写入。
3. `skip_at_stage_3`：Stage 3 点"跳过向导"，断言 calls.onSkip + 不调用 onComplete。
4. `stage_4_skip_then_select_existing`：Stage 4 跳过 → 选已有角色 → Stage 5。
5. `f4_crash_recovery`：mock renderStage throw → 崩溃面板 → 点[重试] → 恢复。
6. `f4_crash_exit_to_manual`：mock renderStage throw → 点[退回手动配置] → onSkip 调用。
7. `back_navigation`：Stage 3 点"上一步" → 回 Stage 2，state 保留。
8. `api_key_never_in_state`：全流程后断言向导 state 对象无 api_key 字段。

### 7.4 L3 烟雾测试（扩展 smoke.mjs）

现有 `smoke.mjs` 是 engine-truth HTTP/SSE 测试（非浏览器自动化）。扩展 6 个用例验证向导调用的端点契约：

| 用例 | 验证 |
|---|---|
| `smoke_onboarding_health` | `GET /health` 返回 `provider_configured` + `data_root_writable` 字段 |
| `smoke_onboarding_settings_get` | `GET /v1/settings` 返回 `api_key_set` 布尔，不返回 api_key 值 |
| `smoke_onboarding_settings_post` | `POST /v1/settings` 空 api_key 不修改、非空修改且不持久化 |
| `smoke_onboarding_models` | `GET /v1/models` 返回 `{id}` 数组或类型化错误 |
| `smoke_onboarding_character_import` | `POST /v1/characters/import` PNG base64 + JSON 双路径 |
| `smoke_onboarding_chat_preview` | `POST /v1/chat/preview` 返回来源标签、不返回 prompt body |

**不在 L3 覆盖**：SSE 流式首聊（`smoke.mjs` 已有 SSE 测试，向导复用相同端点，不重复）。

### 7.5 L4 现有回归守护

不新增测试，但 PR CI 必须全绿：
- `webui/tests/assembly.test.mjs`
- `webui/tests/lorebook.test.mjs`
- `webui/tests/persona.test.mjs`
- `webui/smoke.mjs`（含新增 L3 用例）

注：markdown 渲染和 security 不变式由 engine 侧 Rust 测试守护（`engine/src/daemon/tests/security.rs`、`engine/src/data_dir/security.rs` 等），本 PR 不触及 engine 代码，无影响。

### 7.6 测试不覆盖（显式声明）

1. **真实浏览器自动化**（Playwright/Puppeteer）：首版未引入；它会改变现有 node:test + HTTP 测试基建，但属于 #207 首聊黄金路径仍需继续补齐的工程验收能力。
2. **CSP 违规检测**：依赖现有 system-Chrome smoke（`docs/WEBUI-PRODUCTION-ARCHITECTURE.md` §4 提到的 `securitypolicyviolation` 监听），不新增。
3. **跨浏览器兼容**：evergreen 浏览器假设，不测 IE/旧 Edge。

### 7.7 测试与 #207 验收标准的关系

| #207 验收 | 测试层覆盖 |
|---|---|
| 6 阶段全闭环 | L2 happy_path_dev/prod |
| 不读开发文档/不开 dev tools | 真实浏览器自动化 + 维护者人工验收 |
| 有效配置可见 | L1 chat/preview 来源标签 + L2 topbar 指示器 |
| 失败返回可行动错误 | L1 F5/F6 用例 + L3 端点错误契约 |
| provider secret 不持久化 | L1 api_key 用例 + L3 settings_post 用例 |

---

## 8. 安全不变式汇总

| 不变式 | 来源 | 验证 |
|---|---|---|
| `api_key` 永不作为 Port 成员 | §3.6 | L1 Port 契约 + code review |
| `api_key` 永不写入浏览器存储 | §4.3 Stage 2 | L1 api_key 用例 + L3 settings_post |
| `api_key` 永不出现在 URL/query | §4.3 Stage 2 | L1 + code review |
| `api_key` GET 响应整段文本不含明文 | §4.3 Stage 2 | L3 settings_post raw secret check |
| `api_key` POST 后输入框立即清空 | §4.3 Stage 2 | L1 api_key 非空携带用例 |
| 生产模式 `card_path` 被拒 | §4.3 Stage 4 | L1 生产模式用例 |
| `access_api_key` 生产模式不可改 | §4.1 Stage 2 | L3 settings_post（engine 已拒绝） |
| CSP 禁 inline style / eval | §2.1 | L4 system-Chrome smoke |
| `onboarding.js` 零 import 宿主模块 | §3.6 | L1 + code review |
| 网关日志不得记录 api_key 明文（gemini id=3602733464） | §3.2 | code review + gateway 配置审计 |
| 网关 X-AIRP-* 转发头注入与剥离一致 | §3.2 | L4 production topology smoke |
| `#onboarding-root` 含 `role="dialog"` + `aria-modal="true"` | §4.2 a11y | L1 DOM 静态检查 |
| 向导 mount 时保存焦点、cleanup 时恢复 | §4.2 a11y | L2 cleanup 测试 |

**网关日志过滤**（gemini id=3602733464 + id=3602733482）：生产模式下网关（Caddy / nginx）必须配置 access log 过滤规则，剥离请求 body 与 `Authorization` header 后才落盘。Caddy 示例：`log { output file /var/log/caddy/access.log { roll_size 100mb } format json }` + 自定义 `exclude` 字段；nginx 示例：`log_format api '$remote_addr - $remote_user [$time_local] "$request_method $uri" $status $body_bytes_sent';`（不带 `$request_body`）。Engine 自身日志（tracing）已通过 `tracing::warn!`/`tracing::info!` 输出不含 api_key 明文的结构化字段，无需额外过滤。

**凭证头与 query 参数过滤**（gemini id=3602733482）：除 body 过滤外，网关必须：
- 显式从 access log 中删除 `Authorization`、`Cookie`、`Set-Cookie` 头，不得以任何形式记录
- 对 query 参数进行正则脱敏，已知敏感参数（`api_key`、`access_token`、`key`、`token`、`secret`）替换为 `***REDACTED***`
- 严禁启用 `log_credentials` 或等价的凭证记录选项
- 验证方式：L4 production topology smoke 检查 access log 文件不含 `sk-` 前缀明文与 `Bearer ` 字面量

---

## 9. 文件清单（实现时新增/修改）

| 文件 | 动作 | 说明 |
|---|---|---|
| `webui/onboarding.js` | 新增 | 向导主模块，导出 `mountOnboarding` |
| `webui/shared.js` | 新增 | `makeFetcher` + `formatError`，宿主与向导共享（向导通过 Port 注入拿到） |
| `webui/tests/onboarding.test.mjs` | 新增 | L1 + L2 测试 |
| `webui/smoke.mjs` | 修改 | 扩展 L3 烟雾用例 |
| `webui/index.html` | 修改 | 新增 `#onboarding-root` section + topbar effective config 指示器 |
| `webui/app.js` | 修改 | `bootstrapApp` + `shouldShowOnboarding` + `onComplete`/`onSkip` + `renderEffectiveConfigIndicator` + 抽 `makeFetcher`/`formatError` 到 shared.js |
| `webui/style.css` | 修改 | 向导样式（CSS 类，无 inline style） |

---

## 10. 关联

- 解阻：[#207](https://github.com/GhostXia/AIRP/issues/207) 目标 1（首聊黄金路径）
- 实现 issue：[#209](https://github.com/GhostXia/AIRP/issues/209)
- 砍除项跟踪：[#210](https://github.com/GhostXia/AIRP/issues/210)（Shadow DOM）、[#211](https://github.com/GhostXia/AIRP/issues/211)（版本协商）
- 上层 umbrella：[#130](https://github.com/GhostXia/AIRP/issues/130) WebUI production launch
- 门禁合同：`docs/WEBUI-PRODUCTION-PLAN.md` §2.2
- 架构约束：`docs/WEBUI-PRODUCTION-ARCHITECTURE.md`
- Persona 契约：`docs/PERSONA-HTTP-API-PLAN.md`
- 现有 harness（非向导）：`webui/app.js`（M1 backend validation）
