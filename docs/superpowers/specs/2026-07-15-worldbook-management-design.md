# D：Worldbook 管理 + 高级字段裁定

> 日期：2026-07-15
>
> 交付状态：D-PR1（engine v4）已由 PR #180 合并到 `main@fb523b8`；D-PR2（主面板管理）仍未交付
>
> 方向：#126 Worldbook 管理 + 高级字段裁定
>
> 基线：`main@db4fc12`（PR #179）
>
> 并行策略：D-PR1（engine v4）与 C-PR1/C-PR2 零文件冲突，可完全并行；D-PR2（WebUI 迁移）与 C-PR1 改不同区域，git 可自动合并

## 1. 目标

闭合 #126：补普通用户可操作的 worldbook 管理，并显式裁定高级 SillyTavern 字段是 runtime / advisory / unsupported。

## 2. 范围

### 包含

- 高级字段裁定：`selective` + `secondary_keys` 提升为 deterministic runtime（v4 合同）；`probability`/`case_sensitive`/`position`/`depth`/`recursion` 保持 advisory
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
| v3 runtime trigger（constant + primary keyword） | [lorebook.rs](../../../engine/src/orchestrator/lorebook.rs#L63) `Lorebook::trigger` | v3 已交付 |
| v3 advisory 字段（secondary_keys/case_sensitive/extensions） | [lorebook.rs](../../../engine/src/orchestrator/lorebook.rs#L17) `LorebookEntry` | v3 已交付 |
| 共享 normalizer（PNG/PUT API/Agent tool 三入口） | [worldbook_normalizer.rs](../../../engine/src/orchestrator/worldbook_normalizer.rs) | v3 已交付 |
| 导入诊断 WorldbookImportReport | normalizer | v3 已交付 |
| workbench lorebook 编辑器（6 runtime 字段） | [app.js](../../../webui/app.js#L1952) `renderLoreEntry` | 已交付（开发者面板） |
| lorebook HTTP 端点 | [lorebook handler](../../../engine/src/daemon/handlers/lorebook.rs) | 已交付 |

## 4. 裁定

### 4.1 字段裁定表

| 字段 | 裁定 | 理由 |
|---|---|---|
| `selective` | **runtime (v4)** | 语义可被 AIRP 定义为有界、确定性的二阶段检索；自包含于 `trigger()`，不改 prompt placement |
| `secondary_keys` | **runtime (v4)** | 与 selective 配对；v3 已是 canonical 字段，v4 起 trigger() 消费 |
| `probability` | advisory | 引入非确定性，破坏"确定性 trigger 测试"合同，需单独设计可测 RNG |
| `case_sensitive` | advisory | 需 case-insensitive DFA 或 lowercase 预处理，与现有 LeftmostLongest 语义需协调 |
| `position` | advisory | 需重写 prompt 装配结构，高风险，不适 P1 |
| `depth` | advisory | 依赖 position，同上 |
| `recursion` | advisory | 递归扫描有终止/循环顾虑，需单独安全合同 |

### 4.2 v4 trigger 规则

```text
enabled && (
  constant ||
  (primary_match && (!selective || no_valid_secondary_keys || any_secondary_match))
)
```

- `selective=false`（默认）：退化为 v3 行为 `enabled && (constant || primary_match)`
- `selective=true` 且过滤空字符串后仍有 secondary keys：要求 primary 命中，且任一
  secondary key 命中（OR / any-match）；不是 all-match
- `selective=true` 且过滤后无有效 secondary key：为保持 v3 primary-only 行为而退化为
  primary-only，不报错；import report 增加 needs-review 诊断，提示 selective 无实际约束
- `constant=true`：仍跳过关键词检查，直接注入（v3 行为不变）

此裁定依据 AIRP 的确定性、可测试与固定 prompt-placement 门禁，不以“上游常用”作为
充分理由。`case_sensitive`、position/depth、概率与递归仍不进入 runtime；实现 v4 时同步
更新 [TAVERN-PARITY.md](../../TAVERN-PARITY.md) 中旧的 advisory 候选描述，避免两份合同互相冲突。

## 5. 设计

### 5.1 D-PR1：engine 合同 v4

#### LorebookEntry 字段变更

[lorebook.rs](../../../engine/src/orchestrator/lorebook.rs#L17)：

- `selective: bool` 提升为 canonical 字段，`#[serde(default)]` → `false`
- `secondary_keys: Vec<String>` 已是 v3 canonical 字段，不变；v4 起 trigger() 消费它
- scene merge 的 semantic equality 加入 `selective`；仅该字段不同的条目不得提前去重
- `extensions` 不再包含 `selective`（normalizer 映射到 canonical）
- `LorebookService::read` 在反序列化默认值前检查 raw JSON 的字段 presence：仅当
  top-level `selective` 缺席时提升 `extensions.selective`；显式 top-level `false` 优先

#### trigger() 改动

[lorebook.rs](../../../engine/src/orchestrator/lorebook.rs#L63) `Lorebook::trigger`：

现有流程：
1. 跳过 `enabled=false`
2. `constant=true` 直接标记命中
3. 其余 entry keys 扁平化进 Aho-Corasick DFA 单次扫描
4. 按 priority 降序拼接

v4 流程：
1. 跳过 `enabled=false`
2. `constant=true` 直接标记命中
3. 其余 entry primary keys 扁平化进 DFA 单次扫描
4. **新增**：对 primary 命中且 `selective=true` 的 entry，先过滤空 secondary key
   - 过滤后为空 → 保持 primary 命中
   - 非空 → 任一 secondary key 命中 scan_text 才最终命中（OR / any-match）
   - secondary 匹配可用独立的 Aho-Corasick DFA 或有明确上界的线性扫描
5. 按 priority 降序拼接

实现细节：v4 secondary 匹配使用 case-sensitive 语义，与当前 primary runtime 的实际
行为一致；top-level `case_sensitive` 仍是 advisory，不能据其值切换匹配模式。当前实现
选用线性 `scan_text.contains(key)` 并先过滤空 key；v4 不新增独立数量上限，资源上限与
整个 worldbook 文档及请求体共用，后续若建立统一 lorebook quota 再同时约束 primary 与
secondary，避免两套限制漂移。

#### normalizer 改动

[worldbook_normalizer.rs](../../../engine/src/orchestrator/worldbook_normalizer.rs)：

- `selective` 从 ST-only preservation 列表移除
- 提取优先级固定为 top-level canonical `selective` → `extensions.selective`（v3
  persisted migration）→ `false`；类型错误进入 invalid/needs-review，不静默当 false
- `collect_extensions` 在复制旧 `extensions` 时显式移除已提升的 `selective`，防止同一
  语义同时存在 canonical 与 advisory 两份
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

[index.html](../../../webui/index.html)：
- 在 `state-section` 附近加 `<details class="panel-section" id="lorebook-section">`
- workbench 的 `wb-tab-lorebook` tab 移除（或保留为只读 raw JSON 视图，区分"管理"与"开发检视"）

[app.js](../../../webui/app.js)：
- `wbLoreData`/`wbLoreEntries`/`wbLoreStatus` 等 DOM refs 改为指向新 section
- `loadWorkbenchLore`/`saveWorkbenchLore`/`renderLoreEntries`/`renderLoreEntry`/`addLoreEntry` 迁移到新 section 上下文
- character 切换时自动加载 lorebook（类似 state-section 的刷新）

#### renderLoreEntry 增强

现有 6 字段（keys/priority/enabled/constant/content/comment）保留。新增：

**head 行**：
- `selective` checkbox（标注「选择性」）
- `secondary_keys` input（逗号分隔，当 `selective=true` 时启用，否则 disabled）

`secondary_keys` 保存前按逗号拆分，每个 token 去除首尾空白并移除空 token；保留首次
出现的输入顺序，重复 token 只保留第一次。例：`writer, roleplay, writer` 序列化为
`["writer", "roleplay"]`，不得把前导空格写入触发 key。

**body 区**：
- advisory 字段只读展示区（当 entry 含 advisory metadata 时显示）：
  - top-level `entry.case_sensitive`
  - `entry.extensions` 中的 `position`、`depth`、`probability`、`recursion` 及未知字段
  - 标注「advisory（不影响运行时）」
  - 两条读取路径必须分开；不得假定 `case_sensitive` 位于 `extensions`

`addLoreEntry` 默认值加 `selective: false, secondary_keys: []`。

#### workbench 保留内容

workbench 保留：
- card tab（raw JSON 检视，开发工具）
- decompose/analysis tab（开发工具）

lorebook tab 移除或改为只读 raw JSON（与主面板的管理编辑器区分）。

### 5.3 数据流

```text
D-PR1:
  导入 ST card → normalizer 把 selective 映射到 canonical 字段
  trigger() → selective=true 且有有效 secondary keys 时要求任一 secondary key 命中
  prompt 装配 → 不变（仍是单个 [World Info/Lorebook Information] block）

D-PR2:
  用户选 character → 主面板 lorebook-section 加载
  用户编辑条目（含 selective/secondary_keys）→ PUT /v1/characters/:id/lorebook
  case_sensitive 从 top-level、其余 advisory 从 extensions 只读显示
```

## 6. 边界与错误处理

| 场景 | 行为 |
|---|---|
| `selective=true` 但 `secondary_keys` 为空 | 退化为 primary-only 匹配；不报错 |
| `selective=false` | primary-only 匹配（v3 行为） |
| 旧 v3 数据无 `selective` 字段 | `#[serde(default)]` → `false`，行为不变 |
| 旧 v3 数据仅有 `extensions.selective` | read/normalizer 在默认反序列化前按 presence 提升；显式 top-level 值优先 |
| 导入 ST card 含 `selective` | normalizer 映射到 canonical，不进 extensions |
| 导入 ST card 含 `position`/`depth` | 仍进 extensions（advisory），UI 只读显示 |
| 导入 ST card 含 `probability` | 仍进 extensions（advisory），UI 只读显示 |
| secondary_keys 含空字符串 | normalizer 与 trigger 两层都过滤；过滤后为空则 primary-only，并报告 needs-review |

## 7. 测试

### D-PR1 engine

新 fixture `engine/tests/fixtures/worldbook/airp-v4-selective.json`：
- selective=true + secondary_keys 命中 → 注入
- selective=true + secondary_keys 未命中 → 不注入
- selective=true + secondary_keys 为空 → 退化为 primary-only
- selective=false → primary-only（v3 行为）
- constant=true + selective=true → 仍注入（constant 优先）
- 多个 secondary keys 仅一个命中 → 注入（锁定 OR / any-match）
- 多个 secondary keys 全未命中 → 不注入
- canonical v3 `extensions.selective=true` → 提升到 top-level，extensions 不再残留 selective
- 两条仅 selective 不同的同 content entry merge 后保留独立 activation semantics

deterministic trigger 测试 + prompt-placement 测试（验证 selective 命中/未命中的最终 prompt 输出）。

normalizer round-trip 测试：ST top-level 与 v3 `extensions.selective` 都映射到 canonical，
不再残留于 extensions；top-level 与 extensions 冲突时 top-level 胜出并产生诊断。

### D-PR2 WebUI

在 tracked `webui/tests/` 下用 Node test runner 测试提取出的 lorebook view-model/序列化
纯函数，并在 PR gate 运行 `node --test webui/tests/*.test.mjs`；真实 DOM 迁移另加
production system-Chrome smoke。测试：
1. lorebook-section 在选中 character 后显示
2. selective checkbox 切换 → secondary_keys input 启用/禁用
3. selective=true + secondary_keys 编辑 → 保存 payload 含两字段
4. advisory 字段只读显示（不可编辑）
5. addLoreEntry 默认 selective=false
6. top-level case_sensitive 与 extensions advisory 都显示且保存后无损保留

### 不变式

- `subagent_context_has_no_orchestrator_noise` 全绿
- `subagent_prepared_pipeline_has_no_orchestrator_noise` 全绿（trigger 在 lorebook.rs，需确认 prompt 装配不变式仍绿）
- 当前 workspace 全部 Rust/UI/WebUI/production gates 不退化；不在 spec 固化会漂移的测试数量
- v3 fixture 测试仍绿（向后兼容）

## 8. 文件改动清单

### D-PR1 engine

- [lorebook.rs](../../../engine/src/orchestrator/lorebook.rs)：`LorebookEntry` 加 `selective` 字段；`trigger()` 加 secondary 匹配；merge equality 加 selective
- [worldbook_normalizer.rs](../../../engine/src/orchestrator/worldbook_normalizer.rs)：top-level/ST/v3 extensions 的 `selective` 迁移到 canonical
- [WORLDBOOK-SEMANTICS.md](../../WORLDBOOK-SEMANTICS.md)：升 v4
- [TAVERN-PARITY.md](../../TAVERN-PARITY.md)：实现时同步 runtime/advisory 裁定
- `engine/tests/fixtures/worldbook/airp-v4-selective.json`：新 fixture
- trigger/prompt-placement 测试

### D-PR2 WebUI

- [index.html](../../../webui/index.html)：加 `lorebook-section`；workbench lorebook tab 调整
- [app.js](../../../webui/app.js)：lorebook 编辑器迁移 + `renderLoreEntry` 增强
- `webui/tests/*.test.mjs`：tracked view-model/序列化测试，并接入 PR gate
- [ui/production-browser-smoke.mjs](../../../ui/production-browser-smoke.mjs)：真实 DOM 迁移与交互接线

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
