//! 记忆系统模块：常驻有界记忆 + 自动事实抽取 + 用户模型学习。
//!
//! ## 架构
//! - `resident`: 每角色/每 session 一份有界 markdown（`resident.md`）
//! - `extract`: 从对话中异步抽取关键事实（控制平面 LLM 调用）
//! - `compress`: 超限时 LLM 合并压缩
//! - `user_model`: 每用户一份偏好模型（`user_model.md`）
//!
//! ## Frozen Snapshot 语义
//! 本轮抽取落盘 → 下轮 prepare 阶段才注入 prompt（防模型自反应）。

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
pub use user_model::{inject_user_model, read_user_model, write_user_model};
