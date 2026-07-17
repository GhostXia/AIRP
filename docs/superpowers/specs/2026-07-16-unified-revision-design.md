# #115 Phase 2：统一 revision/provenance 设计

> 日期：2026-07-16
>
> 状态：**已实现**。本文为 #115 Phase 2 的独立设计。
>
> - **Phase 2a 已实现**（PR #201）：`engine/src/revision/` 底层模块（tree_hash + manifest + atomic）+ Phase 2h 部分（4 个 `*_revision_unavailable` 诊断）。
> - **Phase 2b 已实现**（PR #202）：Preset 接入统一 revision 合同（保留 `versions/{generation}/` + `current` 旧指针）。
> - **Phase 2c-2g 已实现**（PR #203）：Character / Worldbook / State / Memory / Persona 接入统一 revision 合同。
> - **Phase 2h 已实现**（本 PR）：`build_prompt_trace` 全部 6 个 `*_revision` 字段按 §5.3 填充实际 u64 或推送对应 `*_revision_unavailable` 诊断；WebUI 装配预览面板展示 6 个 revision（含 unavailable 标识）；`production-browser-smoke.mjs` 覆盖 6 个 revision 字段渲染断言。
>
> 任何能力交付以源码、测试和 `CURRENT-BASELINE.md` 为准。
>
> 设计基线：`main@c38e7ec`（PR #196 / #197）
>
> 审计源模型：GLM-5.2（本会话）
>
> 关联：[#115](https://github.com/GhostXia/AIRP/issues/115) Phase 2、[SESSION-DATA-DESIGN.md](../../SESSION-DATA-DESIGN.md) §4、[#199](https://github.com/GhostXia/AIRP/issues/199)、[#114](https://github.com/GhostXia/AIRP/issues/114)

## 1. 目标

闭合 #115 Phase 2：「在 #115 已交付的真实装配/HTTP/WebUI 可观察闭环上，补齐角色卡、Preset、Worldbook、state/memory 的统一 revision/provenance；不得用 mtime 伪造 revision」。

具体目标：

1. **可追溯**：每类 RP asset 的每次内容变更都产生不可变历史版本，可回到任意历史版本。
2. **可观察**：`PromptAssemblyTrace.effective` 暴露本轮使用的每类 asset 的数值 revision；用户和审计者可凭 trace 复现本轮装配输入。
3. **不可伪造**：revision 必须由内容 hash 派生或与内容 hash 共同持久化；禁止用文件 mtime、文件名时间戳或单调递增计数器冒充内容版本。
4. **不破坏现有合同**：SESSION-DATA-DESIGN.md §4 已定义的 `content_revision` 整数 + `AIRP-TREE-SHA256-v1` + 不可变快照目录 + `current` 指针合同为本设计的权威上限；本设计不得违反该合同。
5. **不破坏现有数据**：现有 Preset `versions/{generation}/` + `current` 指针、Persona `personas/{id}.json` u64 revision、State `history.jsonl` append-only 等已交付形态必须保留；升级通过 lazy migration 完成，不强制批量重写。

## 2. 范围

### 包含

- 统一 revision 数据模型与底层模块（tree hash、manifest schema、atomic commit）
- 5 类 asset 的 revision 升级或新建：Preset、Character card、Worldbook、State、Memory
- Persona revision 升级以对齐统一合同（补不可变历史 + content hash）
- `PromptAssemblyTrace.effective` 全部 6 个 `*_revision` 字段填充
- `PromptSegment` 是否扩展 revision 字段的裁定（见 §6.4）
- 与 SESSION-DATA-DESIGN.md §4 `revisions/{revision_id}/` session manifest 的关系裁定（见 §7）

### 不包含

- session 自包含存档的完整实现（SESSION-DATA-DESIGN.md §5 生命周期、§7 分阶段实施）由 session 数据设计独立推进；本设计只产出 per-asset revision 基础设施
- Persona base lock、drift overlay、头像、导入导出（属 #114，由 #114 闭合；本设计只在 §6.6 升级 Persona 的 revision 存储格式）
- Preset import 的 dry-run / collision preview / overwrite 显式确认协议（属 #115 Phase 1 未完成项，由独立 PR 推进）
- NextPayloadCapture、tool-call/result history integrity、preset tool restriction（属 #115 P2/P3，与本设计正交）
- 第三方世界书素材库（SESSION-DATA-DESIGN.md §5.1、§7.3，由 session 数据设计推进）

## 3. 现状审计（基于 `main@c38e7ec` 源码事实）

### 3.1 Persona revision（唯一被 trace 填充的数值 revision）

- 字段：`Persona.revision: u64`，从 0 起，每次 `PersonaService::save` 执行 `revision = current + 1`
- 乐观锁：`expected_revision` 参数校验，冲突返回 `PersonaRevisionConflict { current_revision }`
- 串行化：per-user `Mutex`（`PERSONA_LOCKS` static）
- 落盘：`data_dir::replace_file` 原子替换 `personas/{pid}.json`（temp-rename-backup）
- **缺口**：
  - 无不可变历史（覆写式存储，旧 revision 随覆写丢失）
  - 无 content hash
  - 无 history.jsonl（`domain.rs:827` 注释声称 append history.jsonl，**与实际代码不符**，属幽灵合同）
  - base lock 路径 `user_persona_lock_path` 已声明但全仓无调用方
  - 导入/导出未实现

**关键文件**：[engine/src/domain.rs](../../../engine/src/domain.rs#L739-L761)（schema）、[#L935-L980](../../../engine/src/domain.rs#L935-L980)（save）、[#L818-L822](../../../engine/src/domain.rs#L818-L822)（冲突响应）

### 3.2 Preset revision（最完善的版本化实现，但 generation 字符串与 §4 合同不一致）

- 字段：`generation: String` = `{timestamp_nanos}-{source_hash_12hex}`（非数值 revision）
- 不可变历史：`presets/{id}/versions/{generation}/` 目录，每次写入新建目录，不覆盖
- 原子指针：`presets/{id}/current` 文件存当前 generation 字符串，`replace_file` 原子替换
- content hash：`source_hash` = SHA-256 截断 12 hex chars
- provenance：`PresetImportReport` 含 `format_version` / `source_hash` / `converter_version` / `imported_at`
- 串行化：`PRESET_WRITE_LOCK` 全局 Mutex
- **缺口**：
  - generation 是字符串，与 SESSION-DATA-DESIGN.md §4 的 `content_revision: u64` 合同不一致
  - `EffectiveIds.preset_revision: Option<u64>` 字段存在但留空
  - trace 推送 `preset_revision_unavailable` 诊断
  - `PresetImportReport` 是否作为独立 sidecar 落盘未明确（当前仅返回 HTTP 客户端）

**关键文件**：[engine/src/orchestrator/preset.rs](../../../engine/src/orchestrator/preset.rs#L487-L519)（write）、[#L60-L90](../../../engine/src/orchestrator/preset.rs#L60-L90)（report schema）、[#L162-L174](../../../engine/src/orchestrator/preset.rs#L162-L174)（source_hash）

### 3.3 Character card（无任何版本化）

- 字段：`TavernCardV2.spec_version`（schema 版本，非内容 revision）
- 落盘：`fs::write` 直接覆盖 `card/card.json` 和 `card/raw.json`
- **缺口**：
  - 无 revision / version / generation 字段
  - 无不可变历史
  - 无 content hash
  - 无 provenance sidecar（`raw.json` 本意是导入时原始 sidecar，但 PUT 端点会一并覆盖，不具备不可变性）
  - `EffectiveIds.character_revision` 留空，trace 推送 `character_revision_unavailable` 诊断

**关键文件**：[engine/src/orchestrator/card.rs](../../../engine/src/orchestrator/card.rs#L37-L62)（schema）、[engine/src/daemon/handlers/characters.rs](../../../engine/src/daemon/handlers/characters.rs#L80-L191)（import）、PUT 端点（覆盖 raw.json）

### 3.4 Worldbook（无版本化，运行时 provenance 仅 in-memory）

- 字段：无 revision / version 字段（`Lorebook { entries: Vec<LorebookEntry> }`）
- 落盘：`LorebookService::write` 调 `replace_file` 整体覆盖 `world/lorebook.json`
- 运行时 provenance：`SourcedLorebook` / `MergedLorebook` / `TriggeredLorebookEntry` 仅 in-memory，不落盘
- **缺口**：
  - 无 revision / version / generation 字段
  - 无不可变历史
  - 无 content hash（`WorldbookImportReport` 无 `source_hash` 字段，与 `PresetImportReport` 不对称）
  - 无持久化 provenance（report 仅返回 HTTP 客户端）
  - `EffectiveIds.lorebook_revision` 留空，**未推送** `lorebook_revision_unavailable` 诊断（与 character/preset 不一致）

**关键文件**：[engine/src/orchestrator/lorebook.rs](../../../engine/src/orchestrator/lorebook.rs#L60-L64)（schema）、[#L180-L220](../../../engine/src/orchestrator/lorebook.rs#L180-L220)（运行时 provenance）、[engine/src/orchestrator/worldbook_normalizer.rs](../../../engine/src/orchestrator/worldbook_normalizer.rs#L64-L83)（report schema）

### 3.5 State（部分版本化，缺 content hash）

- 字段：`StateSnapshot.revision: u64`，从 `latest_revision(history.jsonl) + 1` 派生
- 不可变历史：`state/history.jsonl` append-only，每行一个 `StateSnapshot` JSON
- 当前值：`state/live.json` 覆盖写入
- **缺口**：
  - 无 content hash
  - 无 source provenance（只有 `timestamp` + `revision`）
  - `gating/` 目录（timeline.md / checkpoints.md / known.md / disclosure.json）无任何版本化，in-place 修改
  - `EffectiveIds.state_revision` 留空，**未推送** `state_revision_unavailable` 诊断
  - `state_revision` 与 `gating` 关系未裁定（见 §6.5）

**关键文件**：[engine/src/domain.rs](../../../engine/src/domain.rs#L421-L571)（StateService）、[#L573-L604](../../../engine/src/domain.rs#L573-L604)（latest_revision）、[engine/src/orchestrator/gating.rs](../../../engine/src/orchestrator/gating.rs)（gating 模块）

### 3.6 Memory（无版本化）

- 字段：无 revision / version 字段（卷编号 `vol_{:03}` 是顺序整数，非内容寻址）
- 不可变历史：部分（`volumes/vol_XXX.md` 封卷后不可变；`current.md` 无历史；`index.md` in-place 修改；`turn_counter.txt` read+increment+write）
- **缺口**：
  - 无 content hash
  - 无 provenance
  - `current.md` 不可变历史缺失
  - `EffectiveIds.memory_revision` 留空，**未推送** `memory_revision_unavailable` 诊断

**关键文件**：[engine/src/volume_store.rs](../../../engine/src/volume_store.rs)

### 3.7 trace 暴露现状汇总

| Asset | `EffectiveIds` 字段 | 当前填充 | diagnostic |
|---|---|---|---|
| Character | `character_revision: Option<u64>` | `None` | `character_revision_unavailable` |
| Persona | `persona_revision: Option<u64>` | 实际值 | 无 |
| Preset | `preset_revision: Option<u64>` | `None` | `preset_revision_unavailable` |
| Lorebook | `lorebook_revision: Option<u64>` | `None` | **无**（缺口未声明） |
| State | `state_revision: Option<u64>` | `None` | **无**（缺口未声明） |
| Memory | `memory_revision: Option<u64>` | `None` | **无**（缺口未声明） |

**关键文件**：[engine/src/orchestrator/trace.rs](../../../engine/src/orchestrator/trace.rs#L47-L62)、[engine/src/chat_pipeline.rs](../../../engine/src/chat_pipeline.rs#L217-L251)

## 4. 设计原则

1. **以 SESSION-DATA-DESIGN.md §4 为权威上限**：`content_revision: u64` 非负整数 + `AIRP-TREE-SHA256-v1` + 不可变快照目录 + `current` 原子指针 + 三处相等不变量（`revisions/{revision_id}/manifest.json.content_revision` == `worldbooks/manifest.json.content_revision` == `meta.json.current_revision`）为本设计的硬约束。
2. **per-asset-id 独立 revision 空间**：每个 `character_id` / `preset_id` / `persona_id` / lorebook scope / state scope / memory scope 有自己的 revision 计数器，从 1 起。不同 asset 的 revision 互不干扰。
3. **session `content_revision` 是复合快照，不是 per-asset revision**：SESSION-DATA-DESIGN.md §4 的 `revisions/{revision_id}/` 同时固定多类 asset，是 session-scoped 复合版本；它**引用** per-asset revision，不替代 per-asset revision。两者并存且不冲突（见 §7）。
4. **底层共享，领域独立**：tree hash 算法、manifest schema、atomic commit 流程为共享底层模块；per-asset 领域服务（`PersonaService` / `PresetService` / `LorebookService` / `CharacterService` / `StateService` / `VolumeStore`）保留各自领域逻辑，不强行统一为 `RevisionService`。
5. **渐进式 migration**：现有数据不强制批量重写；首次写入时 lazy 升级为新格式，旧格式作为兼容回退路径保留至 session 数据设计完成迁移窗口。
6. **不破坏幽灵合同之外的已有合同**：Preset `versions/{generation}/` + `current` 指针保留；Persona `personas/{pid}.json` 保留；State `history.jsonl` 保留。新格式作为增量叠加，不删除旧文件。
7. **乐观锁统一**：所有 asset 的写操作支持 `expected_revision` 参数，冲突响应统一为 `RevisionConflict { asset_kind, asset_id, current_revision }`。
8. **trace 完整性**：`EffectiveIds` 全部 6 个 `*_revision` 字段必须填充实际值或推送对应 `*_revision_unavailable` 诊断；不允许静默留空。

## 5. 统一 revision 数据模型

### 5.1 共享底层模块

新建 `engine/src/revision/` 模块，包含：

```text
engine/src/revision/
├── mod.rs              # 公开 API
├── tree_hash.rs        # AIRP-TREE-SHA256-v1 实现
├── manifest.rs         # RevisionManifest schema 与校验
└── atomic.rs           # atomic commit 流程
```

#### 5.1.1 `tree_hash.rs`

实现 SESSION-DATA-DESIGN.md §4 第 5 条规定的 `AIRP-TREE-SHA256-v1` 算法：

- 输入：ASCII 域分隔符 `AIRP-TREE-SHA256\0v1\0` + 每个批准文件 `u64be(path_utf8_length) || path_utf8 || u64be(file_length) || raw_file_bytes`
- 路径要求：`/` 分隔、无空段 / `.` / `..` / 反斜杠、Unicode NFC、UTF-8 字节序升序
- 文件要求：仅普通文件，拒绝符号链接 / junction / reparse point / 设备文件
- 输出：小写十六进制 SHA-256
- 测试向量：空目录 `a9682729b0a5609f08a1c9a8b2bf49b68edb9056d9e910fd297f694cc3ee3dbf`；单文件 `a.txt` 内容 `x` = `cfa2887973ce5ecc1f2bc57b00ad0130a39aae4d4bf67adae0431ccd3a3ae189`

#### 5.1.2 `manifest.rs`

```rust
pub struct RevisionManifest {
    pub schema: u32,                  // = 1
    pub content_revision: u64,        // 非负整数，从 1 起
    pub asset_kind: String,           // "character" | "preset" | "worldbook" | "state" | "memory" | "persona"
    pub asset_id: String,             // character_id / preset_id / persona_id / scope id
    pub created_at: String,           // RFC3339
    pub source: AssetSource,          // provenance
    pub files: Vec<ApprovedFile>,     // 批准文件集合
    pub tree_sha256: String,          // 覆盖 files 子树的 AIRP-TREE-SHA256-v1
}

pub struct AssetSource {
    pub source_kind: String,          // "inline" | "controlled_upload" | "trusted_desktop" | "derived" | "manual_edit"
    pub source_hash: Option<String>,  // 原始输入字节 SHA-256（完整 hex，不截断）
    pub source_filename: Option<String>,
    pub converter_version: Option<String>,
    pub imported_at: Option<String>,
    pub parent_revision: Option<u64>, // 派生自哪个 revision（用于编辑链）
}

pub struct ApprovedFile {
    pub path: String,                 // 相对于 revision 目录
    pub sha256: String,               // 原始字节 SHA-256
    pub bytes: u64,
}
```

**不变量**（加载时强制校验）：

1. 磁盘普通文件集合 == `files` 集合（缺失或额外文件均错误）
2. 每个文件原始字节 SHA-256 == `files[].sha256`
3. `tree_sha256` == 重新计算的 `AIRP-TREE-SHA256-v1(files)`
4. `content_revision` >= 1
5. `asset_kind` ∈ 枚举集合
6. `asset_id` 经路径校验

任一失败拒绝该 revision，禁止回退到工作副本或部分加载。

#### 5.1.3 `atomic.rs`

实现 SESSION-DATA-DESIGN.md §5.3 规定的 atomic commit 流程：

1. 在 `{asset_dir}/revisions/.staging-{revision_id}/` 同文件系统 staging 目录写完批准文件 + `manifest.json`
2. 逐文件 `sync_data`
3. 同步 staging 目录
4. 全量校验（文件集合 + 每文件 hash + tree hash + manifest 不变量）
5. 原子 rename 为 `{asset_dir}/revisions/{revision_id}/`
6. 同步 `revisions/` 父目录
7. 原子替换 `{asset_dir}/current_revision` 文件内容为 `revision_id` 的十进制字符串
8. 同步 `current_revision` 父目录

任一步失败只留下不被引用的 staging / orphan revision；`current_revision` 永不指向半成品快照。

### 5.2 per-asset 目录布局

每个 asset 在自身目录下新增 `revisions/` 和 `current_revision`：

```text
characters/{character_id}/
├── card/                              # 工作副本（可编辑）
├── world/
│   ├── lorebook.json                  # 工作副本（可编辑）
│   └── revisions/                     # 新增
│       ├── 1/
│       │   ├── manifest.json
│       │   └── lorebook.json
│       └── ...
│   └── current_revision               # 新增，内容为 "3"（十进制 u64）
├── revisions/                         # 新增（角色卡 revision）
│   ├── 1/
│   │   ├── manifest.json
│   │   ├── card.json
│   │   └── raw.json
│   └── ...
└── current_revision                   # 新增，内容为 "3"

presets/{preset_id}/
├── versions/{generation}/             # 保留（Preset 历史格式）
│   ├── preset.json
│   └── raw.json
├── current                            # 保留（Preset 历史指针，存 generation 字符串）
├── revisions/                         # 新增
│   └── {content_revision}/
│       ├── manifest.json
│       ├── preset.json                # 复制（禁止 symlink：tree hash 拒绝符号链接）
│       └── raw.json                   # 复制（禁止 symlink：tree hash 拒绝符号链接）
└── current_revision                   # 新增，内容为 content_revision u64

users/{user_id}/personas/{persona_id}.json   # 保留（覆写式）
users/{user_id}/personas/{persona_id}/       # 新增
├── revisions/
│   └── {content_revision}/
│       ├── manifest.json
│       └── persona.json
└── current_revision

characters/{character_id}/state/
├── live.json                          # 保留
├── history.jsonl                      # 保留（append-only）
├── revisions/                         # 新增（每条 history.jsonl 行对应一个 revision）
│   └── {content_revision}/
│       ├── manifest.json
│       └── state.json                 # = 对应 history.jsonl 行的 state 字段
└── current_revision                   # 新增，= history.jsonl 末行 revision

characters/{character_id}/memory/
├── current.md                         # 保留
├── volumes/                           # 保留
├── revisions/                         # 新增
│   └── {content_revision}/
│       ├── manifest.json
│       ├── current.md
│       └── index.md
└── current_revision                   # 新增
```

**说明**：

- Preset 保留 `versions/{generation}/` 作为历史格式，新增 `revisions/{content_revision}/` 通过**复制**（禁止 symlink，因 tree hash 拒绝符号链接）指向对应 generation 目录；`current`（generation）和 `current_revision`（u64）并存，前者为兼容回退，后者为统一合同。
- State 复用 `history.jsonl` 的 `revision` 字段作为 `content_revision`；`revisions/{content_revision}/state.json` 是该行 state 字段的物化快照，用于 tree hash 校验。
- gating 目录（`timeline.md` / `checkpoints.md` / `known.md` / `disclosure.json`）是否纳入 state revision 见 §6.5 裁定。

### 5.3 `EffectiveIds` 填充规则

`build_prompt_trace` 构造 `EffectiveIds` 时，每类 asset 必须按以下规则填充：

| 字段 | 填充来源 | 不可用时 |
|---|---|---|
| `character_revision` | `characters/{id}/current_revision` 文件读取 | 推送 `character_revision_unavailable` |
| `persona_revision` | `personas/{id}/current_revision` 读取；回退 `Persona.revision` legacy 字段（仅当 `> 0`，`0` 视为未保存） | 推送 `persona_revision_unavailable` |
| `preset_revision` | `presets/{id}/current_revision` 文件读取 | 推送 `preset_revision_unavailable` |
| `lorebook_revision` | `characters/{id}/world/current_revision` 读取（仅 character 上下文；scene 模式留 `None` 不推送诊断） | 推送 `lorebook_revision_unavailable` |
| `state_revision` | `characters/{id}/state/current_revision` 读取；= `history.jsonl` 末行 revision | 推送 `state_revision_unavailable` |
| `memory_revision` | `characters/{id}/memory/current_revision` 读取 | 推送 `memory_revision_unavailable` |

> **Phase 2h 实现备注**：当 `character_card_id` 或 `lorebook_path` 显式提供外部/内联源时，不读取 canonical `characters/{cid}/` revision 指针（实际 prompt 内容不来自该目录），对应字段留 `None` 且不推送诊断。scene 模式下 `character/lorebook/state/memory_revision` 均留 `None` 不推送诊断（多角色无单一 revision）。

**回退规则**：asset 目录无 `current_revision` 文件时（旧数据未升级），**禁止**用 mtime、文件名时间戳或 `0` 冒充；必须推送对应 `*_revision_unavailable` 诊断，并在诊断 message 中说明原因（如「asset 未升级到 revision 合同」）。

## 6. 设计决策

### 6.1 决策 D1：以 Preset 模式 + SESSION-DATA-DESIGN.md §4 合同为基底，不以 Persona 为基底

**选项**：

- A. 以 Persona revision（u64 递增 + 乐观锁 + 覆写式存储）为基底，扩展为统一 revision
- B. 以 Preset 版本化（`versions/{generation}/` + `current` 指针 + SHA-256 source_hash + imported_at + converter_version）为基底，对齐 SESSION-DATA-DESIGN.md §4 的 `content_revision: u64` + tree hash 合同
- C. 全新设计，不基于任何现有实现

**选择**：B

**理由**：

1. Persona 缺不可变历史和 content hash；以它为基底会把"只保留当前值"的弱合同复制到所有 asset。
2. Preset 已具备不可变历史快照、content hash、完整 provenance；与 SESSION-DATA-DESIGN.md §4 合同最接近，migration 成本最低。
3. Persona 的 u64 递增形式符合 §4 合同，但仅是乐观锁标记，不构成可回溯历史；保留 u64 形式但补不可变历史即可。
4. C 全新设计违反"不破坏现有合同"原则，且 Preset 已有的版本化资产不应浪费。
5. 独立审计发现 `domain.rs:827` 是幽灵合同（声称 append history.jsonl 但实际未实现），不应作为基底扩展依据。

### 6.2 决策 D2：per-asset-id 独立 revision 空间，session `content_revision` 是复合快照

**选项**：

- A. 所有 asset 共享一个全局 revision 计数器
- B. per-asset-id 独立 revision 计数器（每个 `character_id` / `preset_id` / 等有自己的 r1/r2/r3）
- C. session-scoped 统一 revision（SESSION-DATA-DESIGN.md §4 的 `content_revision`），所有 asset 共享

**选择**：B，并与 C 并存

**理由**：

1. per-asset-id 独立 revision 符合 asset 演化现实：角色卡改 3 次、Preset 改 2 次、Worldbook 改 5 次是独立事件，不应因其中一个改动而 bump 所有 asset 的 revision。
2. SESSION-DATA-DESIGN.md §4 的 `content_revision` 是 session 在某一轮使用的所有 asset 的快照集合版本，引用 per-asset revision；它不是 per-asset revision 的替代。
3. `EffectiveIds` 的 6 个 `*_revision` 字段是 per-asset 槽位，与 B 一致；若选 C 则 `EffectiveIds` 应只有 1 个 `session_revision` 字段，与现有 trace schema 不一致。
4. session manifest 在 §7 单独裁定，本设计的 per-asset revision 是 session manifest 的前置基础设施。

### 6.3 决策 D3：Preset 保留 generation 字符串作为 internal stable id，新增 `content_revision: u64` 对外

**选项**：

- A. 废弃 generation 字符串，全量 migration 为 `content_revision: u64`
- B. 保留 generation 字符串作为 internal stable id（含 timestamp + source_hash），新增 `content_revision: u64` 对外暴露
- C. 用 generation 字符串直接作为 `EffectiveIds.preset_revision`（类型改为 String）

**选择**：B

**理由**：

1. A 强制全量 migration 违反"渐进式 migration"原则，且 `versions/{generation}/` 目录已包含历史数据，重命名风险高。
2. generation 字符串包含 timestamp_nanos 和 source_hash，作为 internal stable id 有诊断价值（可追溯导入时间和大致内容指纹）；保留不浪费。
3. C 违反 SESSION-DATA-DESIGN.md §4 的 `content_revision: u64` 合同，且 `EffectiveIds.preset_revision: Option<u64>` 类型已固定。
4. B 的实现：`presets/{id}/revisions/{content_revision}/manifest.json` 中记录 `source.generation` 字段；`current_revision` 文件存 u64；旧 `current` 文件存 generation 字符串，作为兼容回退。
5. `content_revision` 派生规则：首次升级时 `content_revision = 1`；后续每次 `PresetService::write` 递增 `content_revision`，并记录 `parent_revision` 形成编辑链。

### 6.4 决策 D4：`PromptSegment` 不加 revision 字段，revision 只在 `EffectiveIds` 暴露

**选项**：

- A. `PromptSegment` 新增 `source_revision: Option<u64>` 字段
- B. `PromptSegment` 不加 revision 字段，revision 只在 `EffectiveIds` 暴露

**选择**：B

**理由**：

1. `EffectiveIds` 已暴露 per-asset revision，足以让用户和审计者复现本轮装配输入。
2. `PromptSegment` 是 per-segment 粒度，加 revision 会引入"同一 asset 的多个 segment 是否共享 revision"问题（如角色卡的 `card` intro + `card` details 两个 segment 共享同一 character_revision）。
3. 增加 `source_revision` 会让 trace 体积膨胀，UI 渲染压力上升，与 #199 的 card field 拆分决策耦合。
4. #199 建议在 Phase 2 一并决定角色卡字段是否拆分为独立 segment；本设计裁定：**revision 粒度跟随 asset 粒度，不跟随 segment 粒度**。即使 #199 决定拆分角色卡字段为多个 segment，它们仍共享同一 `character_revision`，通过 `EffectiveIds.character_revision` 暴露。
5. 若未来需要 per-segment provenance（如区分"角色卡 r3 的 personality 字段" vs "角色卡 r3 的 description 字段"），由 #199 单独推进，不在本设计范围。

### 6.5 决策 D5：state revision 范围 — 仅 `state/` 目录，gating 暂不纳入

**选项**：

- A. state revision 覆盖 `state/`（live.json + history.jsonl）+ `gating/`（timeline.md + checkpoints.md + known.md + disclosure.json）
- B. state revision 仅覆盖 `state/`，gating 独立处理或后续纳入
- C. state revision 仅覆盖 `state/history.jsonl` 的 `state` 字段，不含 `live.json`

**选择**：B

**理由**：

1. `state/` 已有 `history.jsonl` + `revision: u64` + append-only 合同，可直接复用为 `content_revision`；升级成本低。
2. `gating/` 是 in-place 修改，无历史快照；若纳入 state revision，需新建 gating 版本化基础设施，工作量与角色卡/Preset/Worldbook 相当，应作为独立阶段。
3. gating 的 `known.md` 已通过 CP-gated 机制进入 prompt（`SystemPromptPart { source_kind: "known" }`），与 state 的 volatile 语义不同；强行统一 revision 会混淆 stable/volatile 边界。
4. C 太窄，`live.json` 是当前值，应纳入 revision 范围以便回滚。
5. 后续若 gating 需要版本化，作为独立 PR 推进，不阻塞本设计。

### 6.6 决策 D6：Persona revision 升级 — 保留 u64 形式，补不可变历史 + content hash

**选项**：

- A. Persona revision 维持现状（u64 + 覆写式存储），不升级
- B. Persona revision 保留 u64 形式，新建 `personas/{id}/revisions/` 目录补不可变历史 + tree hash
- C. Persona 改用 Preset 的 generation 字符串模式

**选择**：B

**理由**：

1. A 不符合"可追溯"目标；Persona 的 base lock / drift / rollback 合同由 #114 推进，但 revision 存储格式升级属本设计范围。
2. C 违反 SESSION-DATA-DESIGN.md §4 的 `content_revision: u64` 合同；Persona 的 u64 形式符合合同，无需改字符串。
3. B 实现路径：`PersonaService::save` 在 `replace_file(personas/{id}.json)` 之后，额外执行 atomic commit 到 `personas/{id}/revisions/{revision}/`；`personas/{id}.json` 保留作为快速读取的当前值（与 `personas/{id}/revisions/{current_revision}/persona.json` 内容一致）。
4. B 的 `EffectiveIds.persona_revision` 填充规则：优先读 `personas/{id}/current_revision`，回退读 `personas/{id}.json` 的 `revision` 字段（兼容旧数据）；两者都不可用时推送 `persona_revision_unavailable` 诊断。
5. B 不实现 base lock / drift / rollback / 头像 / 导入导出；这些属 #114 范围。

### 6.7 决策 D7：乐观锁统一为 `RevisionConflict` 响应

**选项**：

- A. 各 asset service 保留各自冲突响应格式（Persona 用 `PersonaRevisionConflict`，其他 asset 各自定义）
- B. 统一为 `RevisionConflict { asset_kind, asset_id, current_revision }`

**选择**：B

**理由**：

1. A 会导致 HTTP 客户端需要为每类 asset 写不同冲突处理逻辑，违反"更易修正"取向。
2. B 的 `asset_kind` 枚举与 `RevisionManifest.asset_kind` 一致，便于审计。
3. Persona 现有 `PersonaRevisionConflict { current_revision }` 作为 schema=1 兼容格式保留一个迁移窗口，之后迁移到 B；迁移由 §8.2 lazy migration 处理。

### 6.8 决策 D8：Worldbook 与 Character card 的 revision 关系 — 独立 revision

**选项**：

- A. Worldbook 嵌入 Character card revision（角色卡 revision 包含 world/lorebook.json）
- B. Worldbook 独立 revision，与 Character card 解耦
- C. Worldbook 在角色级和场景级分别 revision，角色级嵌入 Character card revision

**选择**：B

**理由**：

1. SESSION-DATA-DESIGN.md §3 把 `world/lorebook.json` 放在 `characters/{character_id}/world/` 下，但 §4 的 `revisions/{revision_id}/` 同时固定 `character/` 和 `worldbooks/`，暗示两者是独立的 asset 集合。
2. 角色卡和世界书是独立 asset，各自有自己的导入、编辑、版本演化路径；强行嵌入会让世界书编辑强制 bump 角色卡 revision，违反 per-asset-id 独立原则（D2）。
3. 场景级世界书（`scenes/{scene_id}/world/lorebook.json`）与角色级世界书（`characters/{character_id}/world/lorebook.json`）各自独立 revision。
4. session manifest（§7）在某一轮引用具体的 character revision + lorebook revision + 其他 asset revision，形成复合快照。

## 7. 与 SESSION-DATA-DESIGN.md §4 的关系

SESSION-DATA-DESIGN.md §4 定义 session-scoped `revisions/{revision_id}/manifest.json`，同时固定角色卡 + 世界书集合，是**复合快照**。本设计的 per-asset revision 是**单一 asset 版本**。

### 7.1 关系裁定

- per-asset revision 是 session manifest 的**前置依赖**：session manifest 引用具体的 `character_revision` / `lorebook_revision` / `preset_revision` / `persona_revision` / `state_revision` / `memory_revision`，不重复存储 asset 内容。
- session manifest 的 `character.files` 和 `worldbooks.files` 字段改为引用 per-asset revision 目录的 `tree_sha256`，不重复计算。
- session `content_revision` 与 per-asset revision 独立计数：session 某轮可能引用 character r3 + lorebook r5 + preset r2，session 自身是 r1；下一轮 character 改为 r4，session 变为 r2。

### 7.2 session manifest schema 扩展（示意）

```json
{
  "schema": 1,
  "content_revision": 2,
  "created_at": "<timestamp>",
  "character": {
    "character_id": "alice",
    "character_revision": 4,
    "tree_sha256": "<per-asset revision tree_sha256>"
  },
  "worldbooks": {
    "manifest_path": "worldbooks/manifest.json",
    "books": [
      {
        "worldbook_id": "character-primary",
        "origin": "character",
        "lorebook_revision": 5,
        "tree_sha256": "<per-asset revision tree_sha256>"
      }
    ]
  },
  "preset": {
    "preset_id": "balanced",
    "preset_revision": 2,
    "tree_sha256": "<per-asset revision tree_sha256>"
  },
  "persona": {
    "persona_id": "writer",
    "persona_revision": 7,
    "tree_sha256": "<per-asset revision tree_sha256>"
  },
  "state": {
    "state_revision": 42,
    "tree_sha256": "<per-asset revision tree_sha256>"
  },
  "memory": {
    "memory_revision": 3,
    "tree_sha256": "<per-asset revision tree_sha256>"
  }
}
```

**说明**：session manifest 不再自存角色卡/世界书内容，只引用 per-asset revision 的 `tree_sha256`；完整性校验时按引用去 per-asset revision 目录验证。session manifest 自身的完整性通过**排除 `tree_sha256` 字段的 canonical manifest hash** 校验（与 Phase 2a 的 per-asset manifest 一致：`manifest.json` 作为元数据 sidecar 不纳入自身 tree hash，避免自引用问题）。

### 7.3 实施顺序

- 本设计先交付 per-asset revision 基础设施（§8）。
- session manifest（SESSION-DATA-DESIGN.md §4）作为独立阶段跟进，引用本设计产出的 per-asset revision。
- 在 per-asset revision 全部交付前，session manifest 不实现；现有 session 数据继续使用工作副本读取，trace 通过 `*_revision_unavailable` 诊断声明缺口。

## 8. 实施阶段

### 8.1 Phase 2a：底层 revision 模块（前置依赖）

**交付物**：

- `engine/src/revision/` 模块（`tree_hash.rs` + `manifest.rs` + `atomic.rs`）
- `AIRP-TREE-SHA256-v1` 实现与测试向量
- `RevisionManifest` schema 与加载校验
- atomic commit 流程与失败回滚测试

**验收**：

- 空目录与单文件测试向量匹配 SESSION-DATA-DESIGN.md §4 第 5 条规定值
- atomic commit 在任意步骤注入失败后，`current_revision` 不指向半成品
- 不变量校验拒绝缺失文件、额外文件、hash 不匹配、tree hash 不匹配
- 神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过

### 8.2 Phase 2b：Preset 升级（已有版本化，最低成本）

**交付物**：

- `PresetService::write` 在现有 `versions/{generation}/` + `current` 基础上，新增 `revisions/{content_revision}/` + `current_revision`
- `build_prompt_trace` 填充 `preset_revision`，移除 `preset_revision_unavailable` 诊断（当 `current_revision` 可读时）
- `PresetImportReport` 持久化到 `revisions/{content_revision}/import_report.json` sidecar
- lazy migration：首次 `PresetService::write` 时，为旧 `versions/{generation}/` 目录生成 `revisions/1/` 并写 `current_revision = "1"`

**验收**：

- 新导入的 Preset 同时有 `versions/{generation}/` 和 `revisions/{content_revision}/`
- 旧 Preset（无 `revisions/`）在 `current_revision` 不可读时推送 `preset_revision_unavailable`
- `EffectiveIds.preset_revision` 在新数据上填充实际 u64
- `PresetImportReport` 可通过 `GET /v1/presets/:id/import-report` 读取（已由 #115 Phase 1 规划，本阶段补 sidecar 落盘）
- 现有 Preset 测试全部通过

### 8.3 Phase 2c：Character card 新建版本化

**交付物**：

- `CharacterService::import_card_to_disk` 在现有 `card/card.json` + `card/raw.json` 基础上，新增 `revisions/{content_revision}/` + `current_revision`
- `CharacterService::update_character_card`（PUT）执行 atomic commit，不再直接覆盖 `card/raw.json`
- `card/raw.json` 在工作副本中保留为可编辑副本；导入时的原始 sidecar 改存 `revisions/1/raw.json`，PUT 不覆盖该 revision
- `build_prompt_trace` 填充 `character_revision`
- `provenance.json` sidecar 落盘到 `revisions/{content_revision}/provenance.json`，记录 source_kind / source_hash / source_filename / imported_at

**验收**：

- 新导入的角色卡有 `revisions/1/` + `current_revision = "1"`
- PUT 更新产生 `revisions/2/`，`current_revision = "2"`，`revisions/1/` 不可变
- `EffectiveIds.character_revision` 填充实际 u64
- 导入时的原始 card bytes 可从 `revisions/1/raw.json` 恢复
- 现有角色卡测试全部通过

### 8.4 Phase 2d：Worldbook 新建版本化

**交付物**：

- `LorebookService::write` 在现有 `world/lorebook.json` 基础上，新增 `world/revisions/{content_revision}/` + `world/current_revision`
- 角色级和场景级世界书各自独立 revision 空间
- `WorldbookImportReport` 补 `source_hash` / `imported_at` / `converter_version` 字段，对齐 `PresetImportReport` schema
- `WorldbookImportReport` 持久化到 `revisions/{content_revision}/import_report.json` sidecar
- `build_prompt_trace` 填充 `lorebook_revision`，新增 `lorebook_revision_unavailable` 诊断

**验收**：

- PUT 更新世界书产生新 revision，旧 revision 不可变
- `WorldbookImportReport` schema 与 `PresetImportReport` 对称（都有 source_hash / imported_at / converter_version）
- `EffectiveIds.lorebook_revision` 填充实际 u64
- 现有 Worldbook 测试全部通过

### 8.5 Phase 2e：State 升级（复用 history.jsonl）

**交付物**：

- `StateService::write` 在现有 `state/live.json` + `state/history.jsonl` 基础上，新增 `state/revisions/{content_revision}/state.json` + `state/current_revision`
- `state/revisions/{content_revision}/state.json` 内容 = 对应 `history.jsonl` 行的 `state` 字段
- `state/current_revision` = `history.jsonl` 末行 `revision`
- `build_prompt_trace` 填充 `state_revision`，新增 `state_revision_unavailable` 诊断

**不包含**：gating 版本化（见 D5）

**验收**：

- `StateService::write` 产生新 revision 时，`revisions/{revision}/state.json` 和 `history.jsonl` 末行一致
- `EffectiveIds.state_revision` 填充实际 u64
- 现有 State 测试全部通过

### 8.6 Phase 2f：Memory 新建版本化

**交付物**：

- `VolumeStore` 在 `current.md` / `index.md` / `volumes/` 基础上，新增 `memory/revisions/{content_revision}/` + `memory/current_revision`
- `append_to_current` 不再直接 append；改为：复制 `current.md` 到 staging、append 新内容、atomic commit 为新 revision、更新 `current.md` 和 `current_revision`
- `write_volume`（封卷）产生新 revision
- `build_prompt_trace` 填充 `memory_revision`，新增 `memory_revision_unavailable` 诊断

**验收**：

- `append_to_current` 产生新 revision，旧 revision 的 `current.md` 不可变
- `EffectiveIds.memory_revision` 填充实际 u64
- 现有 Memory 测试全部通过

### 8.7 Phase 2g：Persona revision 升级

**交付物**：

- `PersonaService::save` 在现有 `replace_file(personas/{id}.json)` 基础上，新增 `personas/{id}/revisions/{revision}/persona.json` + `personas/{id}/current_revision`
- `personas/{id}.json` 保留作为快速读取的当前值（与 `revisions/{current_revision}/persona.json` 内容一致）
- `build_prompt_trace` 在 `personas/{id}/current_revision` 不可读时推送 `persona_revision_unavailable` 诊断
- 乐观锁冲突响应迁移到统一 `RevisionConflict { asset_kind: "persona", asset_id, current_revision }`，`PersonaRevisionConflict` 作为 schema=1 兼容格式保留一个迁移窗口

**不包含**：base lock / drift / rollback / 头像 / 导入导出（属 #114）

**验收**：

- `PersonaService::save` 产生新 revision 时，`revisions/{revision}/persona.json` 不可变
- `EffectiveIds.persona_revision` 填充实际 u64
- 冲突响应格式为 `RevisionConflict`（新数据）或 `PersonaRevisionConflict`（旧数据兼容窗口）
- 现有 Persona 测试全部通过

### 8.8 Phase 2h：trace 完整性收口

**交付物**：

- `build_prompt_trace` 全部 6 个 `*_revision` 字段按 §5.3 规则填充
- 6 个 `*_revision_unavailable` 诊断在 asset 未升级时全部推送
- WebUI 装配预览面板展示 6 个 revision（含 unavailable 标识）
- `production-browser-smoke.mjs` 覆盖 6 个 revision 字段的渲染断言

**验收**：

- 新数据上 trace 全部 6 个 revision 填充实际 u64
- 旧数据上 trace 推送对应 `*_revision_unavailable` 诊断，且 WebUI 明确显示 "unavailable"
- 神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过
- `production-browser-smoke.mjs` 全部通过

## 9. 风险与回滚

### 9.1 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| atomic commit 在 Windows 上 `rename` 失败（目标目录非空） | revision 无法发布 | staging 目录先 `create_dir_all`，目标目录不存在时 `rename`；目标已存在时拒绝（revision 已存在） |
| Preset `versions/{generation}/` 与 `revisions/{content_revision}/` 双指针不一致 | 读者看到不同版本 | `current_revision` 为权威；`current`（generation）仅兼容回退；加载器优先读 `current_revision` |
| lazy migration 在并发写入下产生 revision 竞争 | 两个写入者同时生成 revision 1 | per-asset service 已有 Mutex（Persona / Preset）；Character / Worldbook / Memory 需补 per-asset-id Mutex |
| `AIRP-TREE-SHA256-v1` 实现与 SESSION-DATA-DESIGN.md 测试向量不匹配 | 完整性校验拒绝合法 revision | Phase 2a 先交付测试向量匹配，再进入 Phase 2b |
| Persona `PersonaRevisionConflict` 迁移到 `RevisionConflict` 破坏 HTTP 客户端 | 客户端冲突处理失效 | 保留 schema=1 兼容格式一个迁移窗口；HTTP 响应同时含旧字段和新字段 |
| Worldbook 角色级 + 场景级双 revision 空间复杂度 | `lorebook_revision` 来源歧义 | trace 填充时按实际装配来源（角色级或场景级）填充；`PromptSegment.source_id` 已区分 character_id / scene_id |
| gating 不纳入 state revision 导致 CP-gated known 信息无版本 | 已知信息回滚不一致 | 已知信息变更频率低；gating 版本化作为独立阶段跟进，不阻塞本设计 |

### 9.2 回滚

- 每个 Phase 独立 PR，可独立回滚。
- Phase 2a 底层模块未接入任何 asset service 前可独立合并；接入后回滚需同时回滚对应 asset service 改动。
- 旧数据未升级时仍可正常读取（`*_revision_unavailable` 诊断声明缺口）；回滚后 trace 回到当前状态。
- Preset `versions/{generation}/` + `current` 旧指针不删除；回滚后仍可读取。
- Persona `personas/{id}.json` 不删除；回滚后仍可读取。
- State `history.jsonl` 不删除；回滚后仍可读取。

## 10. 验收标准（整体）

- [ ] `AIRP-TREE-SHA256-v1` 实现与 SESSION-DATA-DESIGN.md §4 测试向量匹配
- [ ] 5 类 asset（Preset / Character / Worldbook / State / Memory）+ Persona 全部有 `revisions/` + `current_revision` 目录结构
- [ ] `EffectiveIds` 全部 6 个 `*_revision` 字段在新数据上填充实际 u64
- [ ] 6 个 `*_revision_unavailable` 诊断在旧数据上推送
- [ ] 不变量校验拒绝缺失文件、额外文件、hash 不匹配、tree hash 不匹配
- [ ] atomic commit 在任意步骤失败后 `current_revision` 不指向半成品
- [ ] 乐观锁统一为 `RevisionConflict` 响应（Persona 保留 schema=1 兼容窗口）
- [ ] WebUI 装配预览面板展示 6 个 revision
- [ ] `production-browser-smoke.mjs` 覆盖 6 个 revision 字段渲染
- [ ] 现有 Preset / Character / Worldbook / State / Memory / Persona 测试全部通过
- [ ] 神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过
- [ ] 不用 mtime、文件名时间戳或单调递增计数器冒充内容版本

## 11. 与 issue #199 的关系

issue [#199](https://github.com/GhostXia/AIRP/issues/199) 是 PR #194 审计遗留：决定 orchestrator card field 是否拆分为独立 `SystemPromptPart`。

本设计 §6.4 裁定：**revision 粒度跟随 asset 粒度，不跟随 segment 粒度**。即使 issue #199 决定拆分角色卡字段为多个 segment，它们仍共享同一 `character_revision`。

issue #199 的决策可在本设计交付后独立推进；本设计不依赖 issue #199 的决策结果。

## 12. 与 issue #114 的关系

issue [#114](https://github.com/GhostXia/AIRP/issues/114) 是 Persona / Preset 高级生命周期：base lock / drift / rollback / 头像 / 导入导出 / Preset revision/collision/overwrite/provenance 合同。

本设计 §6.6 只升级 Persona revision 存储格式（补不可变历史 + content hash），不实现 base lock / drift / rollback / 头像 / 导入导出。

issue #114 的 Persona 高级生命周期可在本设计 Phase 2g 交付后，基于 `personas/{id}/revisions/` 基础设施推进；drift overlay 可作为 `personas/{id}/drift/` 独立目录，与 revision 目录解耦。

## 13. 不在范围

明确不在本设计范围的事项：

- session 自包含存档完整实现（SESSION-DATA-DESIGN.md §5 生命周期、§7 分阶段实施）
- session manifest 完整实现（§7 仅定义关系，实施由 session 数据设计跟进）
- Persona base lock / drift / rollback / 头像 / 导入导出（属 #114）
- Preset import dry-run / collision preview / overwrite 显式确认（属 #115 Phase 1 未完成项）
- NextPayloadCapture / tool-call/result history integrity / preset tool restriction（属 #115 P2/P3）
- 第三方世界书素材库（SESSION-DATA-DESIGN.md §5.1、§7.3）
- gating 版本化（§6.5 裁定为独立阶段）
- #199 orchestrator card field 拆分（§6.4 裁定为独立决策）
- Tauri/Vue 桌面 UI 的 revision 展示（桌面路线暂停）

## 14. 参考资料

- [CURRENT-BASELINE.md](../../CURRENT-BASELINE.md) §3、§4
- [SESSION-DATA-DESIGN.md](../../SESSION-DATA-DESIGN.md) §4、§5.3、§7
- [PLAN.md](../../PLAN.md) §2 不变式、§4.1 产品能力支柱
- [#115 issue body](https://github.com/GhostXia/AIRP/issues/115) Phase 2 范围
- [#115 PR #194 合并后审计遗留归档](https://github.com/GhostXia/AIRP/issues/115#issuecomment-4990498643)
- [#199 orchestrator card field 拆分](https://github.com/GhostXia/AIRP/issues/199)
- [#114 Persona / Preset 高级生命周期](https://github.com/GhostXia/AIRP/issues/114)
- PR #196 scene lorebook provenance（merged）
- PR #194 prompt assembly preview（merged）
- PR #177 PromptAssemblyTrace 数据模型 skeleton（merged）
- PR #176 Preset `versions/{generation}/` + `current` 指针（merged）
