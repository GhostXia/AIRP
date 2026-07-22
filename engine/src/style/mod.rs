//! 风格系统模块：Style Review + Soul-Drift 动态人格。
//!
//! ## 架构
//! - `drift`: Soul-Drift 动态人格 overlay（read/write/inject/compress）
//! - `review`: 风格审查（LLM 驱动的风格一致性检查）
//!
//! ## Soul-Drift 语义
//! - Base + drift 双层：原角色卡 = 不可变 base；`soul_drift.md` = 学习式 overlay
//! - 注入时叠加于 base 之上（prompt assembly 的 card 段后追加 drift 段）
//! - 可读可审可回滚（revision 合同复用）
//! - Frozen snapshot：本轮写入，下轮注入

mod drift;
mod review;

pub use drift::{
    append_soul_drift, append_soul_drift_with_compression, compress_soul_drift_if_needed,
    inject_soul_drift, read_soul_drift, read_soul_drift_with_revision, rollback_soul_drift,
    write_soul_drift, SoulDriftConfig, SOUL_DRIFT_DEFAULT_CAP,
};
pub use review::{run_style_review, run_style_review_for_character, StyleReviewReport};
