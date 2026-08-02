//! 记忆系统模块：常驻有界记忆 + 自动事实抽取 + 用户模型学习 + 遗忘曲线。
//!
//! ## 架构
//! - `resident`: 每角色/每 session 一份有界 markdown（`resident.md`）
//! - `extract`: 从对话中异步抽取关键事实（控制平面 LLM 调用）
//! - `compress`: 超限时 LLM 合并压缩
//! - `decay`: 遗忘曲线——按 importance * recency 衰减，低于阈值的条目淡出
//! - `user_model`: 每用户一份偏好模型（`user_model.md`）。HTTP 手动编辑 +
//!   finalize 异步抽取（阶段二补全 D1 已接入）。
//!
//! ## Frozen Snapshot 语义
//! 本轮抽取落盘 → 下轮 prepare 阶段才注入 prompt（防模型自反应）。
//!
//! ## PR #271 审计修复（B3）
//! 原 `user_model.rs` 暴露了 `inject_user_model` / `append_user_model` /
//! `USER_PREFERENCE_EXTRACTION_PROMPT` 但全程无人调用，且 prepare 路径未接入。
//! MVP 范围内只做手动编辑，相关死代码已删除，待后续 PR 真正接入抽取/注入时再加回。

use crate::error::AirpError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

mod compress;
pub mod decay;
mod extract;
pub mod fts;
mod resident;
mod user_model;

pub use compress::compress_resident_memory;
pub use decay::{apply_decay, read_faded, reinforce_entry, DecayConfig, DecayResult};
pub(crate) use decay::{apply_decay_to_resident, commit_extracted_facts};
pub use extract::{extract_facts, extract_user_preferences, ExtractionConfig};
pub use fts::{FtsStore, SearchResult};
pub(crate) use resident::write_resident_memory_if_unchanged;
pub use resident::{
    append_resident_memory, inject_resident_memory, is_over_capacity, read_resident_memory,
    write_resident_memory, ResidentMemoryConfig, RESIDENT_MEMORY_DEFAULT_CAP,
};
pub use user_model::{
    append_user_model_in_home, read_user_model, write_user_model, USER_MODEL_CAP,
};

type MemoryMutationRegistry = Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>;

/// Serialize resident/decay/faded mutations for one session. Weak entries keep
/// the process-wide registry bounded after inactive sessions release the lock.
static MEMORY_MUTATION_LOCKS: LazyLock<MemoryMutationRegistry> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn memory_mutation_lock(session_dir: &Path) -> Arc<Mutex<()>> {
    let mut locks = MEMORY_MUTATION_LOCKS.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("memory mutation lock registry was poisoned; recovering");
        poisoned.into_inner()
    });
    locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(lock) = locks.get(session_dir).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(session_dir.to_path_buf(), Arc::downgrade(&lock));
    lock
}

pub(crate) fn with_memory_mutation<T>(
    session_dir: &Path,
    mutate: impl FnOnce() -> Result<T, AirpError>,
) -> Result<T, AirpError> {
    let lock = memory_mutation_lock(session_dir);
    let _guard = lock.lock().unwrap_or_else(|poisoned| {
        tracing::warn!(path = %session_dir.display(), "memory mutation lock was poisoned; recovering");
        poisoned.into_inner()
    });
    mutate()
}
