# Guest Runtime 沙箱与热插拔扩展计划（思路备份）

- **日期**: 2026-08-15
- **状态**: Draft / 思路备份（未立项，未过审计）
- **来源**: deepseek-harness（dsh）深度研究 + 四轮架构讨论收敛
- **前置阅读**: `docs/plans/2026-08-04-DIRECTION-BASELINE.md` §3、`docs/plans/2026-08-07-widget-trusted-plugin-design.md`、`docs/CONVERSATION-CONTRACT.md`（WASM/dylib 预留条款）

## 1. 背景与动机

dsh（"一切皆插件"，Cordis 同进程微内核）证明了插件化 agent harness 的扩展性上限；AIRP 走的是"Rust 核心焊死 + 分级信任隔离"的相反路线。目标：**在不放弃核心焊死与分级信任的前提下，让 Dev Mode 的扩展性/兼容性达到 dsh 水平**——非 Rust 插件（JS/Python 等）不重写即可挂接引擎阶段。

**明确否决的路径**: JS→Rust"转译"（技术上等价于内嵌解释器，不存在诚实实现）。

## 2. 核心结论：GuestRuntime 统一抽象

```
GuestRuntime（engine 内嵌沙箱抽象，trait）
├─ QuickJS 实例   → JS 专用 guest，动态性最高（dsh 级热改/闭包注册）
└─ wasmtime 实例  → polyglot guest，语言最广（WASM Component Model + WIT）
```

- engine 只定义**一次**接缝合同 + **一次**生命周期/效果绑定，guest runtime 是可插拔 backend。
- 不是两个 feature，是一个 feature 的两个 backend。

### 2.1 三平面沙箱梯度（与既有设计同构）

| 平面 | 沙箱 | 语言 | 信任档 |
|---|---|---|---|
| 前端 | Widget iframe（opaque origin） | JS/HTML | 零信任 |
| 逻辑 | 嵌入 guest 沙箱（本计划） | wasm 全家 + JS | 中/高（binding 面 + capability 授予） |
| 系统 | Trusted Plugin 子进程 | 任意 | 显式信任（全 OS 权限） |

沙箱梯度贯穿三平面，AIRP 对扩展的统一答案："能力由宿主授予，越权物理不可能"。WASI p2 capability-based 模型与 `widget-grants.json` 一一对应。

### 2.2 语言支持矩阵（诚实梯度）

| 梯度 | 语言 | 方式 | 代价 |
|---|---|---|---|
| 真编译 | Rust / Go / C / C++ / AssemblyScript / TS 静态子集 | → wasm-component + WIT bindgen | 小模块、快启动、near-native |
| 解释器打包 | Python / JS / Lua / PHP | 解释器整体进模块（Pyodide / jco） | 5–15MB+，启动稍慢，可用 |
| 实验期 | Java (GraalVM) / .NET (NativeAOT) | 路径不稳 | 不对外承诺 |

## 3. 宪法一致性检查（DIRECTION-BASELINE §3 合宪四条件）

1. 激活权威在引擎 ✓ guest runtime 挂载/卸载由 engine 决定
2. 常驻可见性 ✓ Dev Mode 语义照搬
3. 接缝声明权归引擎 ✓ binding 面/WIT 合同由 engine 定义；JS 无反射逃逸（opaque binding），wasm 无逃逸（线性内存隔离）
4. 迁移宪法 ✓ 结构级改动走 migration + 备份 + 回滚，对 guest 插件同样生效

## 3.5 宪法分层适用与功能下放（graduation）机制（2026-08-15 补）

治理立场：Dev Mode 是普通模式的**功能孵化区**（与 webui 作为桌面端契约孵化器的惯例同构），允许做普通模式禁止的事，并在验证后逐步下放。但表述不是"可违背宪法"，而是**宪法分层适用**：

- **信任语义类条款**（四条件本体、迁移宪法）对所有模式生效，含 Dev Mode——它们是 Dev Mode 自身可信的前提，豁免即自我瓦解（激活权威不在引擎⇒插件可自我提权；无常驻提示⇒用户不知在提权态）。
- **权限范围类条款**（hook 挂接范围、结构级改动、compat shim、Admin 权限）按模式分级，Dev Mode 豁免须进**例外清单**：清单外无豁免。

下放判据（graduation criteria）:

1. Dev Mode 稳定运行窗口（≥ 一个 minor 周期）
2. 零逃逸 / 零事故审计
3. 权限面最小化重构完成
4. **该功能权限需求必须可翻译为普通模式 grant 词汇表的一项**——翻译不出（需无法表达的全权）⇒ 永不下放
5. 独立审计通过

单向棘轮防护：下放门槛 ≥ AGENTS.md 代际重构的"市场验证"逻辑——Dev Mode 内自测跑通不算数；普通模式用户形成依赖后回收 = 破坏性变更，须按 migration 宪法处理。

本计划功能的下放前景分类:

| 功能 | 宪法敏感度 | 下放前景 |
|---|---|---|
| wasm 沙箱 + capability 授予 | 零（强化宪法的功能） | 天然可下放，可中档信任直接出生 |
| 热插拔 + digest 锁分发 | 低 | 判据易满足 |
| compat shim | 中（opt-in + 常驻提示本即 Dev Mode 语义） | 高门槛：承诺版本集 + 审计 |
| 细粒度 hook（越过粗粒度阶段边界） | 最高 | **建议永不豁免，或仅代际级决策**——侵蚀 engine 演进自由度（见 widget-trusted-plugin-design §4 判别标准） |

## 4. 决策点

- **D-1 runtime 次序**: PoC 只做 wasmtime 单 backend（jco 覆盖 JS 动态性达标场景）；QuickJS 视"运行时热改"真实需求再上。理由：wasmtime async 支持成熟，QuickJS 无事件循环是最大工程坑。
- **D-2 binding 面分级**: guest 默认只拿 hook 合同面；IO/fs/network 经 capability grant 授予——复用 `widget-grants.json` 模型新增 extension 域。
- **D-3 前提件**: hook 合同必须先语言中立（WIT/JSON Schema 定义 4 个粗粒度阶段：prepare / finalize / generation_step / memory_compress 的 payload 类型），五通道（widget/quickjs/wasm/subprocess/rust-hook）共享。合同未冻结不嵌沙箱。
- **D-4 分发**: `.wasm` + digest 锁（复用 Widget digest-pinned 机制），承接 `CONVERSATION-CONTRACT.md` 预留的"签名/provenance、取消与资源回收"设计义务。
- **D-5 不做 Cordis API 克隆**（原生路径）：dsh rc 阶段 breaking churn + 独立实现铁律。但见 §6 compat shim 的 opt-in 例外。

## 5. 后续研究项 A：热插拔（2026-08-15 讨论结论）

四要素：

1. **挂载/卸载不重启 daemon**: wasmtime instance 动态 instantiate/drop；QuickJS runtime dispose。沙箱 guest 无全局渗透，drop 即真正释放——比同进程插件干净（dsh 靠效果 unwind 约定，AIRP 靠内存隔离事实）。
2. **in-flight 中断语义**: wasmtime epoch-based interruption（宿主线程定时 bump epoch，guest 在安全点硬停，不破坏引擎内存）为**首选卸载策略**；drain（等待 in-flight 完成）为温和选项。QuickJS 通道 host 驱动 job 循环天然安全点。
3. **注册表世代化**: `HookRegistry { generation: u64, slots: Vec<Option<HookInstance>> }`；挂载 = 写 slot + generation++；in-flight 调用持有 generation 快照，旧代 hook 完成后自然退役。**与 per-session in-flight mutex 设计（PR #374/#431 审计线）协调**: hook 卸载 = 资源释放，必须与 session in-flight 边界对齐，禁止在 in-flight 中途换 hook 语义（或明确声明 epoch-kill 语义）。
4. **状态迁移**: guest 线性内存/JS 堆默认即弃（冷启重挂）；持久状态走插件命名空间侧表（DIRECTION-BASELINE §3.3 既有设计），重挂载时回读。

## 6. 后续研究项 B：注入 + hook 的兼容层（2026-08-15 讨论结论）

分两层，兼容是 opt-in 而非主干：

- **主干**: AIRP 原生 hook 合同（§4 D-3），stable，独立演进。
- **compat shim（opt-in）**: 第三方插件 API 面的**稳定子集**克隆 + 映射到原生合同。复用 Dev Mode 语义管理风险：显式开启 + 常驻 compat 提示 + 只承诺"测试过的插件版本集"，不跟随上游 churn。

映射表（草案）:

| 第三方形状 | 映射到 AIRP |
|---|---|
| dsh `export function apply(ctx)` + `ctx.on('tools/pre-execute', …)` waterfall | ctx.on → 阶段 hook 注册；waterfall decision → 阶段返回值（allow/deny/rewrite） |
| dsh `agent.inject()` | AIRP 上下文注入通道（memory inject，须遵守 "model-visible means logged"） |
| ST 后端插件 init/exit + HTTP 路由 | init/exit → guest 生命周期；路由 → Trusted Plugin 反代域 |
| ST 前端 jQuery 插件 | Widget 域（iframe JS），原生即兼容 |
| ST core hooks（消息拦截/前后 gen） | generation_step / prepare / finalize 阶段 |

与桌面端 TWO-HAND 策略（D6 待决）同构：默认原生生成路径，第三方兼容 opt-in 自担风险。

## 7. PoC 清单（最小验证）

1. WIT 定义单阶段（`memory_compress`）签名 + payload
2. Rust guest（真编译）+ jco JS guest（解释器打包）各挂一个 hook
3. dispose/卸载后注册失效验证（效果可逆性）
4. epoch interruption 杀 in-flight guest 验证（热插拔硬中断）
5. async 阻抗：wasmtime async support × engine tokio runtime
6. 兼容 shim 冒烟：一个 ST 后端插件形状（init/exit + 一个消息拦截 hook）经映射跑通

## 8. 开放问题

- [ ] hook 合同 payload 版本化策略（additive-only？封闭 error 集？对齐 `widget-intents.json` 惯例）
- [ ] 多 hook 同阶段冲突语义：互斥替换 vs 顺序链（§5 世代化只解决卸载，未解决共存）
- [ ] guest 计量与限额（fuel/epoch 上限、内存上限、wall-clock）
- [ ] compat shim 的测试矩阵与"承诺版本集"治理流程
- [ ] QuickJS backend 是否立项的判据（什么算"真实热改需求"）

## 9. 参考

- dsh 一手文档: `docs/architecture.md`、`docs/capability-seams.md`、`docs/subsystems/session.md`、`docs/cookbook/extension-cookbook.md`（deepseek-harness 0.1.0-rc.5, 2026-08-13, MIT）
- 内部: `docs/plans/2026-08-04-DIRECTION-BASELINE.md`、`docs/plans/2026-08-07-widget-trusted-plugin-design.md`、`docs/CONVERSATION-CONTRACT.md`
- 按 AGENTS.md 第三方吸收规则：仅吸收理念（事件三分域、效果可逆性、feature→mechanism map 等），实现完全独立；落地前记 `docs/ACKNOWLEDGEMENTS.md`
