# Session 存档与世界书物化设计

> 状态：**产品方向已确认，分阶段实施**
>
> 关联：[Issue #168](https://github.com/GhostXia/AIRP/issues/168)
>
> 最后更新：2026-07-15

本文定义 AIRP 的 session、第三方世界书和可复现游玩存档边界。它是 AIRP 根据自身需求形成的独立设计；SillyTavern 仅作为公开行为与互操作性参考，不复用其实现代码、规则、测试或资产。

## 1. 已确认的产品语义

- 一个 `{session_id}` 就是一个独立“开局”或“存档槽位”。同一角色的“开局 1”和“开局 2”使用不同的稳定 session ID。
- “开局 1”“二周目”等是可修改的显示标题，保存在 session 元数据中；重命名不移动目录。
- session 不只是聊天 JSONL。完整目标是让聊天、记忆、状态、剧情进度和本局实际使用的全部世界书共同构成可复制、备份、魔改和恢复的游玩存档。
- 第三方世界书先进入独立素材库；被某局采用时复制并归一化到该 session，运行时不再依赖素材库中的活动文件。
- session 内副本允许自由修改，但不自动回写角色卡或第三方素材库。提升为角色默认或另存为素材必须是显式操作。

## 2. 身份与命名

目录只使用经过路径校验的稳定 ID：

- `character_id`：角色身份；不使用可能重名、变化或含危险字符的显示名。
- `session_id`：存档槽位身份；建议保持当前 UUID 合同。
- `worldbook_id`：session 内世界书身份；显示名、原始文件名和来源名称只作元数据。

`session_id` 也是该局聊天历史和记忆的唯一规范 UUID。`history/`、`memory/` 是其直接子目录，`chat_log_meta.json` 必须记录同一个 ID；不得再生成第二个内部“聊天 session ID”，也不得在两个子目录下重复嵌套相同 UUID。

最小 session 元数据示例：

```json
{
  "schema": 1,
  "session_id": "<stable-id>",
  "character_id": "<stable-id>",
  "title": "开局 1",
  "created_at": "<timestamp>",
  "updated_at": "<timestamp>",
  "initial_revision": 1,
  "current_revision": 1
}
```

## 3. 目标目录合同

以下路径相对于 effective data root。单用户模式就是 `data/`；多用户请求则位于对应 `users/{user_id}/` 根下。

```text
<effective-root>/
├── characters/{character_id}/
│   ├── card/
│   ├── world/
│   │   └── lorebook.json          # 角色默认素材，不是活跃 session 的唯一真值
│   └── sessions/{session_id}/
│       ├── meta.json
│       ├── history/
│       │   ├── chat_log.jsonl
│       │   └── chat_log_meta.json
│       ├── memory/
│       ├── state/
│       ├── character/             # 本局可编辑的角色卡工作副本
│       │   ├── card.json
│       │   ├── card.png           # 可选；存在头像/卡图时保留
│       │   ├── greetings/
│       │   └── provenance.json
│       ├── worldbooks/
│       │   ├── manifest.json      # 本局加载清单与顺序的唯一真值
│       │   ├── character/         # 从角色默认素材物化
│       │   ├── third_party/       # 从第三方素材库物化
│       │   └── session/           # 本局新建或完全自定义
│       └── revisions/
│           └── {revision_id}/      # 角色卡 + 世界书的不可变逻辑快照
│               ├── manifest.json
│               ├── character/
│               └── worldbooks/
└── third_party/
    └── worldbooks/{source_id}/{package_id}/{version}/
        ├── raw/                    # 原始导入文件，只保留 provenance
        ├── lorebook.json           # AIRP 归一化素材
        └── provenance.json
```

实现可在内部增加内容寻址去重，但用户可见、可备份的 session 必须保持自包含；不能要求恢复工具再访问外部缓存才能还原一局。

## 4. 统一 revision 与世界书 manifest

`revisions/{revision_id}/manifest.json` 是恢复和导出的统一真值，必须同时固定角色卡快照与世界书集合，不能只记录世界书版本。最小结构示例：

```json
{
  "schema": 1,
  "revision": 3,
  "created_at": "<timestamp>",
  "character": {
    "path": "character/",
    "tree_sha256": "<snapshot-tree-hash>"
  },
  "worldbooks": {
    "path": "worldbooks/",
    "manifest_path": "worldbooks/manifest.json",
    "tree_sha256": "<snapshot-tree-hash>"
  }
}
```

manifest 中的路径相对于该 revision 目录；tree hash 覆盖目录内全部文件，包括 greetings、卡图、provenance 和世界书 manifest。revision 目录内的 `character/` 和 `worldbooks/` 是不可变快照；session 根下同名目录是当前工作副本。实现可以使用内容寻址去重，但完整 session 导出后仍须在无外部缓存时还原每个 revision。

统一 revision manifest 的 `revision` 是唯一权威的内容版本标识。加载器和导出器必须校验 `revisions/{revision_id}/manifest.json.revision == meta.json.current_revision`（读取当前版本时），并校验该 revision 内 `worldbooks/manifest.json.content_revision == manifest.json.revision`；目录名 `{revision_id}` 也必须是同一标识的规范字符串。任一值不一致都属于完整性错误，禁止猜测、静默选择较新值或回退到工作副本。

运行时只读取 `worldbooks/manifest.json` 列出的文件，不能靠递归扫描或文件名猜测加载顺序。建议的最小记录为：

```json
{
  "schema": 1,
  "content_revision": 3,
  "books": [
    {
      "worldbook_id": "character-primary",
      "path": "character/primary.json",
      "origin": "character",
      "enabled": true,
      "order": 100,
      "sha256": "<content-hash>"
    },
    {
      "worldbook_id": "third-party-example",
      "path": "third_party/example.json",
      "origin": "third_party",
      "source_id": "<source/package/version>",
      "enabled": true,
      "order": 200,
      "sha256": "<content-hash>"
    }
  ]
}
```

`content_revision` 是权威 `revision` 在世界书 manifest 中的冗余完整性字段，必须满足上一节的相等不变量，不能再形成独立的世界书版本身份。`origin` 表示来源，不赋予更高或更低优先级。实际装配顺序由显式 `order` 和 AIRP 世界书运行时合同决定。

## 5. 生命周期

### 5.1 导入第三方世界书

1. 在临时区读取原始文件并执行大小、类型和结构校验。
2. 保存原始文件、hash、来源、导入时间、原文件名、上游版本和已知许可证信息。
3. 通过 AIRP shared normalizer 生成规范化 `lorebook.json`。
4. 第三方内容始终视为数据；不得执行其中的脚本、HTML 或其他主动内容。

### 5.2 新建开局

1. 生成稳定 `session_id` 和默认显示标题。
2. 创建独立 history、memory、state、character、worldbooks 和 revisions 目录。
3. 把角色卡及 greetings/卡图复制为 `character/` 工作副本，并记录来源 provenance。
4. 把角色默认、用户选择的第三方、场景、Persona 或其他启用世界书复制为 `worldbooks/` 工作副本。
5. 同时快照角色卡和世界书，写入统一 revision 1；此后本局 prompt 装配只读取 session 工作副本。

### 5.3 游玩中修改或新增世界书

- 用户可直接编辑 session 工作副本，也可通过 UI/API 引入新世界书。
- 发送下一条消息前计算角色卡与世界书集合 hash；任一发生变化时生成新的统一 revision。
- 每条新写入的 `history/chat_log.jsonl` 消息记录都必须包含 `content_revision`；同一轮的 user/assistant 消息使用同一值。该字段与对应消息位于同一个 JSONL 对象中，由一次追加写入共同落盘，不使用可与消息分离提交的旁路索引。发送模型请求前先确定并持久化 user 消息及其 revision；生成结果再以同一 revision 追加 assistant 消息。旧记录缺少该字段时标记为“版本未知”，不得用当前 revision 冒充。这样即使一轮中途失败，历史回放仍能从已落盘消息准确选择当时的角色卡与世界书快照。
- 更新第三方素材库不得自动覆盖已存在 session；用户必须显式选择刷新、比较或重新物化。

### 5.4 复制、提升与导出

- 复制 session 等价于从当前进度分叉出新世界线，新 session 获得独立 ID。
- “提升为角色默认世界书”“另存为自建素材”都是显式、可审计操作。
- 导出完整存档必须包含 meta、history、memory、state、character、worldbooks、revisions 和 provenance，不依赖原机器的素材库。

### 5.5 用户兜底与派生角色卡

session 必须同时承担“可自由魔改的工作分支”和“可恢复的用户兜底”两种职责：

- 创建 session 时写入不可变的初始快照（revision 1），其 manifest 同时固定当时采用的角色卡和全部世界书；后续编辑只能产生新 revision，不能原地覆盖初始快照。
- 原始角色卡、角色默认世界书、第三方素材库和 session 初始快照互相独立。用户即使大范围改坏本局设定，也能恢复到开局时的原始版本。
- revision 必须记录内容 hash；恢复、提升为角色默认、另存为素材和导出均为显式操作，禁止自动回写来源。
- 用户可从任意 `{session_id}` 直接生成一张新的派生角色卡。导出器以选定 revision 内不可变的角色卡与世界书快照及必要 provenance 为输入，不再追踪或读取 session 当前工作副本或原机器上的外部世界书。
- 派生角色卡默认不包含聊天记录、记忆和游玩状态；这些内容属于完整 AIRP session 存档。若用户要备份或复盘整个世界线，应导出包含 meta、history、memory、state、character、worldbooks、revisions 和 provenance 的 session 存档包。

因此产品语义是：原始角色卡是模板，session 是可恢复、可魔改的工作分支，从 session 导出的派生角色卡则是一张新的独立模板。

## 6. 兼容与迁移

- 当前命名 session 已隔离 `history/` 与 `memory/`，但角色 state 和默认世界书仍在角色级目录；因此当前实现尚不是完整自包含存档。
- 未提供 `session_id` 的旧调用继续使用角色级 history/memory，直到单独的兼容迁移方案落地。
- 已存在的根级 `world.md`、`items.md` 和 legacy `worldbooks/` 不自动删除或覆盖。
- 迁移遇到新旧位置均有内容时必须保留两者并生成报告，禁止静默覆盖。

## 7. 分阶段实施

1. **目录去歧义与设计存档**：停止为新数据根创建根级 `world.md`/`items.md`，停止为新角色创建 legacy `worldbooks/`，记录本文合同。
2. **完整 session 边界**：新增 `meta.json`，把 state、剧情进度和角色卡工作副本隔离到命名 session，并为旧调用保留兼容路径。
3. **第三方世界书素材库**：实现安全导入、raw/normalized/provenance、稳定 ID 和查询 API。
4. **session 世界书物化**：创建 manifest、复制全部启用世界书、让 prompt 装配只读 session 副本。
5. **revision 与产品操作**：用统一 manifest 同时固定角色卡和世界书，保留不可变初始快照并记录每轮 `content_revision`，支持恢复原始版本、复制开局、刷新来源、比较差异、提升默认、从 session 导出派生角色卡，以及完整 session 导出/恢复。

后续 PR 不得把本文的目标目录写成当前已交付能力；每完成一阶段，再同步 `CURRENT-BASELINE.md` 和相应验收证据。
