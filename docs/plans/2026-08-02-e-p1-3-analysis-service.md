# E-P1-3 P0: AnalysisService 提取计划

> 本 PR 目前含本计划文档。审计已完成（见下文"审计结论"），实施按修订后方案在同一个 PR 内提交。

## 现状（为什么是 P0 最危险）

`analysis` MD 资产有两处非原子写盘点，均用 `tokio::fs::write`（非 `replace_file`，无 tmp+rename+fsync，半写状态可被并发 reader 看到）：

1. `engine/src/agent/tools/analysis.rs:270` — `ApplyEnhancedAnalysisTool` 写 `characters/{id}/analysis/{filename}`
2. `engine/src/daemon/decompose_handlers.rs:306` — `enhance_or_apply_character_analysis` 的 `action=apply` 分支写同一文件

对比其他已收口资产：
- `world_events.json`：原子写 + `AssetKind::WorldEvents` + `revisions/{n}/` 快照合同
- `plot_arc.json`：原子写（`replace_file`），无 revision 合同但单文件
- `analysis/`：**非原子写 + 无 revision 合同 + 多文件资产**

此外无 `character_lock` 串行化，agent tool 与 daemon HTTP 端点可并发写同一文件。

## 审计结论（2026-08-03）

Owner @GhostXia 在 PR #431 评论提交了面向扩展性的替代设计方案（5 个方案）。项目方逐条回应后，Owner 进一步提交反驳分析。项目方核验后确认反驳成立，修订方案如下：

| 方案 | 审计/反驳立场 | 项目方最终立场 |
|---|---|---|
| 1. CAS 乐观并发（`expected_hash` + `SaveOutcome`） | 结论同意"本 PR 不做"；但项目方原论证"无并发场景"与计划自述的 B1 风险矛盾；`u64` 碰撞论证不成立 | **采纳反驳**：CAS 结论维持"本 PR 不做"，但论证换成"超出 A1 范围、应与 revision 合同一起设计"；**强制文档化 last-write-wins 静默丢失风险 + 开 follow-up issue** |
| 2. struct 参数替代位置参数 | 反驳方主动收回（CAS 否决后 struct 动机消失） | 不采纳 |
| 3. `AssetPolicy` trait / 自由函数 | 反驳方建议抽通用安全纯函数 `reject_unsafe_filename` | **核验后收回补充建议**：`char_analysis_file_path` 已内置完整路径穿越/绝对路径/非 .md 校验（白名单 `[a-z0-9_/.-]+\.md` + `strip_prefix` + `Component::Normal` 检查），`AnalysisService` 调用它即自动获得全部安全校验，无需再抽通用函数。`world_book/` 拒绝是 analysis 专属业务规则，保留在 Service 内 |
| 4. `spawn_blocking` 包装调用边界 | 反驳成立：原代码用 `tokio::fs::write`（已卸载到 blocking 池），改同步 `std::fs` 不包装是真实回归（`search.rs:44` 有 `spawn_blocking` 既定惯例） | **采纳反驳**：本 PR 在两个 async 调用点加 `tokio::task::spawn_blocking` 包装；`AnalysisService` 内部保持同步 |
| 5. 泛型化资产 Service 基础设施 | 双方一致：不在本 PR 做 | 不采纳；API 命名已对齐（`load_file`/`save_file`） |

**审计揭示的项目方论证错误**（已修正）：
1. Point 1：用"无并发场景"搪塞 CAS，而计划"现状"节自述"agent tool 与 daemon HTTP 端点可并发写同一文件"——自相矛盾
2. Point 4：用"与其他 Service 一致"豁免了其他 Service 不存在的、新引入的 `tokio::fs`→`std::fs` 回归

## 设计决策（修订后）

### 决策 A：revision 合同范围

- **选定 A1 + 预留 `AssetKind::Analysis` 枚举变体**：本 PR 只做原子写改造 + Service 收口，**不实现 revision 快照逻辑**。同时在 `revision/manifest.rs` 的 `AssetKind` 枚举加 `Analysis` 变体（`rename_all = "lowercase"` 自动序列化为 `"analysis"`），加序列化往返测试。
- 理由：analysis MD 是用户手动编辑 + LLM 增强的文档，不是 runtime 高频变更状态；加完整 revision 合同需要设计多文件 manifest 格式，超出 E-P1-3 "写路径收口"范围。原子写已解决最危险的"半写可见"问题。预留枚举变体让未来加 revision 合同零成本接入（无需改 manifest 格式或担心序列化兼容）。
- 不选 A2（完整 revision 合同）：范围过大，应作为独立 issue 跟进。

### 决策 B：锁策略

- **选定 B1**：`AnalysisService` 写盘时持有 `character_lock(character_id).read()`（与 `LorebookService` 读取一致），不加 `state_lock`（analysis 不是 state 资产）。
- 理由：能串行化同一 character 的并发写，与现有锁序兼容。analysis 不参与 LOCK-ORDER 合同的嵌套路径（不与 `session_lock`/`state_lock` 交互）。

### 决策 C：Service 方法签名（同步 vs async）

- **选定 C1**：`AnalysisService` 用**同步 `std::fs` + `replace_file`**，与现有 5 个 Service（`LorebookService`/`StateService`/`PersonaService`/`PlotService`/`WorldEventService`）一致。
- 理由：现有 Service 都被 async 代码（`Tool::call` / daemon handler）调用且均为同步。若 analysis 用 async，会产生"domain 层一半同步一半 async"的分裂。如果未来要改 async，应该是 domain 层统一迁移（属于代际重构范畴），不是单个 Service 的决策。保持同步 = 保持一致性 = 未来一次性迁移更容易。

### 决策 D（新增）：`spawn_blocking` 包装调用边界

- **选定 D**：`AnalysisService` 内部保持同步，但在两个 async 调用点（`analysis.rs::ApplyEnhancedAnalysisTool::call` 和 `decompose_handlers.rs` 的 `apply` 分支）用 `tokio::task::spawn_blocking` 包装 `AnalysisService` 调用。
- 理由：原代码用 `tokio::fs::write`（内部已通过 `spawn_blocking` 卸载到 blocking 线程池），改同步 `std::fs` 裸调用会从"卸载到 blocking 池"倒退到"占用 tokio worker 线程"，是真实性能回归。`search.rs:44` 已有 `spawn_blocking` 既定惯例。这是维持"零行为变化"承诺的最低要求，不是扩展性加分项。
- 读路径（`EnhanceAnalysisTool` 读盘、`get_character_analysis_file`、`enhance` 分支）同样加 `spawn_blocking` 包装，保持读路径与写路径一致。
- **边界文档化**：`AnalysisService` 注释明确标注"进程内 `character_lock`，不跨进程；调用方在 async context 应使用 `spawn_blocking` 包装"。

### 决策 E：world_book 拒绝逻辑收口

- `load_file` / `save_file` 内部做 `world_book/` 前缀拒绝（而非调用方）。调用方只传 filename，Service 决定是否可操作。
- 理由：资产边界规则收口到 Service，未来加新调用方无需重复实现。
- **路径安全校验不重复实现**：`char_analysis_file_path` 已内置完整白名单校验（`[a-z0-9_/.-]+\.md` + `strip_prefix` + `Component::Normal`），`AnalysisService` 调用它即自动获得全部安全校验。

## 实施步骤（在同一 PR 内提交）

### Slice 1：创建 `engine/src/domain/analysis.rs`

```rust
//! Analysis MD domain service: read/write character analysis markdown files.
//!
//! Extracted from `agent/tools/analysis.rs` + `daemon/decompose_handlers.rs`
//! (E-P1-3 P0). Behavior changes:
//! - Writes are now atomic (replace_file) — eliminates half-write visibility.
//! - Concurrent writes serialized via character_lock.
//!
//! Boundary: character_lock is process-local; it does NOT protect against
//! multi-process or out-of-process writes. Callers in async context MUST
//! wrap Service calls with `tokio::task::spawn_blocking` to avoid blocking
//! tokio worker threads (the original code used `tokio::fs::write` which
//! internally offloads to a blocking pool — see PR #431 audit Point 4).
//!
//! Known gap (last-write-wins): character_lock serializes writes but does
//! NOT detect semantic conflicts. Two sequential non-conflicting writes
//! (each atomic, each holding the lock) can still silently overwrite each
//! other's content. This is documented as a known gap; future revision
//! contract or optimistic concurrency will address it.

use std::path::{Path, PathBuf};
use crate::error::AirpError;
use crate::data_dir;
use super::locks::character_lock;

#[derive(Clone, Debug)]
pub struct AnalysisService {
    data_root: PathBuf,
}

const WORLD_BOOK_REJECT_MSG: &str = "world_book entries are read-only";

impl AnalysisService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self { data_root: data_root.as_ref().to_path_buf() }
    }

    pub fn load_file(&self, character_id: &str, filename: &str) -> Result<String, AirpError> {
        if filename.starts_with("world_book/") {
            return Err(AirpError::BadRequest(WORLD_BOOK_REJECT_MSG.into()));
        }
        let _guard = character_lock(character_id).read().unwrap_or_else(|p| p.into_inner());
        let path = data_dir::char_analysis_file_path(&self.data_root, character_id, filename)?;
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(AirpError::NotFound(format!(
                    "analysis file {filename} not found for character {character_id}"
                )))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_file(&self, character_id: &str, filename: &str, content: &str) -> Result<(), AirpError> {
        if filename.starts_with("world_book/") {
            return Err(AirpError::BadRequest(WORLD_BOOK_REJECT_MSG.into()));
        }
        let _guard = character_lock(character_id).read().unwrap_or_else(|p| p.into_inner());
        let path = data_dir::char_analysis_file_path(&self.data_root, character_id, filename)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::data_dir::replace_file(&path, content.as_bytes())?;
        Ok(())
    }
}
```

### Slice 2：更新 `engine/src/domain/mod.rs`

```rust
mod analysis;
pub use analysis::AnalysisService;
```

### Slice 3：更新 `engine/src/agent/tools/analysis.rs`

- `EnhanceAnalysisTool::call` 读盘改为 `spawn_blocking` 包装 `AnalysisService::load_file`
- `ApplyEnhancedAnalysisTool::call` 写盘改为 `spawn_blocking` 包装 `AnalysisService::save_file`
- 删除两处 `tokio::fs::try_exists` / `read_to_string` / `write` + world_book 拒绝逻辑（已移入 Service）

```rust
// 调用点示例（analysis.rs ApplyEnhancedAnalysisTool::call）
let svc = AnalysisService::new(state.data_root.clone());
let cid_str = cid.as_str().to_string();
let filename_str = filename.to_string();
let enhanced_md_owned = enhanced_md.clone();
tokio::task::spawn_blocking(move || {
    svc.save_file(&cid_str, &filename_str, &enhanced_md_owned)
}).await
    .map_err(|e| AirpError::Internal(format!("analysis save task failed: {e}")))??;
```

### Slice 4：更新 `engine/src/daemon/decompose_handlers.rs`

- `get_character_analysis_file` 读盘改为 `spawn_blocking` 包装 `AnalysisService::load_file`
- `enhance_or_apply_character_analysis` 的 `enhance` 分支读盘、`apply` 分支写盘同样用 `spawn_blocking` 包装

### Slice 5：预留 `AssetKind::Analysis` 枚举变体

- `engine/src/revision/manifest.rs`：在 `AssetKind` 枚举加 `Analysis` 变体 + `as_str` 分支返回 `"analysis"`
- 加序列化往返测试（参照 `asset_kind_world_events_serializes_lowercase`）

### Slice 6：验证

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --locked`（含 3 个 analysis 测试 + manifest 序列化测试 + 神圣不变式 `subagent_context_has_no_orchestrator_noise`）
- 转 PR ready，等 CI 7/7，合并

## 风险与回滚

- **零功能行为变化**：类型和接口不变，仅写盘从 `tokio::fs::write` 改为 `replace_file`（原子性**提升**，半写可见问题消除）。
- **性能不回归**：原 `tokio::fs::write` 内部通过 `spawn_blocking` 卸载到 blocking 池；本 PR 在调用点显式 `spawn_blocking` 包装同步 `AnalysisService`，保持原有"不阻塞 worker"行为（审计 Point 4 修正）。
- **已知 gap：last-write-wins 静默丢失风险**。`character_lock` 串行化写操作、保证原子性，但**不检测、不阻止**两次语义上冲突的写操作互相覆盖（last-write-wins 静默数据丢失风险依然存在）。此风险由未来的 revision 合同或乐观并发机制解决，本 PR 不处理。**follow-up issue 跟踪**。
- **回归风险低**：现有 3 个 analysis 测试覆盖 enhance readonly / apply dry-run→confirm / world_book 拒绝 / error precedence。
- **回滚**：单 PR squash merge，revert 即可。

## 不在本次范围

- 完整 revision 合同（`AssetKind::Analysis` 的 `commit_revision` / `revisions/` 目录 / manifest 多文件格式）—— follow-up issue 跟踪
- last-write-wins 静默丢失风险的解决（CAS / revision 合同）—— follow-up issue 跟踪
- domain 层 sync IO 技术债统一治理（现有 5 个 Service 同样有 async 裸调用同步 IO 的问题）—— follow-up issue 跟踪
- `decompose.rs` 的 6 处 `tokio::fs::write`（初始拆卡生成，一次性产物，非 runtime 写盘点）
- `volume_context.rs` 的 2 处写盘（P2，export bundle 一次性产物）

## Follow-up issues（PR 合并后开）

1. **analysis last-write-wins 静默丢失风险**：文档化 + 与 revision 合同一起设计解决方案
2. **domain 层 sync IO 技术债**：现有 5 个 Service + AnalysisService 在 async 调用方的 `spawn_blocking` 包装统一治理

## 关联

- 上游：#381 E-P1-3（Domain write path 唯一化）
- 先例：#429（WorldEventService）、#430（PlotService）
- 审计：PR #431 评论（@GhostXia 替代设计方案 + 反驳分析）
