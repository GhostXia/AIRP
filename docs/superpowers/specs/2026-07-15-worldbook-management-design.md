# D：Worldbook 管理 + 高级字段裁定

> 日期：2026-07-15
>
> 方向：#126 Worldbook 管理 + 高级字段裁定
>
> 基线：`main@c54428e`（PR #177）
>
> 并行策略：D-PR1（engine v4）与 C-PR1/C-PR2 零文件冲突，可完全并行；D-PR2（WebUI 迁移）与 C-PR1 改不同区域，git 可自动合并

## 1. 目标

闭合 #126：补普通用户可操作的 worldbook 管理，并显式裁定高级 SillyTavern 字段是 runtime / advisory / unsupported。

## 2. 范围

### 包含

- 高级字段裁定：`selective` + `secondary_keys` 提升为 runtime（v4 合同）；`probability`/`case_sensitive`/`position`/`depth`/`recursion` 保持 advisory
- engine v4：`LorebookEntry` 加 canonical `selective`/`secondary_keys`；`trigger()` 实现 selective 二次匹配
- normalizer v4：`selective` 从 `extensions` 提升到 canonical 字段
- WebUI：lorebook 编辑器从 workbench 迁移到 character-scoped 主面板 section
- `renderLoreEntry` 增强：selective/secondary_keys 可编辑；advisory 字段只读显示
- WORLDBOOK-SEMANTICS.md 更新到 v4

### 不包含

- `probability`/`case_sensitive`/`position`/`depth`/`recursion` 的 runtime 实现（保持 advisory，未来合同版本）
- prompt 装配结构改动（position/depth 需重写装配，高风险，不适 P1）
- workbench card raw JSON / decompose/analysis 迁移（保留为开发工具）

## 3. 背景：已交付地基

| 能力 | 位置 | 状态 |
|---|---|---|
| v3 runtime trigger（constant + primary keyword） | [lorebook.rs:63-129](file:///d:/AIRP-Dev/engine/src/orchestrator/lorebook.rs) `Lorebook::trigger` | v3 已交付 |
| v3 advisory 字段（secondary_keys/case_sensitive/extensions） | [lorebook.rs:17-46](file:///d:/AIRP-Dev/engine/src/orchestrator/lorebook.rs) `LorebookEntry` | v3 已交付 |
| 共享 normalizer（PNG/PUT API/Agent tool 三入口） | [worldbook_normalizer.rs](file:///d:/AIRP-Dev/engine/src/orchestrator/worldbook_normalizer.rs) | v3 已交付 |
| 导入诊断 WorldbookImportReport | normalizer | v3 已交付 |
| workbench lorebook 编辑器（6 runtime 字段） | [app.js:1952-2080](file:///d:/AIRP-Dev/webui/app.js) `renderLoreEntry` | 已交付（开发者面板） |
| lorebook HTTP 端点 | [lorebook handler](file:///d:/AIRP-Dev/engine/src/daemon/handlers/lorebook.rs) | 已交付 |

## 4. 裁定

### 4.1 字段裁定表

| 字段 | 裁定 | 理由 |
|---|---|---|
| `selective` | **runtime (v4)** | 最常用 ST 功能；自包含于 trigger()，不改 prompt 结构 |
| `secondary_keys` | **runtime (v4)** | 与 selective 配对；v3 已是 canonical 字段，v4 起 trigger() 消费 |
| `probability` | advisory | 引入非确定性，破坏"确定性 trigger 测试"合同，需单独设计可测 RNG |
| `case_sensitive` | advisory | 需 case-insensitive DFA 或 lowercase 预处理，与现有 LeftmostLongest 语义需协调 |
| `position` | advisory | 需重写 prompt 装配结构，高风险，不适 P1 |
| `depth` | advisory | 依赖 position，同上 |
| `recursion` | advisory | 递归扫描有终止/循环顾虑，需单独安全合同 |

### 4.2 v4 trigger 规则

```
enabled && (
  constant ||
  (primary_match && (selective ? secondary_match : true))
)
```

- `selective=false`（默认）：退化为 v3 行为 `enabled && (constant || primary_match)`
- `selective=true` 且 `secondary_keys` 非空：要求 primary AND secondary 同时命中
- `selective=true` 且 `secondary_keys` 为空：退化为 primary-only（selective 无意义，不报错）
- `constant=true`：仍跳过关键词检查，直接注入（v3 行为不变）

## 5. 设计

### 5.1 D-PR1：engine 合同 v4

#### LorebookEntry 字段变更

[lorebook.rs:17-46](file:///d:/AIRP-Dev/engine/src/orchestrator/lorebook.rs)：

- `selective: bool` 提升为 canonical 字段，`#[serde(default)]` → `false`
- `secondary_keys: Vec<String>` 已是 v3 canonical 字段，不变；v4 起 trigger() 消费它
- `extensions` 不再包含 `selective`（normalizer 映射到 canonical）

#### trigger() 改动

[lorebook.rs:63-129](file:///d:/AIRP-Dev/engine/src/orchestrator/lorebook.rs) `Lorebook::trigger`：

现有流程：
1. 跳过 `enabled=false`
2. `constant=true` 直接标记命中
3. 其余 entry keys 扁平化进 Aho-Corasick DFA 单次扫描
4. 按 priority 降序拼接

v4 流程：
1. 跳过 `enabled=false`
2. `constant=true` 直接标记命中
3. 其余 entry primary keys 扁平化进 DFA 单次扫描
4. **新增**：对 primary 命中且 `selective=true` 且 `secondary_keys` 非空的 entry，再检查 secondary_keys 是否命中 scan_text
   - secondary 匹配用独立的 Aho-Corasick DFA 或线性扫描（secondary_keys 通常数量少，线性扫描可接受）
   - secondary 命中 → entry 最终命中；未命中 → 不注入
5. 按 priority 降序拼接

实现细节：secondary 匹配用线性 `scan_text.contains(key)` 还是 DFA？考虑 secondary_keys 通常每条目 1-3 个，线性扫描足够，避免 DFA 复杂度。但 case_sensitive 需与 primary 一致（当前 case-sensitive LeftmostLongest）。v4 secondary 匹配也用 case-sensitive。

#### normalizer 改动

[worldbook_normalizer.rs](file:///d:/AIRP-Dev/engine/src/orchestrator/worldbook_normalizer.rs)：

- `selective` 从 ST-only preservation 列表移除
- `selective` 映射到 canonical `selective: bool` 字段（ST `selective` 是 bool）
- `secondary_keys` 映射不变（v3 已映射）
- `extensions` 不再包含 `selective`

#### WORLDBOOK-SEMANTICS.md 更新

- 版本升至 v4
- runtime 字段表加 `selective`、`secondary_keys`
- trigger 规则更新为 v4 公式
- advisory 字段表移除 `selective`（保留 secondary_keys 说明已升级为 runtime）
- extensions 列表移除 `selective`
- change gate：v4 trigger + prompt-placement 测试 + fixture

### 5.2 D-PR2：WebUI worldbook 管理迁移

#### 迁移：workbench → 主面板

将 lorebook 编辑器从 workbench `wb-tab-lorebook` 迁移到主面板的新 character-scoped section。

[index.html](file:///d:/AIRP-Dev/webui/index.html)：
- 在 `state-section` 附近加 `<details class="panel-section" id="lorebook-section">`
- workbench 的 `wb-tab-lorebook` tab 移除（或保留为只读 raw JSON 视图，区分"管理"与"开发检视"）

[app.js](file:///d:/AIRP-Dev/webui/app.js)：
- `wbLoreData`/`wbLoreEntries`/`wbLoreStatus` 等 DOM refs 改为指向新 section
- `loadWorkbenchLore`/`saveWorkbenchLore`/`renderLoreEntries`/`renderLoreEntry`/`addLoreEntry` 迁移到新 section 上下文
- character 切换时自动加载 lorebook（类似 state-section 的刷新）

#### renderLoreEntry 增强

现有 6 字段（keys/priority/enabled/constant/content/comment）保留。新增：

**head 行**：
- `selective` checkbox（标注「选择性」）
- `secondary_keys` input（逗号分隔，当 `selective=true` 时启用，否则 disabled）

**body 区**：
- advisory 字段只读展示区（当 entry 含 advisory metadata 时显示）：
  - `case_sensitive`、`position`、`depth`、`probability`、`recursion` 等
  - 标注「advisory（不影响运行时）」
  - 从 `entry.extensions` 读取

`addLoreEntry` 默认值加 `selective: false, secondary_keys: []`。

#### workbench 保留内容

workbench 保留：
- card tab（raw JSON 检视，开发工具）
- decompose/analysis tab（开发工具）

lorebook tab 移除或改为只读 raw JSON（与主面板的管理编辑器区分）。

### 5.3 数据流

```
D-PR1:
  导入 ST card → normalizer 把 selective 映射到 canonical 字段
  trigger() → selective=true 时要求 secondary_keys 也命中
  prompt 装配 → 不变（仍是单个 [World Info/Lorebook Information] block）

D-PR2:
  用户选 character → 主面板 lorebook-section 加载
  用户编辑条目（含 selective/secondary_keys）→ PUT /v1/characters/:id/lorebook
  advisory 字段从 extensions 只读显示
```

## 6. 边界与错误处理

| 场景 | 行为 |
|---|---|
| `selective=true` 但 `secondary_keys` 为空 | 退化为 primary-only 匹配；不报错 |
| `selective=false` | primary-only 匹配（v3 行为） |
| 旧 v3 数据无 `selective` 字段 | `#[serde(default)]` → `false`，行为不变 |
| 导入 ST card 含 `selective` | normalizer 映射到 canonical，不进 extensions |
| 导入 ST card 含 `position`/`depth` | 仍进 extensions（advisory），UI 只读显示 |
| 导入 ST card 含 `probability` | 仍进 extensions（advisory），UI 只读显示 |
| secondary_keys 含空字符串 | normalizer 过滤空字符串（与 primary keys 一致） |

## 7. 测试

### D-PR1 engine

新 fixture `engine/tests/fixtures/worldbook/airp-v4-selective.json`：
- selective=true + secondary_keys 命中 → 注入
- selective=true + secondary_keys 未命中 → 不注入
- selective=true + secondary_keys 为空 → 退化为 primary-only
- selective=false → primary-only（v3 行为）
- constant=true + selective=true → 仍注入（constant 优先）

deterministic trigger 测试 + prompt-placement 测试（验证 selective 命中/未命中的最终 prompt 输出）。

normalizer round-trip 测试：ST `selective` 字段映射到 canonical，不进 extensions。

### D-PR2 WebUI

DOM 测试：
1. lorebook-section 在选中 character 后显示
2. selective checkbox 切换 → secondary_keys input 启用/禁用
3. selective=true + secondary_keys 编辑 → 保存 payload 含两字段
4. advisory 字段只读显示（不可编辑）
5. addLoreEntry 默认 selective=false

### 不变式

- `subagent_context_has_no_orchestrator_noise` 全绿
- `subagent_prepared_pipeline_has_no_orchestrator_noise` 全绿（trigger 在 lorebook.rs，需确认 prompt 装配不变式仍绿）
- 现有 552 lib + 1 ignored + 40 integration 不退化
- v3 fixture 测试仍绿（向后兼容）

## 8. 文件改动清单

### D-PR1 engine

- [lorebook.rs](file:///d:/AIRP-Dev/engine/src/orchestrator/lorebook.rs)：`LorebookEntry` 加 `selective` 字段；`trigger()` 加 secondary 匹配
- [worldbook_normalizer.rs](file:///d:/AIRP-Dev/engine/src/orchestrator/worldbook_normalizer.rs)：`selective` 映射到 canonical
- [WORLDBOOK-SEMANTICS.md](file:///d:/AIRP-Dev/docs/WORLDBOOK-SEMANTICS.md)：升 v4
- `engine/tests/fixtures/worldbook/airp-v4-selective.json`：新 fixture
- trigger/prompt-placement 测试

### D-PR2 WebUI

- [index.html](file:///d:/AIRP-Dev/webui/index.html)：加 `lorebook-section`；workbench lorebook tab 调整
- [app.js](file:///d:/AIRP-Dev/webui/app.js)：lorebook 编辑器迁移 + `renderLoreEntry` 增强
- `target/` 下加 WebUI DOM 测试

## 9. 验收标准

1. 导入含 `selective` 的 ST card → normalizer 映射到 canonical，不进 extensions
2. `selective=true` + secondary_keys 命中 → entry 注入
3. `selective=true` + secondary_keys 未命中 → entry 不注入
4. `selective=false` → v3 行为不变
5. 旧 v3 数据无 `selective` → 行为不变（向后兼容）
6. 主面板 lorebook-section 可编辑 selective/secondary_keys
7. advisory 字段只读显示
8. engine 全套测试 + WebUI DOM 测试全绿
9. 2 个 subagent 不变式全绿
10. v3 fixture 测试仍绿

## 10. 后续（不在本 spec 范围）

- `probability` runtime：需设计可测 RNG（seed 化或注入式）
- `case_sensitive` runtime：需 case-insensitive DFA
- `position`/`depth` runtime：需重写 prompt 装配结构
- `recursion` runtime：需递归深度限制与终止合同
