//! 统一 revision/provenance 底层模块（#115 Phase 2a）。
//!
//! 本模块提供 per-asset 不可变版本化的共享底层：
//! - [`tree_hash`]：`AIRP-TREE-SHA256-v1` 算法（参考 `docs/SESSION-DATA-DESIGN.md` §4 第 5 条）
//! - [`manifest`]：`RevisionManifest` schema 与加载校验
//! - [`atomic`]：atomic commit 流程（staging → 全量校验 → rename → 更新 `current_revision`）
//!
//! ## 设计原则
//!
//! 1. **per-asset-id 独立 revision 空间**：每个 `character_id` / `preset_id` / 等有自己的 revision 计数器。
//! 2. **不可伪造**：revision 必须由内容 hash 派生或与内容 hash 共同持久化；禁止用 mtime 冒充。
//! 3. **不破坏现有数据**：通过 lazy migration 升级，旧格式作为兼容回退保留。
//! 4. **乐观锁统一**：所有 asset 的写操作支持 `expected_revision` 参数。
//!
//! ## 不在范围
//!
//! - session 自包含存档（SESSION-DATA-DESIGN.md §4 session manifest）
//! - Persona base lock / drift / rollback（属 #114）
//! - gating 版本化（spec §6.5 裁定为独立阶段）

pub(crate) mod atomic;
pub(crate) mod manifest;
pub(crate) mod tree_hash;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_hash_algorithm_label_is_stable() {
        assert_eq!(tree_hash::TREE_HASH_ALGORITHM, "AIRP-TREE-SHA256-v1");
    }
}
