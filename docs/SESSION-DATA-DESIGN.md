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
- `session_id`：存档槽位身份；必须是通过格式校验的 UUID。
- `worldbook_id`：session 内世界书身份；显示名、原始文件名和来源名称只作元数据。

对命名 session，`session_id` 是该局唯一规范 UUID：`sessions/{session_id}` 的目录段、`meta.json.session_id`、`history/chat_log_meta.json.session_id`，以及任何历史、记忆、状态或导出记录中实际存在的 `session_id` 字段都必须使用同一个经校验的值。`history/` 和 `memory/` 只是该目录下的固定子目录，本身不承载 UUID。所有命名 session 的创建、读取、更新、复制、导入和导出边界都必须接收并持久化该 UUID，不得接受其他 ID 形式或再生成内部聊天 ID。

兼容豁免仅限未提供 `session_id` 的 legacy 角色级调用：它们继续读取 `characters/{character_id}/history/` 和角色级 memory，并可在旧 `chat_log_meta.json` 中保留历史聊天 ID。这类数据不是命名 session，也不能直接作为自包含 session 复制或导出；只有显式迁移分配并验证新 UUID 后，才进入上述一对一合同。已提供 UUID 的命名 session 不适用该豁免。

最小 session 元数据示例：

```json
{
  "schema": 1,
  "session_id": "<uuid>",
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
  "content_revision": 3,
  "created_at": "<timestamp>",
  "character": {
    "path": "character/",
    "files": [
      { "path": "card.json", "sha256": "<content-hash>" },
      { "path": "provenance.json", "sha256": "<content-hash>" }
    ],
    "tree_sha256": "<snapshot-tree-hash>"
  },
  "worldbooks": {
    "path": "worldbooks/",
    "manifest_path": "worldbooks/manifest.json",
    "files": [
      { "path": "manifest.json", "sha256": "<content-hash>" },
      { "path": "character/primary.json", "sha256": "<content-hash>" },
      { "path": "third_party/example.json", "sha256": "<content-hash>" }
    ],
    "tree_sha256": "<snapshot-tree-hash>"
  }
}
```

manifest 中的路径相对于该 revision 目录；tree hash 覆盖目录内全部文件，包括 greetings、卡图、provenance 和世界书 manifest。revision 目录内的 `character/` 和 `worldbooks/` 是不可变快照；session 根下同名目录是当前工作副本。实现可以使用内容寻址去重，但完整 session 导出后仍须在无外部缓存时还原每个 revision。

所有位置统一使用字段名 `content_revision`，不存在第二个 `revision` 身份。加载器和导出器必须校验 `revisions/{revision_id}/manifest.json.content_revision == worldbooks/manifest.json.content_revision`；读取当前版本时还必须等于 `meta.json.current_revision`。目录名 `{revision_id}` 是同一非负整数的无前导零十进制字符串。任一值不一致都属于完整性错误，禁止猜测、静默选择较新值或回退到工作副本。

`tree_sha256` 使用统一的 `AIRP-TREE-SHA256-v1` 算法：

1. revision manifest 中每个 `files` 数组是对应快照子树的唯一批准文件集。构建器必须在空 staging 目录中只物化应用层明确提供的文件，再生成 `files`；不得从工作目录扫描并自动纳入未知文件。加载器必须确认磁盘上的普通文件集合与 `files` 完全相等，缺失或额外文件都属于完整性错误。因此批准集合内不排除任何文件，而临时文件和系统元数据因为未获批准不能进入已提交快照。
2. 只接受目标快照子树内的普通文件；符号链接、junction/reparse point、设备文件和其他特殊节点一律拒绝。
3. 每个相对路径必须使用 `/` 分隔、不得含空段、`.`、`..` 或反斜杠，并且必须已经是 Unicode NFC。路径按其 UTF-8 字节序升序排列；任何重复路径都属于错误。
4. SHA-256 输入先写入 ASCII 域分隔符 `AIRP-TREE-SHA256\0v1\0`，随后为每个批准文件依次写入 `u64be(path_utf8_length) || path_utf8 || u64be(file_length) || raw_file_bytes`。长度均为无符号 64 位大端整数；文件内容不得做换行、BOM 或文本编码转换。
5. `sha256` 文件字段是原始文件字节的 SHA-256 小写十六进制值；`tree_sha256` 是上述完整输入摘要的小写十六进制值。快照构建器和全部加载、导出、导入校验必须复用同一文件选择与哈希实现及测试向量，不得按平台另行解释。

规范测试向量：空目录的摘要为 `a9682729b0a5609f08a1c9a8b2bf49b68edb9056d9e910fd297f694cc3ee3dbf`；仅含路径 `a.txt`、内容为单字节 ASCII `x` 的目录摘要为 `cfa2887973ce5ecc1f2bc57b00ad0130a39aae4d4bf67adae0431ccd3a3ae189`。

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

世界书 manifest 中的 `content_revision` 是统一内容版本在该文件中的冗余完整性字段，必须满足上一节的相等不变量，不能再形成独立的世界书版本身份。`origin` 表示来源，不赋予更高或更低优先级。实际装配顺序由显式 `order` 和 AIRP 世界书运行时合同决定。

组装任何一轮 prompt 前，加载器必须先完成整体验证，不能边验证边部分加载：规范化每个 `books[].path`，确认它是相对于所选 revision 的 `worldbooks/` 根目录且解析后仍被该根目录包含，拒绝绝对路径、父级跳转、符号链接和非普通文件；确认路径存在于统一 revision manifest 的 `worldbooks.files` 批准集合；校验文件原始字节同时匹配 `books[].sha256` 与 `worldbooks.files` 中对应的 `sha256`；最后重新计算并比对整个 `worldbooks.tree_sha256`（包括 `manifest.json`）。任一步失败都必须在装配前拒绝该 revision，禁止回退到可变工作副本或只加载通过的部分条目。

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
5. 同时快照角色卡和世界书，发布统一 revision 1；工作副本只作为用户编辑面，此后每轮 prompt 装配读取并验证该轮选定的不可变 revision。

### 5.3 游玩中修改或新增世界书

- 用户可直接编辑 session 工作副本，也可通过 UI/API 引入新世界书。
- 发送下一条消息前计算角色卡与世界书集合 hash；任一发生变化时生成新的统一 revision。构建器必须先在 `revisions/` 下的同文件系统 staging 目录写完批准文件、两个 manifest 和哈希，逐文件 `sync_data`，同步 staging 目录并完成全量校验；随后以不覆盖既有目标的原子 rename 发布为 `{revision_id}/`，再同步 `revisions/` 父目录。发布成功后通过原子替换更新并同步 `meta.json.current_revision`。只有这些步骤全部成功，才允许追加任何引用该 `content_revision` 的 user 或 assistant 消息；失败只会留下不被引用的 staging/orphan revision，消息绝不能指向缺失或半成品快照。
- 每条新写入的 `history/chat_log.jsonl` 消息记录都必须包含 `content_revision`；同一轮的 user/assistant 消息使用同一值。该字段与对应消息位于同一个 JSONL 对象中，由一次追加写入共同落盘，不使用可与消息分离提交的旁路索引。发送模型请求前先确定并持久化 user 消息及其 revision；生成结果再以同一 revision 追加 assistant 消息。旧记录缺少该字段时标记为“版本未知”，不得用当前 revision 冒充。这样即使一轮中途失败，历史回放仍能从已落盘消息准确选择当时的角色卡与世界书快照。
- 更新第三方素材库不得自动覆盖已存在 session；用户必须显式选择刷新、比较或重新物化。

JSONL 的目标崩溃恢复合同如下；当前运行时尚未实现这些同步与恢复语义，必须在第 7 节 revision 阶段一并落地后才可宣称逐轮可复现：

- 在 session 写锁内把一条完整 JSON 对象及结尾 `\n` 组装为单个字节缓冲区，`write_all` 后执行 `sync_data`；只有同步成功才能确认该消息已持久化。user 与 assistant 各自形成独立同步边界，因此生成中断时可以只存在已确认的 user 消息。
- 加载器按字节偏移扫描。文件末尾唯一一个没有 `\n` 的片段一律视为 torn tail，不参与回放，也不标记为“版本未知”；必须报告偏移、长度和 SHA-256，并保留原始字节供显式修复。
- 已由 `\n` 终止但无法解析的记录属于中段损坏。加载器记录恢复诊断并继续扫描后续完整记录，使后续消息仍可回放；不得静默删除、改写损坏行或用当前 `content_revision` 填充。
- 修复操作必须先把原始 JSONL 和诊断复制到 `history/recovery/`，再通过原子替换写入仅含可验证记录的新文件。普通读取不得截断或改写历史。

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
5. **revision 与产品操作**：用统一 manifest 同时固定角色卡和世界书，落地规范 tree hash、JSONL 同步/崩溃恢复并记录每轮 `content_revision`，支持恢复原始版本、复制开局、刷新来源、比较差异、提升默认、从 session 导出派生角色卡，以及完整 session 导出/恢复。

后续 PR 不得把本文的目标目录写成当前已交付能力；每完成一阶段，再同步 `CURRENT-BASELINE.md` 和相应验收证据。
