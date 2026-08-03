//! Analysis MD domain service: read/write character analysis markdown files.
//!
//! Extracted from `agent/tools/analysis.rs` + `daemon/decompose_handlers.rs`
//! (E-P1-3 P0). Behavior changes vs original:
//! - Writes are now atomic (`data_dir::replace_file` — tmp + rename + fsync),
//!   eliminating half-write visibility of the original `tokio::fs::write`.
//! - Concurrent writes for the same character are now serialized via
//!   `character_lock(character_id).read()`.
//!
//! # Boundary assumptions (callers MUST read)
//!
//! - **`character_lock` is process-local.** It does NOT protect against
//!   multi-process deployments, out-of-process CLI tools, or object-storage
//!   backends. The lock silently no-ops in those scenarios. Documented as a
//!   known gap; future revision contract or optimistic concurrency will
//!   address cross-process correctness.
//! - **Callers in async context MUST wrap Service calls with
//!   `tokio::task::spawn_blocking`.** The original code used `tokio::fs::write`
//!   which internally offloads blocking syscalls to a dedicated thread pool;
//!   calling the sync `std::fs` inside this Service directly from an async
//!   task would occupy a tokio worker thread and is a real performance
//!   regression (see PR #431 audit Point 4). `search.rs:44` sets the
//!   project precedent of `spawn_blocking`-wrapping sync calls from async
//!   tools.
//!
//! # Known gap: last-write-wins silent loss
//!
//! `character_lock` serializes writes and guarantees atomicity, but does NOT
//! detect semantic conflicts. Two sequential non-conflicting writes (each
//! atomic, each holding the lock) can still silently overwrite each other's
//! content. For example: user edits `basic_info.md` in WebUI while an agent
//! `apply_enhanced_analysis` finishes — the later write wins, the earlier
//! content is lost with no error. This is documented as a known gap; future
//! revision contract or optimistic concurrency (CAS) will address it. See
//! follow-up issue tracked in PR #431.

use std::path::{Path, PathBuf};

use crate::data_dir;
use crate::error::AirpError;

use super::locks::character_lock;

/// #160 A2：world_book 条目只读，`AnalysisService` 与原 enhance/apply 路径共享同一文案。
/// 原实现两路径各自硬编码 "not eligible for enhance"，apply 路径描述不准确。
/// 提取到 Service 时保留原文案以维持错误消息兼容（现有测试断言此文案）。
const WORLD_BOOK_REJECT_MSG: &str =
    "world_book entries are read-only and not eligible for enhance or apply (issue #87)";

/// Analysis MD domain service.
///
/// See module-level docs for boundary assumptions and known gaps.
#[derive(Clone, Debug)]
pub struct AnalysisService {
    data_root: PathBuf,
}

impl AnalysisService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    /// 读取 analysis MD 文件。
    ///
    /// - 拒绝 `world_book/` 前缀（资产边界规则，#160 A2）。
    /// - 路径安全校验（白名单 `[a-z0-9_/.-]+\.md`、拒绝 `..` / 绝对路径 / 非 .md 扩展）
    ///   由 `data_dir::char_analysis_file_path` 内置，本 Service 不重复实现。
    /// - 文件不存在返回 `AirpError::NotFound`（与原 `tokio::fs::try_exists` 行为一致）。
    ///
    /// **调用方在 async context 必须用 `tokio::task::spawn_blocking` 包装**（见模块文档）。
    pub fn load_file(&self, character_id: &str, filename: &str) -> Result<String, AirpError> {
        if filename.starts_with("world_book/") {
            return Err(AirpError::BadRequest(WORLD_BOOK_REJECT_MSG.into()));
        }
        let character = character_lock(character_id);
        let _guard = character.read().unwrap_or_else(|p| p.into_inner());
        let path = data_dir::char_analysis_file_path(&self.data_root, character_id, filename)?;
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(AirpError::NotFound(
                format!("analysis file {filename} not found for character {character_id}"),
            )),
            Err(e) => Err(e.into()),
        }
    }

    /// 保存 analysis MD 文件（原子写）。
    ///
    /// - 拒绝 `world_book/` 前缀（资产边界规则，#160 A2）。
    /// - 路径安全校验由 `data_dir::char_analysis_file_path` 内置。
    /// - 写盘走 `data_dir::replace_file`（tmp + rename + fsync），消除半写可见。
    /// - `character_lock` 串行化同一 character 的并发写。
    ///
    /// **调用方在 async context 必须用 `tokio::task::spawn_blocking` 包装**（见模块文档）。
    pub fn save_file(
        &self,
        character_id: &str,
        filename: &str,
        content: &str,
    ) -> Result<(), AirpError> {
        if filename.starts_with("world_book/") {
            return Err(AirpError::BadRequest(WORLD_BOOK_REJECT_MSG.into()));
        }
        let character = character_lock(character_id);
        let _guard = character.read().unwrap_or_else(|p| p.into_inner());
        let path = data_dir::char_analysis_file_path(&self.data_root, character_id, filename)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::data_dir::replace_file(&path, content.as_bytes())?;
        Ok(())
    }
}
