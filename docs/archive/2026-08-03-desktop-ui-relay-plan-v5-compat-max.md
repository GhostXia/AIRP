# 桌面端 UI 画布接力开发计划书 v5（兼容性与扩展性最大化）

> 版本：v5（compat/ext-max 重排）
> 日期：2026-08-03
> 取代：v4 融合稿（`2026-08-03-desktop-ui-relay-plan-v4-fused.md`）——v4 的信任脊柱全部保留，但**组织前提改变**
> 状态：**规划草案，非开工授权**。任何"已写入计划"项未经 R0/R1 证据不得声称落地。

---

## 0. 前提转变（本稿与 v3.7 / 审计 v4 的根本差异）

| | v3.7（作者） | 审计 v4（否决方） | **v5（本稿）** |
|---|---|---|---|
| 第一前提 | 尽快取得开工授权 | 安全边界必须先被证伪 | **兼容性与扩展性最大化** |
| 安全的角色 | 需被满足的约束 | 唯一优化目标 | **扩展生态的前置条件**（不是对手） |
| 兼容工程 | §5 三层 + 矩阵（有，但未可执行） | 基本删除 | **一等交付，进 CI harness** |
| 扩展面 | 较宽但边界含糊 | 收窄至最小 | **面积最大化，权限最小化**（两者正交） |

**核心论断**：安全与扩展性不是同一维度的取舍。真正压缩扩展性的是**含糊的边界**——因为边界含糊，唯一安全做法就是收窄权限。把边界做到可证伪、可枚举、可撤销之后，**扩展面反而可以开到最大**：每一个扩展点都有明确的能力、明确的失败语义、明确的审计轨迹。

因此 v5 的目标函数是：
> **在可证伪的信任边界内，最大化「扩展点数量 × 跨版本/跨运行时可用性 × 生态可迁移性」。**

---

## 1. 不可让渡原则（安全侧，沿用 v4，不再讨论）

1. 信任边界由引擎证明与强制，UI consent 只是**渲染**，不是授权权威。
2. 默认拒绝、fail-closed：未知发送方/未握手/越向/超限/重放/缺沙箱/远程 URL 一律拒绝并审计。
3. 隔离靠证伪，不靠宣称；未证实的边界不得写"已落地"。
4. 用户资产不可静默损坏；还原点覆盖运行时写入（WAL）。
5. 市场/SDK 是分发层，**永不回压**运行时隔离合同。

**源码事实（已核验，构成对 v3.7 §4.8 "已落地"的否决依据）**：
- `WidgetEntry.sandbox?: boolean` 可选、默认 false（`ui/src/protocol/types.ts:153`）；
- `sandboxed = entry.kind==="esm" && entry.sandbox===true`（`ui/src/components/WidgetHost.vue:34-35`）；
- esm widget 在 registry 注册为 `kind:"module"`（`ui/src/registry/registry.ts:67-68`），故未沙箱化第三方 esm 走 `mountModule()` **进程内渲染**（`WidgetHost.vue:194`），无 iframe 隔离。
→ v5 修正：第三方 esm **默认 `sandbox:true`**，缺失或显式 false **拒绝加载**，不回退进程内路径。

---

## 2. 扩展性最大化：八根支柱

### 2.1 单一合同面（One Contract Surface）
- `hostApi` + `contextSchema` + `hookApi` + `intentSchema` 是**唯一**扩展入口，无侧门。
- 同一合同面在 **WebView2 桌面 / 浏览器 webui / 未来任意 WebView 宿主** 行为一致；宿主差异由 host 适配层吸收，不外泄给扩展。
- 收益：一次编写、全宿主分发 → 分发半径最大化，这是兼容性的最大杠杆。

### 2.2 能力协商，而非版本判断（Capability Negotiation）
- 扩展 manifest 声明 `requires[]` / `optional[]` 能力集；引擎握手时回 `granted[]` + `denied[]{reason}`。
- 扩展**按 granted 自适应降级**，禁止基于 engine 版本号做 if 分支。
- 收益：引擎增删能力不再是 breaking change；老扩展在新引擎、新扩展在老引擎都有定义好的行为。

### 2.3 扩展点目录（Extension Point Catalog，机器可读）
- 所有扩展点集中注册：`slot` / `hook` / `intent` / `command` / `theme token` / `data namespace` / `panel` / `menu` / `shortcut`。
- 每点带元数据：`id, since, stability(experimental|stable|frozen), deprecated_since, replaced_by, required_capability, payload_schema`。
- 引擎按 catalog 授权；**未知扩展点 fail-closed**。
- 收益：扩展面「可枚举、可禁用、可审计、可演进」；新增扩展点无需改引擎核心（扩展性 SLO 见 §5）。

### 2.4 信任梯度增设 T2.5（已验证第三方）
| 平面 | 内容 | 隔离 | 能力天花板 |
|---|---|---|---|
| T0 首方 | 随包构建 | 同进程（唯一允许） | 全权 |
| T1 声明式 | Blueprint/L1 spec/Theme | 只产数据，不执行 JS | 数据面全开 |
| **T2 不可信** | 本地 digest-pinned 包 | 强制隔离 runtime | 最小 capability，无 DOM/密钥/FS/直连网络 |
| **T2.5 已验证第三方（新增）** | digest-pinned + 签名/评审背书 + 显式用户 consent | **同 T2 的隔离，不放松** | 引擎签发**窄能力**：受限 FS scope、经 proxy 的受限网络、命名 domain command 写入 |
| T3 后端/外部 | 受管 sidecar / 用户自管服务 | 独立生命周期 + 每实例凭证 | 逐请求 capability |

- **关键**：T2.5 抬高的是**能力天花板**，不是**隔离级别**。隔离恒定，权限可协商。这解决了 v4 "要么首方全权、要么不可信最小权"的二值困境。

### 2.5 Hook 能力注册表 + 扩展面恢复
- v4 把 Hook 收窄到 observe/transform/command 三档是正确的**语义**收窄，但不应等于**数量**收窄。
- v5：Hook 点数量按 catalog 开放（消息前后、渲染前、上下文装配、检索、工具调用、持久化前后等），每点必须落在三档语义之一，并绑定 `required_capability`。
- 插件 manifest 声明所需 hook 点 → 引擎按点签发 → 未声明即无权 → 未知点拒绝。
- 每个 Hook 冻结确定性 effect 契约：顺序/优先级、事务边界、超时与取消、异常与重试、重入与幂等、payload 上限、replay 确定性、审计 trace。

### 2.6 前向兼容的数据与协议规则（Forward-Compat Rules）
- **未知字段必须保留并原样回传**，禁止静默丢弃（这是双向兼容的第一条铁律）。
- schema 演进 **additive-only**；breaking 变更只能通过新版本号 + 迁移器 + 双读单写窗口。
- 所有跨边界 payload 带 `schema_version`；引擎与扩展各自声明可读版本区间。
- 收益：新旧混用不产生数据损坏，也不阻塞演进。

### 2.7 适配器边界（Adapter Boundary）承载第三方生态
- 第三方生态（ST/酒馆等）兼容**不做进主域**，而是定义**合同化适配器**：
  - 明确 API 映射子集表（支持 / 降级 / 不支持，逐条列出）；
  - 适配器本身作为 **T2.5 包**运行，享受同一隔离与授权机制；
  - 兼容承诺分级：`best-effort` / `contractual`，写进文档而非口头。
- 收益：生态兼容故事最大化，且**不需要逐插件 AI 适配**，也不牺牲安全边界（回应 TWO-HAND 的 D6：opt-in 走 T2.5 而非放开主域）。

### 2.8 稳定性分级与弃用政策（治理面）
- `experimental` → `stable` → `frozen`；弃用需 ≥2 个 minor 窗口 + 自动迁移器 + 编译期/加载期告警。
- frozen 合同（hostApi/contextSchema/hookApi）在 R5 冻结版本后进入兼容性测试基线。
- 收益：生态不被频繁破坏——兼容性最大化的**制度**保障，而非仅技术保障。

---

## 3. 兼容工程三层 + 可执行矩阵（恢复 v3.7 §5 并升级）

### 3.1 协议层
- 版本协商（`protocol_version` 交集）、未知字段保留、能力集协商、错误码稳定表。

### 3.2 Widget / 扩展层
- 合同版本声明 + 能力降级路径；每个 optional 能力必须有 `denied` 分支的定义行为。

### 3.3 引擎合同层（降级矩阵）
| 场景 | 引擎行为 | UI 行为 |
|---|---|---|
| 旧 engine 缺 capability | 握手 `denied{reason:"unsupported"}` | 功能**诚实禁用** + 原因 + 文档链接，不崩不静默 |
| 新 engine 增能力 | 老扩展不声明即不获得 | 无感知，行为不变 |
| 扩展要求 frozen 合同的更高 minor | 拒绝加载 + 明确提示 | 错误占位卡，提示升级 |
| 扩展点已 deprecated | 授权 + 告警 | 显示弃用提示与替代点 |
| 扩展点未知 | **拒绝** | 错误占位卡 |

### 3.4 兼容性验收矩阵（进 CI，非文档声明）
| 维度 | 断言 | 门禁 |
|---|---|---|
| 跨运行时 | 同一 widget 在 WebView2 与浏览器 webui 行为一致（同 fixtures 同快照） | R2 起 |
| 跨引擎版本 | 第三方包**零改动**跨 N-2 个 engine minor 可运行或诚实降级 | R5 起 |
| 协议前向兼容 | 注入未知字段，往返不丢失 | R1 起 |
| 降级正确性 | 逐条覆盖 §3.3 五种场景 | R3 起 |
| 安全负向 | 密钥拦截 / 崩溃隔离 / 越权 / 重放 / 未知 hook 全拒 | R0/R3 |
| 生态适配器 | ST 映射子集表逐条断言（支持/降级/不支持三态） | R5（gated） |

**compat harness** 与 R0 隔离 harness 同级：兼容性必须像隔离一样**被证明**，不得只写在计划里。

---

## 4. 安全脊柱（沿用 v4，压缩表述）

- **不可变包**：v1 只运行本地安装包；digest 寻址；内容变化即新身份、旧授权失效；删除运行时 `import(remote)`、卡内 JS、远程 ESM。
- **Engine grant**：`package_digest + instance_id + profile/session + capability + resource scope + expiry/revocation_epoch`，逐调用强制；实际权限 = manifest ∩ instance scope ∩ 用户同意 ∩ engine policy；widget 永不接触 bearer/provider key/data-root；签发写入**结构化授权决策日志**。
- **Runtime handshake**：绑定 `protocol version + runtime/instance id + package digest + nonce + seq + direction + schema + max frame`；opaque origin 下不把 `"null"` 当认证。
- **资源隔离诚实分层**：iframe 只承诺 DOM/storage；要承诺 CPU/内存/崩溃隔离，T2/T2.5 必须独立 WebView/renderer/helper process + watchdog，以实测定 runtime；不得把 iframe 冒充 OS sandbox。
- **沙箱强制**：第三方 esm 缺 sandbox 即拒绝加载（§1 源码修正点）。
- **跨边界审计事件**：widget↔host / sidecar↔engine / hook 每次跨边界 emit 结构化事件，受 WAL 保护。
- **威胁模型首类交付**：R0 先出 STRIDE 文档；每条安全声明映射到一条防御断言 + 一条证伪测试。
- **插件数据三合同**：private / governed（命名 domain command + expected revision）/ external（只读扫描 + 单向导入）；还原点 = crash-consistent WAL，**覆盖运行时写入**，restore drill 进 CI。
- **sidecar**：每实例临时凭证（mTLS/签名 token）+ 逐请求 capability；loopback 非身份。
- **渲染**：sanitizer 白名单与 CSP `img-src` **同一配置源**，两端共享 XSS fixtures，零违规为硬门禁。

---

## 5. 扩展性 SLO（把扩展性变成可测指标）

| 指标 | 目标 | 验证 |
|---|---|---|
| 新增一个 slot / hook 点 | **不需修改引擎核心逻辑**，只增 catalog 条目 + schema | R3 演练 |
| 第三方包跨引擎版本 | 零改动跨 **N-2 minor** 运行或诚实降级 | compat harness |
| 跨宿主一致性 | 同 widget 在两宿主快照一致率 100% | compat harness |
| 扩展点可禁用率 | catalog 中 100% 可被用户/策略单点禁用 | R3 |
| 生态适配覆盖 | ST 映射子集表覆盖率与三态标注完整 | R5（gated） |
| 能力最小化 | 任一 T2/T2.5 包实际授权 ⊆ 声明 ∩ 用户同意 | 逐调用断言 |

---

## 6. R0–R6 重排（proof-first，compat harness 提前）

1. **R0 — 威胁模型 + 隔离证伪（硬门禁）**：STRIDE 文档；deterministic `hello-panel` + 恶意 fixtures；真实打包 WebView2 跑 `hello→mount→state→intent/error→destroy`；验证 DOM/storage/network/伪造 sibling/nonce/重放/超限/死循环/崩溃；比较 iframe 与独立 runtime。任一核心边界失败即停止。（R0 = 最小隔离可行性；R5 = 生产包/多入口完整回归，二者解耦。）
2. **R1 — Trust kernel + 合同面骨架**：不可变 package store、版本化 manifest/schema、handshake、grant issue/use/revoke/expiry、sidecar 身份；**同时**建立扩展点 catalog 与能力协商，前向兼容规则进测试。仍不开放第三方代码。
3. **R2 — 真实垂直链 + T1 + 跨宿主一致性**：`Tauri→engine→SSE/patch→render` 真实链、一个 declarative widget、一个首方 slot/typed context、一个 engine-issued intent；**compat harness 首次上线**（跨运行时快照一致）。
4. **R3 — T2/T2.5 本地代码扩展 + 降级矩阵**：hash-pinned 本地包、强制隔离、CSP/网络代理、watchdog/错误占位、安装/启停/升级/卸载；T2.5 窄能力签发；§3.3 五种降级场景全覆盖；扩展点逐点可禁用演练。R0 若未能证资源隔离，本阶段明确不支持任意第三方 ESM。
5. **R4 — 数据恢复 + Hook 面开放**：migration registry、backup/export/restore、WAL 与恢复演练；按 catalog 开放 Hook 点（每点绑定 capability + 确定性 effect 契约）；故障注入（跨 namespace、崩溃中断、磁盘满、重放、升级失败）。
6. **R5 — 合同冻结 + 生态适配器 + 兼容基线**：冻结 hostApi/contextSchema/hookApi 版本与迁移；两端 sanitizer/CSP 同源 + 共享 XSS fixtures；SDK + 1–2 示例；**N-2 跨版本兼容测试进基线**；ST 适配器映射子集表（gated 实现，合同先立）。
7. **R6 — Release candidate**：固定硬件/runtime/fixtures，首屏、p95、内存、CPU、100k/50-widget 阈值；NSIS/EXE + real engine、离线、升级失败回滚、backup/restore、密钥 canary、**完整负向矩阵 + 兼容矩阵全绿**。远程包、AI L3、市场另立 RFC。

---

## 7. 保留的已决骨干（不重新争论）

经 `2026-07-28-desktop-ui-relay-plan-audit.md` 已决，原样保留：路线 B+ 旗舰协议、性能架构（id-keyed 聊天 / 虚拟列表 / 窗口化状态 / RFC6902 store / patch 原子性）、Widget/Blueprint 体系、接力模型（交接包 / 棒次 / 溯源表 / 审计门禁）、视觉对拍（令牌 0 偏差、取色 ±1）、四条数据安全语义、AGENTS.md 治理条款、自定义四层体系（首批 6 slot）、酒馆两手策略（手二收编为 T2.5 适配器）、第三方导入通道（市场地基）、AI L1/L2 声明式优先、100k/50-widget 性能合同。

---

## 8. 相对前两版的净变化

**相对 v3.7**：删除"§4.8 已落地"误标 → 全部改"已写入计划/待验收"；附录 D 待拍板项由信任平面直接裁决（D4/D6/D7 按默认拒绝 / opt-in 走 T2.5）；删除 `D:\.WorkBuddy\...` 绝对路径引用，改仓库相对 + SHA-256 + 保留责任；修正 sandbox 默认与 sanitizer/CSP 冲突。

**相对审计 v4**：恢复并升级兼容工程三层 + 可执行兼容矩阵；新增 T2.5 梯度打破能力二值；Hook 由"数量收窄"改为"语义收窄 + 目录化开放"；新增能力协商、扩展点 catalog、前向兼容铁律、扩展性 SLO、稳定性/弃用政策、适配器边界。安全脊柱**一条未松**。

---

## 9. 合并前置（硬门禁，不变）

R0 隔离证伪通过 + 13 条历史 review threads 全部关闭 + compat harness 骨架就位（R1 项可后置但需在计划中定义）+ 重新取得审计通过。在此之前本稿为**规划输入**，不构成开工授权。
