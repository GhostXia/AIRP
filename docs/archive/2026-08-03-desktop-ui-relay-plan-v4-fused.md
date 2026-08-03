# 桌面端 UI 画布接力开发计划书 v4（融合方案）

> 版本：v4（融合稿）
> 日期：2026-08-03
> 来源：融合 (A) v3.7 计划书已采纳项 + (B) 2026-08-03 独立审计的 proof-first v4 架构 + (C) 本稿独立审计新增项
> 审计性质：本稿遵守 AGENTS.md 审计守则——不附和、可质疑、以源码证据为准。v3.4 骨干中经 `2026-07-28-desktop-ui-relay-plan-audit.md` 已决的结论**原样保留**，仅重建信任架构脊柱。
> 状态：**规划草案，非开工授权**。凡"已写入计划/待验收"者，未经 R0 硬门禁与运行时证据不得落地为代码。

---

## 0. 三方来源与融合原则

| 来源 | 贡献 | 在本稿中的处理 |
|---|---|---|
| **A — v3.7 计划书** | 路线 B+ 旗舰协议、Widget/Blueprint/sandbox 体系、接力模型、视觉对拍、自定义四层、酒馆两手、AI L1/L2 声明式、性能合同、后端扩展升为一等交付 | 已决骨干**保留**；§4.8 安全框架以本稿信任架构**替换** |
| **B — 审计 v4** | 信任平面 T0–T3、引擎自有信任内核、不可变包、Engine grant、runtime handshake、资源隔离诚实分层、proof-first R0 门禁 | **采纳**为安全脊柱主结构 |
| **C — 本稿独立新增** | 威胁模型首类交付、sidecar 每实例凭证、WAL 式还原点、Hook 确定性 effect 契约、跨边界结构化审计事件、consent 为"渲染非权威"形式化、SDK/市场不回压信任边界 | **新增**，见 §3.7 / §4 / §5 / §7 / §9 |

**融合原则**：保留 v3.4→v3.7 中已闭合的工程决策，不重新争论；推倒重建的 only 是"安全边界如何被证明与强制执行"，并以可运行证据为合并前置。

---

## 1. 设计原则（不可让渡）

1. **信任边界由引擎证明与强制，不由 UI 暗示。** consent 是用户意愿的*渲染*，不是授权权威；实际权限取 manifest 声明 ∩ instance scope ∩ 用户同意 ∩ engine policy 的交集，由引擎逐调用签发与校验。
2. **默认拒绝、fail-closed。** 未知发送方、未握手、越向、超限、重放、缺沙箱、远程 URL、运行时 `import(remote)` 一律拒绝并审计。
3. **隔离是证伪出来的，不是宣称的。** 任何信任声明须有 R0 隔离证伪实验的通过证据；未证实的边界不得写"已落地"。
4. **用户资产不可静默损坏。** 迁移为单向、幂等、校验和验证的复制；原目录永不被改；还原点覆盖运行时写入。
5. **市场/SDK 是分发层，永不回压信任边界。** 签名/目录成熟不改变运行时隔离合同。

---

## 2. 信任平面（采纳 B，细化）

| 平面 | 允许内容 | 隔离与强制 | 当前代码差距 |
|---|---|---|---|
| **T0 首方宿主** | 随发布包构建的 Vue/ESM | 唯一允许同进程执行的代码；第三方 manifest **无权**选 `sandbox:false` | 现状：首方走 `kind:"module"` 进程内，符合 |
| **T1 声明式扩展** | Blueprint/L1 spec、受信模板、Theme tokens | 只产生数据，不执行任意 JS；覆盖多数定制 | 符合；保留 |
| **T2 不可信 UI 扩展** | 已安装、内容寻址的本地包 | **必须**进入隔离 runtime，仅经 schema 化消息 broker；无 DOM/密钥/文件系统/直连网络 | **现状缺陷**：`WidgetEntry.sandbox?: boolean` 默认 false，未设时走 `mountModule()` 进程内（WidgetHost.vue:194）——须改为默认强制 `true`、缺失即拒 |
| **T3 后端/外部扩展** | 受管 sidecar 或用户自管外部服务 | 独立生命周期/身份/RPC broker；loopback 非身份，无 OS 隔离时显式为 trusted-user/实验 | 现状：loopback 桥无 sidecar 注册与逐请求 capability（待建） |

由此消除两个根本歧义（采纳 B）：
- 第三方代码**不得**沿用同进程 ESM fallback；`entry.sandbox` 不是第三方自选开关。
- 前端扩展权**不自动**包含后端写权限；UI consent 只是界面，不是授权权威。

---

## 3. 引擎自有信任内核（采纳 B + C 细化）

### 3.1 不可变包
- v1 只运行**本地安装包**。目录/zip 规范化后计算 package digest，安装到 content-addressed store。
- 升级或内容变化即新身份，旧授权自动失效。
- **删除**：运行时 `import(remoteUrl)`、卡内 JS、远程 ESM。
- 哈希/签名只证完整性或来源，**不等于**代码安全。

### 3.2 Engine grant（采纳 B + C：决策日志）
- 引擎按 `package_digest + instance_id + profile/session + capability + resource scope + expiry/revocation_epoch` 签发**最小授权**，逐调用强制。
- 实际权限 = manifest ∩ instance scope ∩ 用户同意 ∩ engine policy 交集。
- widget **永远**拿不到 bearer/provider key/data-root 路径。
- **C 新增**：capability 签发是引擎纯函数，输出写入**结构化授权决策日志**（谁/何时/依据哪条 policy/签发哪类 capability），供审计回放。

### 3.3 Runtime handshake（采纳 B）
- 消息协议至少绑定 `protocol version + runtime/instance id + package digest + nonce + seq + direction + schema + max frame`。
- 未知/越向/超限/重放 → fail-closed + 审计。
- opaque origin 下不把字符串 `"null"` 当认证；用可工作传输 + `event.source` + 每实例 nonce/session 建立身份。

### 3.4 资源隔离诚实分层（采纳 B）
- iframe 只承诺 DOM/storage 边界。
- 若要承诺死循环/CPU/内存/崩溃不拖垮主域，T2 **必须**用独立 WebView/renderer/helper process + watchdog，并以实测定最终 runtime。
- **不得**把 iframe 冒充 OS sandbox。

### 3.5 沙箱强制（C 修正源码缺陷）
- 第三方 `kind:"esm"` widget：**`entry.sandbox` 默认视为 `true`**；缺失或显式 `false` → 加载前**拒绝**（不回退进程内 module 路径）。
- 对应代码修正点：`WidgetHost.vue` 的 `sandboxed` 判定维持；`reg?.kind==="module"` 分支**仅限首方**；第三方 esm 未沙箱化 → 错误占位卡而非挂载。

### 3.6 跨边界审计事件（C 新增）
- widget↔host、sidecar↔engine、hook 调用，每次跨边界动作 emit **结构化审计事件**（src/dst/instance/digest/capability/结果/耗时）。
- 这是"fail-closed + 审计"从口号变机制的关键；审计事件本身受 WAL 保护，不可被 widget 篡改。

### 3.7 威胁模型首类交付（C 新增）
- R0 须先产出 **STRIDE 式威胁模型文档**，列出每平面的攻击者能力假设与对应的防御断言。
- 架构的每条安全声明必须能映射到威胁模型中的一条防御断言 + 一条 R0 证伪测试。

---

## 4. 插件数据与还原点（采纳 B + C：WAL）

三个清晰合同（采纳 B）：

- **private**：引擎托管插件命名空间，schema/quota/revision/导出卸载保留策略明确；是否进 backup **显式**声明。
- **governed**：插件只提交命名 domain command，引擎校验 expected revision + capability + 不变量后事务提交；**不允许**直接写核心表或任意 RP 真相。
- **external**：用户自管 sidecar 数据，只记 provenance/hash + 排除范围；默认不承诺 AIRP backup，进核心只走只读扫描 + 显式校验 + 单向导入。

**还原点 = WAL（C 修正）**：
- 安装/升级快照**不**冒充通用恢复。
- 受管写入须：提交前 schema/namespace 校验 + **crash-consistent WAL/journal** + 不可变 revision + backup manifest + **restore drill（CI 门禁）** + 卸载 quarantine/显式 purge。
- 运行时 P2 写入**同样**受 WAL/还原点覆盖（修正 v3.7 "仅安装前还原点"的局限）。

---

## 5. Hook v1 收窄与确定性语义（采纳 B + C：effect 契约）

- **observe**：提交后只读通知，at-least-once，带 event/idempotency key。
- **transform**：仅少数 pre-commit 事件，固定排序、deadline、无网络/随机/时间依赖，只返回 schema-validated patch。
- **command**：所有副作用经 engine grant + expected revision。
- **onRender** 留在 UI 扩展协议，不混进 engine Hook。
- **C 新增（确定性 effect 契约）**：每个 Hook 冻结——顺序/优先级、事务边界、超时与取消、异常与重试、重入与幂等、payload 上限、审计 trace；慢 Hook/重复投递/replay 的**确定性行为**写入契约，R5 合同冻结前纳入兼容性测试，与 engine 侧强制调用约束一致。

---

## 6. 后端/外部扩展（采纳 B + C：sidecar 凭证）

- 每个 sidecar 启动由引擎签发**每实例临时凭证**（mTLS 或签名 token），绑定可撤销身份。
- 请求绑定到该注册身份 + 显式 capability，**逐请求校验**（不只依赖 loopback 来源）。
- 桥接接口：凭证拦截与脱敏审计、URL/redirect 限制、超时与响应大小上限。
- 无 OS 隔离时，sidecar 显式为 trusted-user/实验能力，不得冒充受管域。

---

## 7. 渲染 / 富文本 / XSS（采纳 A 优点 + C 一致性强制）

- 默认管线 `img[src]` sanitizer 白名单与 `img-src` CSP **同一份配置源**（修正 v3.7 中"显式允许域"与"仅 self/data/blob"的冲突）。
- 两端（桌面 WebView2 + webui）使用**同一来源策略 + 共享 XSS fixtures**，对相同 URL 得出相同 allow/deny。
- 富文本清理、兼容域渲染、DOM 隔离保留为 R5 验收项；XSS fixtures 零违规为硬门禁。

---

## 8. ST 兼容与 AI L3 后置（采纳 B，保留 A 酒馆两手）

- 当前路线只交付 copy-only 的 ST 数据迁移、报告、原目录 hash 不变、失败回滚。
- 可运行 ST shim 另立隔离实验：独立 runtime、默认关、无密钥/bearer、明确 API 子集与配额；不注入主域 DOM。
- AI 只交付 L1 spec / L2 模板。L3 生成源码可查看/导出，**不进** release 执行路径；未来若执行，按普通 T2/T3 不可信包处理，另开安全与治理 PR。
- 酒馆衔接保留两手（手一原生生成默认安全 + 手二兼容域 spike），手二仍受 D6 裁决与全局开关默认关约束。

---

## 9. 证明优先的 R0–R6 重排（采纳 B，补 harness）

每一棒一个有界、可回滚 PR；若替换范围使 `main` 无法持续可发布，则建立隔离分支，原 WebUI/主线继续维护。

1. **R0 — 威胁模型 + 隔离证伪实验（硬门禁）**：先产出 §3.7 威胁模型文档；提交本地 deterministic `hello-panel` + 恶意 fixtures；在真实打包 WebView2 中跑 `hello→mount→state→intent/error→destroy`。验证 DOM/storage/network、伪造 sibling、nonce、重放、超限消息、死循环/崩溃；比较 iframe 与独立 runtime。**任一核心边界失败即停止，不进 R1。**（修正 v3.7 把同 spike 既放 R0 又放 R5 的混淆——R0=最小隔离可行性证据，R5=生产包/多入口完整回归。）
2. **R1 — Trust kernel**：不可变 package store、版本化 manifest/schema、runtime handshake、Engine grant issue/use/revoke/expiry、sidecar 启动身份。仍不开放第三方代码。
3. **R2 — 一条真实产品垂直链 + T1**：`Tauri→engine→SSE/patch→render` 真实聊天链、一个 declarative widget、一个首方 slot/typed context、一个 engine-issued intent；real engine，MockBus 仅留测试。
4. **R3 — T2 本地代码扩展**：仅开放 hash-pinned local package；强制隔离、CSP/网络代理、协议负向测试、watchdog/错误占位、安装/启停/升级/卸载。R0 若无法证资源隔离，本阶段明确不支持任意第三方 ESM。
5. **R4 — 数据恢复 + 最小 Hook/sidecar**：先 migration registry、backup/export/restore、WAL 与恢复演练，再 `private/governed/external` 垂直切片及极少量 Hook；故障注入（跨 namespace、崩溃中断、磁盘满、重放、升级失败）。
6. **R5 — 富文本、迁移与开发者合同收口**：两端 sanitizer/CSP 同源 + 共享 XSS fixtures；冻结 hostApi/contextSchema/hookApi 版本与迁移；交付 SDK、1–2 示例、compatibility harness。回归 R0 生产包/多入口沙箱。
7. **R6 — Release candidate**：固定硬件/runtime/fixtures，首屏、p95、内存、CPU、100k/50-widget 数值阈值；NSIS/EXE + real engine、离线、升级失败回滚、backup/restore、密钥 canary、完整负向矩阵全绿。ST runtime、远程包、AI L3、市场另立 RFC/隔离产品线。

---

## 10. 可翻转 BLOCK 的硬证据（采纳 B）

- 第三方 `sandbox:false`、缺 sandbox、remote URL 加载前拒绝；仅固定 digest 本地包可运行。
- 真实浏览器/打包 E2E 证桥可通信，篡改/未知/重放/超限消息全拒且有审计。
- UI/sidecar 伪造 grant、跨 namespace、越权 operation 全拒；密钥不进消息/日志/插件域。
- crash-consistent write、backup/restore、升级/卸载 rollback 可重复演练，原迁移目录 hash 不变。
- sanitizer 与 CSP 对相同 URL 同结果，XSS fixtures 零违规。
- 报告、owner 决策、实现状态、runtime evidence **分栏**记录；第三方评审以仓库相对/稳定附件引用并附 SHA-256/来源/保留责任（替换 v3.7 中的 `D:\.WorkBuddy\...` 绝对路径）。

---

## 11. 与 v3.4 已决骨干的衔接（保留项，不重新争论）

以下经 `2026-07-28` 审计已决，本稿原样保留：路线 B+ 旗舰协议、性能架构（id-keyed 聊天/虚拟列表/窗口化状态/RFC6902 store/patch 原子性）、Widget/Blueprint/sandbox 体系、接力模型（交接包/棒次/溯源表/审计门禁）、视觉对拍（令牌 0 偏差、取色 ±1）、四条数据安全语义、AGENTS.md 治理条款、自定义四层体系（首批 6 slot）、酒馆两手策略、第三方导入通道（市场地基）、AI L1/L2 声明式优先、100k/50-widget spike 性能合同。

---

## 12. 版本与待办

- 本稿版本：**v4（融合）**。相对 v3.4：替换信任架构脊柱（§2–§6、§9），新增威胁模型/sidecar 凭证/WAL/审计事件/Hook 确定性契约（§3.5–§3.7、§4、§5、§6）。相对 v3.7：删除"§4.8 已落地"误标，改"已写入计划/待验收"；附录 D「待拍板」项由本稿信任平面模型直接裁决（D4/D6/D7 均按默认拒绝/opt-in 风险区处理）。
- 待办：① 起草 R0 威胁模型文档与隔离证伪 harness fixtures；② 修正 `WidgetEntry.sandbox` 默认强制 + 第三方回退拒绝的代码；③ 建立 Engine grant 决策日志与 WAL 还原点骨架；④ 统一 sanitizer/CSP 配置源。
- **本 PR 合并前置**：R0 硬门禁通过 + 上述 13 条历史 review threads（sandbox 默认、plugin-data 能力、loopback sidecar 注册、sanitizer/CSP 同源、postMessage 会话认证、Hook 冻结语义、还原点覆盖运行时、R0/R5 解耦、绝对路径引用等）全部关闭 + 重新取得审计通过。
