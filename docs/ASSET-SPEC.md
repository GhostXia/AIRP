# AIRP 资产规格策略（角色卡/世界书/预设的自有开放规格）

> 用户 2026-07-02 提问：导入的预设/卡/世界书有很多参数对我们无用，应内部自动剔除；要不要建一个"开源的、未来可增删参数的、我们自己的规格标准"？
> **决策：建——但它本质是 engine 数据层模型的正式化/版本化/文档化形态（非另起炉灶），且守两条硬规则。**
> 关联：[MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)（数据模型=engine 规格）· [PLAN.md §5](PLAN.md)（导入解耦重组）· [DEV-GUIDE.md §5](DEV-GUIDE.md) · `protocol/`（UI 线协议，与本"数据/资产规格"互补、非同一物）。
> 最后更新：2026-07-02

---

## 定位
**AIRP 资产规格 = 我们 engine 数据层的 canonical 模型，正式化 + 版本化 + 文档化 + 开源。** 我们本就在通过融入 MCP-Server 的域模型建这个数据层——把它命名、定版本、写文档、开源，就成了"我们自己的规格"。**不是发明新格式，是把已在建的内部模型立为标准。**

价值：① engine 数据层 + agent 工具的稳定契约；② 第三方扩展/工具/widget 读写的目标 schema（§3.8 扩展生态 + agentskills.io）；③ 版本化可增删字段，生态可成长；④ 对标 Character Card V2/V3 那样的开放社区规格。

## 两条硬规则（不可破）

### 规则1 · 超集现有开放规格，绝不重新发明
- **建在 Character Card V3 之上**（V3 本就开放 + 可扩展：`extensions` / `assets` / `creator_notes_multilingual` / spec 版本），+ ST World Info + 预设格式。
- 导入 = **归一化（normalize）到 canonical 模型**，不是翻译成不兼容新格式。
- **保持可导出 / round-trip**——用户能把资产导回酒馆生态，不被我们困死。
- 理由：最大化兼容 = 最大化用户资产可迁入 + 不锁定 = 符合"无缝支持第三方"（§3.8）。

### 规则2 · "剔除" ≠ "销毁"（分两层，最易踩的坑）
- **存储层：全部保留。** canonical 字段 + **原始 raw** + **未知/extension 字段 → passthrough sidecar**。依据 = MCP-Server 自有规矩（`SKILL.md §16/§15.5`：未知捆绑内容原样旁路 sidecar、不解析不删——"可能是第三方工具标记语法"；`[UNKNOWN_ORIGIN]` 标记待查不删）+ V3 extensions 哲学。**永不丢数据**，保 re-export + 第三方扩展数据存活。
- **工作模型 + 装配层：只用我们用的。** engine 活动模型只建模我们实际使用的字段；orchestrator 只把**相关子集**注入角色平面。
- **用户说的"自动剔除无用参数"精确定位**：在**装配层剔**（无用参数不进 prompt = 干净提示词 §1）+ **活动模型只建模有用的**；**存储层绝不删**（sidecar 留全）。剔的是"进 prompt / 进活动逻辑"的，不是"进磁盘"的。

## 导入流程（两条轨 —— 别把"重组"交给 Agent）
```
导入文件（路径，path-first，守不变式6）
├─ 主干【引擎·代码·确定性·无损·快】 ← "拆解 + 重组成规格文件"是这条
│   读盘 → 解析(png_parser/JSON) → 归一化到 canonical(V3 超集)
│        → 未知/extension 字段进 passthrough sidecar → 存
└─ 可选【Agent·语义·按需·旁路】 ← "Agent 分析"是这条
    analyze_card(性格/推断 state schema) / decompose / validate(未知宏)
        → 产物进 analysis/ sidecar，不动主干规格文件
```
- **"重组成规格文件"= 代码，不是 Agent**：本质是确定性字段映射（V3→canonical，未知进 sidecar）。代码干=快/无损/可复现/免费。**若让 Agent 做 = 把整张卡/整本世界书灌进模型 = 违反不变式6（烧 token）+ 慢 + 不确定 + 可能改坏数据。**
- **Agent 分析 = 可选语义增强层**：按需、非必经（不分析也能用）；跑在**已解析的结构化数据**上，校验未知宏只喂**可疑片段**、不喂整文件；产物旁路进 `analysis/` sidecar。
- **预设特殊（§3.3）**：预设是运行时给 Agent 的**建议素材**——导入只**存**，Agent 在**使用时**按当前模型适配，**不在导入时分析**。
- 一句话：**导入=代码确定性归一化成规格文件(+sidecar 留全)；Agent 分析是可选/按需/旁路的语义层，不在导入主干、不看原始大 blob。**

## 可扩展 / 版本化
- 规格带版本号（如 `airp_asset_spec: "1"`），字段增删走版本 + 迁移（类比 `protocol/` 的协议 v + State-Protocol 迁移纪律）。
- 开源（MIT/Apache，与仓库一致），发布为社区可引用的 schema（JSON Schema 真相 + Rust/TS 绑定，同 `protocol/` 模式）。
- 第三方按此 schema 扩字段（走 extensions 命名空间，不撞核心字段——类比 widget `namespace.name`）。

## 落地时机（不前置）
- **别在导入建成前先写死 spec**——canonical 模型的字段要**从真实酒馆文件 + 实际使用中长出来**。
- 增量：Task 1.1（卡）→ 1.3（世界书）→ 1.5（预设）各自落地时，把该类的 canonical 字段 + sidecar passthrough 固化进本 spec。
- 现在：**立原则 + 本策略文档**；字段随导入 Task 逐类补全。全字段 spec = Phase 1 完成时的产物。

## 一句话
我们的资产规格 = **V3 超集 + passthrough sidecar 的 engine canonical 数据模型**，开源版本化。剔无用参数只在装配/活动层，存储永不丢；导入是归一化非翻译，保 round-trip。既满足"内部只用有用的 + 干净提示词"，又不锁死用户资产、不丢第三方数据。
