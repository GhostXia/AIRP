//! 风格系统模块：Style Review + Soul-Drift 动态人格 + Style Learn 风格迁移。
//!
//! ## 架构
//! - `drift`: Soul-Drift 动态人格 overlay（read/write/inject/compress）
//! - `review`: 风格审查（LLM 驱动的风格一致性检查）
//! - `learn`: Phase 4.2 风格迁移（从文本样本提取风格特征写入 profile）
//!
//! ## Soul-Drift 语义
//! - Base + drift 双层：原角色卡 = 不可变 base；`soul_drift.md` = 学习式 overlay
//! - 注入时叠加于 base 之上（prompt assembly 的 card 段后追加 drift 段）
//! - 可读可审可回滚（revision 合同复用）
//! - Frozen snapshot：本轮写入，下轮注入
//!
//! ## Style Learn 语义（Phase 4.2）
//! - Profile 是参考指南，drift 是动态修正；二者解耦
//! - `POST /v1/style/learn` 覆盖写 `styles/profiles/{id}.md`
//! - `POST /v1/style/review` 读取 profile 做风格对齐检查

mod drift;
mod learn;
mod review;

pub use drift::{
    append_soul_drift, append_soul_drift_with_compression, compress_soul_drift_if_needed,
    inject_soul_drift, read_soul_drift, read_soul_drift_with_revision, rollback_soul_drift,
    write_soul_drift, SoulDriftConfig, SOUL_DRIFT_DEFAULT_CAP,
};
pub use learn::{
    character_profile_path, features_count, global_profile_path, render_profile_entries,
    render_profile_header, render_profile_markdown, run_style_learn, validate_profile_id,
    write_profile, StyleFeatures, StyleLearnRequest, StyleLearnResponse,
};
pub use review::{run_style_review, run_style_review_for_character, StyleReviewReport};
