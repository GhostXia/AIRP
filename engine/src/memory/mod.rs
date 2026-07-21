//! 记忆系统模块：常驻有界记忆 + 自动事实抽取 + 用户模型学习。
//!
//! ## 架构
//! - `resident`: 每角色/每 session 一份有界 markdown（`resident.md`）
//! - `extract`: 从对话中异步抽取关键事实（控制平面 LLM 调用）
//! - `compress`: 超限时 LLM 合并压缩
//! - `user_model`: 每用户一份偏好模型（`user_model.md`），MVP 仅支持手动编辑
//!
//! ## Frozen Snapshot 语义
//! 本轮抽取落盘 → 下轮 prepare 阶段才注入 prompt（防模型自反应）。
//!
//! ## PR #271 审计修复（B3）
//! 原 `user_model.rs` 暴露了 `inject_user_model` / `append_user_model` /
//! `USER_PREFERENCE_EXTRACTION_PROMPT` 但全程无人调用，且 prepare 路径未接入。
//! MVP 范围内只做手动编辑，相关死代码已删除，待后续 PR 真正接入抽取/注入时再加回。

mod compress;
mod extract;
mod resident;
mod user_model;

pub use compress::compress_resident_memory;
pub use extract::{extract_facts, ExtractionConfig};
pub use resident::{
    append_resident_memory, inject_resident_memory, is_over_capacity, read_resident_memory,
    write_resident_memory, ResidentMemoryConfig, RESIDENT_MEMORY_DEFAULT_CAP,
};
pub use user_model::{read_user_model, write_user_model};
