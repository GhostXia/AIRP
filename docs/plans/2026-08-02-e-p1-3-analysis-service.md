# E-P1-3 P0: AnalysisService 提取计划

> 本 PR 目前仅含本计划文档，**不含代码改动**。等审计反馈后，在该 PR 内继续提交实际实现。

## 现状（为什么是 P0 最危险）

`analysis` MD 资产有两处非原子写盘点，均用 `tokio::fs::write`（非 `replace_file`，无 tmp+rename+fsync，半写状态可被并发 reader 看到）：

1. `engine/src/agent/tools/analysis.rs:270` — `ApplyEnhancedAnalysisTool` 写 `characters/{id}/analysis/{filename}`
2. `engine/src/daemon/decompose_handlers.rs:305` — `enhance_or_apply_character_analysis` 的 `action=apply` 分支写同一文件

对比其他已收口资产：
- `world_events.json`：原子写 + `AssetKind::WorldEvents` + `revisions/{n}/` 快照合同
- `plot_arc.json`：原子写（`replace_file`），无 revision 合同但单文件
- `analysis/`：**非原子写 + 无 revision 合同 + 多文件资产**

此外无 `character_lock` 串行化，agent tool 与 daemon HTTP 端点可并发写同一文件。

## 设计决策

### 决策 A：revision 合同范围

- **选定 A1 + 预留 `AssetKind::Analysis` 枚举变体**：本 PR 只做原子写改造 + Service 收口，**不实现 revision 快照逻辑**。同时在 `revision/manifest.rs` 的 `AssetKind` 枚举加 `Analysis` 变体（`rename_all = "lowercase"` 自动序列化为 `"analysis"`），加序列化往返测试。
- 理由：analysis MD 是用户手动编辑 + LLM 增强的文档，不是 runtime 高频变更状态；加完整 revision 合同需要设计多文件 manifest 格式，超出 E-P1-3 "写路径收口"范围。原子写已解决最危险的"半写可见"问题。预留枚举变体让未来加 revision 合同零成本接入（无需改 manifest 格式或担心序列化兼容）。
- 不选 A2（完整 revision 合同）：范围过大，应作为独立 issue 跟进。

### 决策 B：锁策略

- **选定 B1**：`AnalysisService` 写盘时持有 `character_lock(character_id).read()`（与 `LorebookService` 读取一致），不加 `state_lock`（analysis 不是 state 资产）。
- 理由：能串行化同一 character 的并发写，与现有锁序兼容。analysis 不参与 LOCK-ORDER 合同的嵌套路径（不与 `session_lock`/`state_lock` 交互）。
- 不选 B2（无锁）：仅靠 `replace_file` 原子性保证"最后写入胜出"，但无法防止 enhance→apply 与 agent tool 并发写同一文件导致内容丢失。

### 决策 C：Service 方法签名（同步 vs async）

- **选定 C1**：`AnalysisService` 用**同步 `std::fs` + `replace_file`**，与现有 5 个 Service（`LorebookService`/`StateService`/`PersonaService`/`PlotService`/`WorldEventService`）一致。
- 理由：现有 Service 都被 async 代码（`Tool::call` / daemon handler）调用且均为同步。若 analysis 用 async，会产生"domain 层一半同步一半 async"的分裂。如果未来要改 async，应该是 domain 层统一迁移（属于代际重构范畴），不是单个 Service 的决策。保持同步 = 保持一致性 = 未来一次性迁移更容易。
- 不选 C2（async）：打破现有 Service 同步约定，制造 domain 层分裂。

### 额外决策：world_book 拒绝逻辑收口

- `load_file` / `save_file` 内部做 `world_book/` 路径拒绝（而非调用方）。调用方只传 filename，Service 决定是否可操作。
- 理由：资产边界规则收口到 Service，未来加新调用方（新 tool 或新端点）无需重复实现拒绝逻辑。未来加新规则（大小限制、编码校验）只改 Service。

## 实施步骤（审计通过后在同一 PR 内提交）

### Slice 1：创建 `engine/src/domain/analysis.rs`

```rust
//! Analysis MD domain service: read/write character analysis markdown files.
//!
//! Extracted from `agent/tools/analysis.rs` + `daemon/decompose_handlers.rs`
//! (E-P1-3 P0). Zero behavior change except: writes are now atomic
//! (replace_file) and serialized via character_lock.

use std::path::{Path, PathBuf};
use crate::error::AirpError;
use crate::data_dir;
use super::locks::character_lock;

#[derive(Clone, Debug)]
pub struct AnalysisService {
    data_root: PathBuf,
}

impl AnalysisService {
    pub fn new(data_root: impl AsRef<Path>) -> Self { ... }

    /// 拒绝 world_book/ 前缀（资产边界规则，#274）。
    fn validate_filename(filename: &str) -> Result<(), AirpError> {
        if filename.is_empty()
            || filename.starts_with("world_book/")
            || filename.contains("..")
            // ... 其他现有校验
        {
            return Err(AirpError::BadRequest(...));
        }
        Ok(())
    }

    pub fn load_file(&self, character_id: &str, filename: &str) -> Result<String, AirpError> {
        Self::validate_filename(filename)?;
        let _guard = character_lock(character_id).read().unwrap_or_else(|p| p.into_inner());
        let path = data_dir::char_analysis_file_path(&self.data_root, character_id, filename)?;
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(AirpError::NotFound(...))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_file(&self, character_id: &str, filename: &str, content: &str) -> Result<(), AirpError> {
        Self::validate_filename(filename)?;
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

- `ApplyEnhancedAnalysisTool::call` 写盘改为 `AnalysisService::new(&state.data_root).save_file(cid, filename, &enhanced_md)`
- `EnhanceAnalysisTool::call` 读盘改为 `AnalysisService::load_file`
- 删除两处 `tokio::fs::try_exists` / `read_to_string` / `write` + world_book 拒绝逻辑（已移入 Service）

### Slice 4：更新 `engine/src/daemon/decompose_handlers.rs`

- `get_character_analysis_file` 读盘改为 `AnalysisService::load_file`
- `enhance_or_apply_character_analysis` 的 `enhance` 分支读盘改为 `AnalysisService::load_file`，`apply` 分支写盘改为 `AnalysisService::save_file`

### Slice 5：预留 `AssetKind::Analysis` 枚举变体

- `engine/src/revision/manifest.rs`：在 `AssetKind` 枚举加 `Analysis` 变体 + `as_str` 分支返回 `"analysis"`
- 加序列化往返测试（参照 `asset_kind_world_events_serializes_lowercase`）

### Slice 6：验证

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --locked`（含 3 个 analysis 测试 + manifest 序列化测试 + 神圣不变式 `subagent_context_has_no_orchestrator_noise`）
- 开 PR，等 CI 7/7，合并

## 风险与回滚

- **零行为变化**：类型和接口不变，仅写盘从 `tokio::fs::write` 改为 `replace_file`（原子性**提升**，半写可见问题消除）
- **回归风险低**：现有 3 个 analysis 测试覆盖 enhance readonly / apply dry-run→confirm / world_book 拒绝 / error precedence
- **回滚**：单 PR squash merge，revert 即可

## 不在本次范围

- 完整 revision 合同（`AssetKind::Analysis` 的 `commit_revision` / `revisions/` 目录 / manifest 多文件格式）—— 若需要，单独 issue 跟进
- `decompose.rs` 的 6 处 `tokio::fs::write`（初始拆卡生成，一次性产物，非 runtime 写盘点）
- `volume_context.rs` 的 2 处写盘（P2，export bundle 一次性产物）

## 审计请求

请独立审计以下决策（按 AGENTS.md §Audit Agent Charter 三原则）：

1. **A1（仅原子写，不实现 revision 合同）是否足够**？还是应该在本次一并实现完整 revision 合同？我倾向 A1，因为 analysis MD 不是 runtime 高频变更状态，且多文件 manifest 格式设计超出"写路径收口"范围。
2. **B1（`character_lock.read()`）锁粒度是否合适**？是否需要专用锁或 `write()` 锁？
3. **C1（同步 `std::fs`）是否正确**？还是应该在 analysis 率先采用 async，作为 domain 层 async 迁移的起点？
4. **world_book 拒绝逻辑移入 Service** 是否合理？还是应该保留在调用方？
5. **`AssetKind::Analysis` 枚举变体预留** 是否合适？还是应该等真正实现 revision 合同时再加？
6. 是否有遗漏的写盘点或更好的设计？

## 关联

- 上游：#381 E-P1-3（Domain write path 唯一化）
- 先例：#429（WorldEventService）、#430（PlotService）
- 跟进：完整 revision 合同（待独立 issue）
