//! Analysis MD domain service: read/write character analysis markdown files.
//!
//! Extracted from `agent/tools/analysis.rs` + `daemon/decompose_handlers.rs`
//! (E-P1-3 P0). Behavior changes vs original:
//! - Writes are now atomic (`data_dir::replace_file` — tmp + rename + fsync),
//!   eliminating half-write visibility of the original `tokio::fs::write`.
//! - Concurrent writes for the same character are now serialized via
//!   `character_lock(character_id).write()` (#503 post-merge fix: write-lock
//!   required for CAS+write atomicity; `.read()` caused a TOCTOU race where
//!   two writers with identical `expected_hash` could both pass CAS then
//!   silently overwrite each other).
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
//! # Optimistic concurrency (CAS): `expected_hash`
//!
//! `save_file` accepts an optional `expected_hash` parameter. When `Some`,
//! the Service reads the current file content, computes its SHA-256, and
//! compares against `expected_hash` before writing. A mismatch returns
//! `AirpError::Conflict` (HTTP 409), letting the caller re-enhance from the
//! fresh content instead of silently overwriting.
//!
//! Typical flow: `enhance_analysis` returns `original_md_hash` → user reviews
//! diff → `apply_enhanced_analysis` passes `expected_hash = original_md_hash`
//! → `save_file` rejects if the file changed between enhance and apply.
//!
//! `expected_hash = None` skips the check (backward-compatible for callers
//! that don't participate in CAS).

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

    /// 计算 content 的 SHA-256 hex 指纹，用于 CAS `expected_hash` 比对。
    ///
    /// 调用方（enhance 端点/agent tool）在读取文件后调用本方法得到 `original_md_hash`，
    /// 在 apply 时作为 `expected_hash` 传回 `save_file`。
    pub fn content_hash(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(content.as_bytes());
        format!("{:x}", digest)
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

    /// 保存 analysis MD 文件（原子写 + 可选 CAS）。
    ///
    /// - 拒绝 `world_book/` 前缀（资产边界规则，#160 A2）。
    /// - 路径安全校验由 `data_dir::char_analysis_file_path` 内置。
    /// - 写盘走 `data_dir::replace_file`（tmp + rename + fsync），消除半写可见。
    /// - `character_lock` **写锁** 串行化同一 character 的并发写，保证 CAS 检查
    ///   + 写入的原子性（TOCTOU 安全，#503 修复）。
    /// - `expected_hash = Some(h)`：写入前校验当前文件 SHA-256 是否匹配，不匹配返回
    ///   `AirpError::Conflict`（HTTP 409）。文件不存在但 expected_hash 为 Some 也返回
    ///   Conflict（enhance 时文件存在，现在不存在说明被删/改名）。
    /// - `expected_hash = None`：跳过校验（向后兼容）。
    ///
    /// **调用方在 async context 必须用 `tokio::task::spawn_blocking` 包装**（见模块文档）。
    pub fn save_file(
        &self,
        character_id: &str,
        filename: &str,
        content: &str,
        expected_hash: Option<&str>,
    ) -> Result<(), AirpError> {
        if filename.starts_with("world_book/") {
            return Err(AirpError::BadRequest(WORLD_BOOK_REJECT_MSG.into()));
        }
        let character = character_lock(character_id);
        // #503 修复：必须用写锁（write）而非读锁（read）。CAS 检查 + 文件写入必须
        // 在同一临界区内原子完成，否则两写入者可同时通过 CAS 后 last-write-wins 静默丢失。
        let _guard = character.write().unwrap_or_else(|p| p.into_inner());
        let path = data_dir::char_analysis_file_path(&self.data_root, character_id, filename)?;

        // CAS: 如果调用方传了 expected_hash，写入前校验当前文件未被修改。
        // 与 replace_file 写盘受同一把写锁保护，TOCTOU 安全。
        if let Some(expected) = expected_hash {
            let current = std::fs::read_to_string(&path).map_err(|e| {
                // 文件不存在但 expected_hash 为 Some → 冲突（enhance 时文件存在）
                if e.kind() == std::io::ErrorKind::NotFound {
                    AirpError::Conflict(format!(
                        "analysis file {filename} was deleted after enhance; refresh and re-enhance"
                    ))
                } else {
                    AirpError::from(e)
                }
            })?;
            let actual = Self::content_hash(&current);
            if actual != expected {
                return Err(AirpError::Conflict(format!(
                    "analysis file {filename} changed after enhance (expected hash {expected}, got {actual}); refresh and re-enhance"
                )));
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::data_dir::replace_file(&path, content.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_file(tmp: &TempDir, cid: &str, filename: &str, content: &str) -> AnalysisService {
        let svc = AnalysisService::new(tmp.path());
        let _ = svc.save_file(cid, filename, content, None);
        svc
    }

    /// #432: hash 匹配时正常写入
    #[test]
    fn save_file_with_expected_hash_accepts_unchanged() {
        let tmp = TempDir::new().unwrap();
        let cid = "alice";
        let original = "# Basic Info\n\nplaceholder";
        let svc = setup_file(&tmp, cid, "basic_info.md", original);

        let hash = AnalysisService::content_hash(original);
        let enhanced = "# Basic Info\n\nfilled by LLM";
        svc.save_file(cid, "basic_info.md", enhanced, Some(&hash))
            .unwrap();

        let loaded = svc.load_file(cid, "basic_info.md").unwrap();
        assert_eq!(loaded, enhanced);
    }

    /// #432: 文件在 enhance 后被修改，hash 不匹配 → Conflict
    #[test]
    fn save_file_with_expected_hash_rejects_stale_content() {
        let tmp = TempDir::new().unwrap();
        let cid = "alice";
        let original = "# Basic Info\n\nplaceholder";
        let svc = setup_file(&tmp, cid, "basic_info.md", original);

        let hash = AnalysisService::content_hash(original);

        // 模拟 enhance→apply 之间文件被修改
        std::fs::write(
            tmp.path()
                .join("characters")
                .join(cid)
                .join("analysis")
                .join("basic_info.md"),
            "# Basic Info\n\nuser edited manually",
        )
        .unwrap();

        let enhanced = "# Basic Info\n\nfilled by LLM";
        let err = svc
            .save_file(cid, "basic_info.md", enhanced, Some(&hash))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::Conflict(_)),
            "expected Conflict, got {err:?}"
        );

        // 文件内容不应被覆盖
        let loaded = svc.load_file(cid, "basic_info.md").unwrap();
        assert!(loaded.contains("user edited manually"));
    }

    /// #432: expected_hash = None 跳过 CAS（向后兼容）
    #[test]
    fn save_file_without_expected_hash_skips_check() {
        let tmp = TempDir::new().unwrap();
        let cid = "alice";
        let original = "# Basic Info\n\nplaceholder";
        let svc = setup_file(&tmp, cid, "basic_info.md", original);

        // 文件被修改但不传 expected_hash → 应该直接覆盖
        std::fs::write(
            tmp.path()
                .join("characters")
                .join(cid)
                .join("analysis")
                .join("basic_info.md"),
            "# Basic Info\n\nuser edited manually",
        )
        .unwrap();

        let enhanced = "# Basic Info\n\nfilled by LLM";
        svc.save_file(cid, "basic_info.md", enhanced, None).unwrap();

        let loaded = svc.load_file(cid, "basic_info.md").unwrap();
        assert_eq!(loaded, enhanced);
    }

    /// #432: 文件在 enhance 后被删除，expected_hash 为 Some → Conflict
    #[test]
    fn save_file_with_expected_hash_rejects_when_file_deleted() {
        let tmp = TempDir::new().unwrap();
        let cid = "alice";
        let original = "# Basic Info\n\nplaceholder";
        let svc = setup_file(&tmp, cid, "basic_info.md", original);

        let hash = AnalysisService::content_hash(original);

        // 模拟 enhance→apply 之间文件被删除
        std::fs::remove_file(
            tmp.path()
                .join("characters")
                .join(cid)
                .join("analysis")
                .join("basic_info.md"),
        )
        .unwrap();

        let enhanced = "# Basic Info\n\nfilled by LLM";
        let err = svc
            .save_file(cid, "basic_info.md", enhanced, Some(&hash))
            .unwrap_err();
        assert!(
            matches!(err, AirpError::Conflict(_)),
            "expected Conflict, got {err:?}"
        );
    }

    /// content_hash 确定性：相同内容产生相同 hash
    #[test]
    fn content_hash_is_deterministic() {
        let content = "# Test\n\nhello world";
        let h1 = AnalysisService::content_hash(content);
        let h2 = AnalysisService::content_hash(content);
        assert_eq!(h1, h2);
        assert_ne!(h1, AnalysisService::content_hash("# Test\n\nhello WORLD"));
    }

    // ── #503 回归：TOCTOU 并发写锁正确性 ────────────────────────────────
    //
    // 模拟真实场景：两用户几乎同时 enhance（拿到相同 original_md_hash = H），
    // 然后同时 apply 传 expected_hash = H。修复前（.read() 锁）两写入者都能
    // 通过 CAS 检查后先后写入，后者覆盖前者，静默数据丢失。修复后（.write() 锁）
    // 第一个进入临界区的写入成功，第二个在临界区内重新读文件发现 hash 已变，
    // 返回 Conflict 拒绝写入，不丢数据。

    use std::sync::{Arc, Barrier};
    use std::thread;

    /// #503：两写入者传同一 expected_hash 并发 save_file → 恰好 1 个成功、1 个 Conflict。
    /// 验证写锁把 CAS 检查 + 写入包成了原子临界区，无 TOCTOU 静默丢失。
    #[test]
    fn concurrent_save_with_same_expected_hash_serializes_via_write_lock() {
        let tmp = TempDir::new().unwrap();
        let cid = "concurrent-char";
        let original = "# Basic Info\n\nplaceholder";
        let filename = "basic_info.md";
        let svc = Arc::new(setup_file(&tmp, cid, filename, original));

        let n_writers = 2usize;
        let barrier = Arc::new(Barrier::new(n_writers));
        let hash = AnalysisService::content_hash(original);

        let mut handles = Vec::with_capacity(n_writers);
        for i in 0..n_writers {
            let svc = Arc::clone(&svc);
            let barrier = Arc::clone(&barrier);
            let hash = hash.clone();
            let cid = cid.to_string();
            let filename = filename.to_string();
            handles.push(thread::spawn(move || {
                // 两线程尽量同时进入 save_file，放大竞态窗口
                barrier.wait();
                let content = format!("# Basic Info\n\nwritten by writer-{}", i);
                (i, svc.save_file(&cid, &filename, &content, Some(&hash)))
            }));
        }

        let mut results = Vec::with_capacity(n_writers);
        for h in handles {
            results.push(h.join().unwrap());
        }

        // 统计：恰好 1 个 Ok(())，恰好 1 个 Conflict
        let mut ok_count = 0usize;
        let mut conflict_count = 0usize;
        let mut winner_content: Option<String> = None;
        for (i, r) in &results {
            match r {
                Ok(()) => {
                    ok_count += 1;
                    // 读回实际文件内容，确认胜出者写入的内容确实落盘
                    let loaded = svc.load_file(cid, filename).unwrap();
                    let expected = format!("# Basic Info\n\nwritten by writer-{}", i);
                    assert_eq!(
                        loaded, expected,
                        "winner writer-{} content should match actual file",
                        i
                    );
                    winner_content = Some(loaded);
                }
                Err(AirpError::Conflict(msg)) => {
                    conflict_count += 1;
                    assert!(
                        msg.contains("changed after enhance"),
                        "Conflict message should mention stale content, got: {}",
                        msg
                    );
                }
                Err(other) => {
                    panic!("unexpected error for writer-{}: {:?}", i, other);
                }
            }
        }
        assert_eq!(ok_count, 1, "exactly 1 writer must succeed (write lock serializes CAS+write)");
        assert_eq!(
            conflict_count, 1,
            "exactly 1 writer must get Conflict (loser detects hash changed inside write lock)"
        );
        // 额外保险：最终文件内容不是 original，也不是两写入者内容的混合
        let final_content = svc.load_file(cid, filename).unwrap();
        assert_ne!(final_content, original, "file must be changed by the winner");
        assert!(
            winner_content.as_ref().unwrap() == &final_content,
            "final file must equal the winner's write, no silent overwrite"
        );
    }

    /// #503：三写入者并发（1 传 None 跳过 CAS + 2 传同一 expected_hash）→
    /// 写锁保证三者不重叠；跳过 CAS 的若第一个写则两传 hash 的都 Conflict；
    /// 若跳过 CAS 的最后写则它直接覆盖（符合 expected_hash=None 的语义）。
    /// 这里验证不崩溃、无数据损坏。
    #[test]
    fn concurrent_save_mixed_cas_modes_no_corruption() {
        let tmp = TempDir::new().unwrap();
        let cid = "mixed-cas-char";
        let original = "# Mixed\n\nstart";
        let filename = "mixed.md";
        let svc = Arc::new(setup_file(&tmp, cid, filename, original));

        let n = 3usize;
        let barrier = Arc::new(Barrier::new(n));
        let hash = AnalysisService::content_hash(original);

        let mut handles = Vec::with_capacity(n);
        for i in 0..n {
            let svc = Arc::clone(&svc);
            let barrier = Arc::clone(&barrier);
            let hash = hash.clone();
            let cid = cid.to_string();
            let filename = filename.to_string();
            handles.push(thread::spawn(move || {
                barrier.wait();
                let content = format!("# Mixed\n\nwriter-{} content here", i);
                // writer-1 跳过 CAS，其他传 expected_hash
                let eh = if i == 1 { None } else { Some(hash.clone()) };
                (i, svc.save_file(&cid, &filename, &content, eh.as_deref()))
            }));
        }

        for h in handles {
            let (i, r) = h.join().unwrap();
            match r {
                Ok(()) => {} // 成功：跳过 CAS 的，或第一个拿写锁传 hash 的
                Err(AirpError::Conflict(_)) => {} // 预期：拿锁晚的传 hash 者
                Err(other) => panic!("writer-{} unexpected error: {:?}", i, other),
            }
        }

        // 文件必须是有效的 UTF-8 完整内容（无半写、无截断、无两个 writer 内容混合）
        let final_content = svc.load_file(cid, filename).unwrap();
        assert!(
            final_content.starts_with("# Mixed\n\n"),
            "final file must preserve MD heading structure, got: {:?}",
            final_content
        );
        // 精确匹配某个 writer 的完整输出，确认不是两段写入的混合
        let valid_writes: Vec<String> = (0..n)
            .map(|i| format!("# Mixed\n\nwriter-{} content here", i))
            .collect();
        assert!(
            valid_writes.iter().any(|w| w == &final_content),
            "final file must equal exactly one writer's full write (no mix/truncate/partial), got: {:?}",
            final_content
        );
    }
}
