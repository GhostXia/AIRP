use crate::error::AirpError;
use crate::revision::atomic::{
    commit_revision, next_content_revision, CommitOptions, StagedRevision,
};
use crate::revision::manifest::{AssetKind, AssetSource};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 估算文本的 token 数。
///
/// **精度局限（M0 F-22 / 6.0f）**：这是一个粗糙启发式，仅用于卷系统的软/硬压力阈值判断，
/// 不应作为计费或上下文窗口管理的依据。
///   - ASCII：4 字符 ≈ 1 token（OpenAI 经验值，对短词偏高，对长 URL 偏低）
///   - 非 ASCII（CJK / emoji）：1 字符 ≈ 1.5 tokens（GPT-4 tokenizer 实测约 1.0–2.5，取中位）
///   - **与真实 tokenizer 的偏差可达 ±30%**。若需精确值，应接入 `tiktoken-rs`。
///
/// 因实际使用场景（卷系统封卷触发）允许较大误差容忍度，目前保持启发式以避免引入
/// 大依赖 + WASM/ARM 平台兼容性问题。
pub fn estimate_tokens(text: &str) -> usize {
    let mut ascii_count: usize = 0;
    let mut cjk_count: usize = 0;
    for c in text.chars() {
        if c.is_ascii() {
            ascii_count += 1;
        } else {
            cjk_count += 1;
        }
    }
    (ascii_count / 4) + (cjk_count * 3 / 2)
}

/// 返回 session 目录下 current.md 的完整路径。
fn current_path(session_dir: &Path) -> PathBuf {
    session_dir.join("current.md")
}

/// 返回 session 目录下 index.md 的完整路径。
fn index_path(session_dir: &Path) -> PathBuf {
    session_dir.join("index.md")
}

/// 返回 session 目录下 volumes 子目录的完整路径。
fn volumes_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("volumes")
}

/// 返回某个卷编号对应的文件路径，例如 vol_001.md。
fn volume_path(session_dir: &Path, number: u32) -> PathBuf {
    volumes_dir(session_dir).join(format!("vol_{:03}.md", number))
}

/// 确保 session 目录及其 volumes 子目录存在，并初始化空的 current.md 与 index.md（如果不存在）。
pub fn ensure_session_dirs(session_dir: &Path) -> Result<(), AirpError> {
    fs::create_dir_all(session_dir)?;
    fs::create_dir_all(volumes_dir(session_dir))?;

    let cp = current_path(session_dir);
    if !cp.exists() {
        fs::write(&cp, "")?;
    }

    let ip = index_path(session_dir);
    if !ip.exists() {
        let initial = "# 全局索引\n\n## 人物\n\n## 物品\n\n## 悬挂线索\n\n## 地点\n\n## [已归档]\n";
        fs::write(&ip, initial)?;
    }

    Ok(())
}

/// #115 Phase 2f：Memory 接入统一 revision 合同。
///
/// 在 `session_dir/` 下新增 `revisions/{content_revision}/` + `current_revision`，
/// 批准文件为 `current.md` + `index.md`（封卷时额外含 `volumes/vol_NNN.md`）。
///
/// - lazy migration：首次 commit 时 `current_revision` 不存在则 `content_revision=1`
/// - provenance：source_hash = 所有批准文件拼接的 SHA-256
fn commit_memory_revision(
    session_dir: &Path,
    extra_files: Vec<(String, Vec<u8>)>,
    source_kind: &str,
) -> Result<u64, AirpError> {
    use sha2::{Digest, Sha256};
    // 使用 next_content_revision 跳过 orphan revision_dir（详见 atomic::next_content_revision 文档）。
    let content_revision = next_content_revision(session_dir)?;
    let current_bytes = fs::read(current_path(session_dir))?;
    let index_bytes = fs::read(index_path(session_dir))?;
    let mut files = vec![
        ("current.md".to_string(), current_bytes.clone()),
        ("index.md".to_string(), index_bytes.clone()),
    ];
    // CodeRabbit #4: 枚举所有已存在的 volume 文件，不只新写的。
    // 每个 revision 快照应包含完整的 memory 状态（current + index + 所有已封卷）。
    let mut volume_files: Vec<(String, Vec<u8>)> = Vec::new();
    let vd = volumes_dir(session_dir);
    if vd.is_dir() {
        if let Ok(entries) = fs::read_dir(&vd) {
            let mut vol_paths: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("vol_") && n.ends_with(".md"))
                })
                .map(|e| e.path())
                .collect();
            vol_paths.sort();
            for vp in vol_paths {
                let relative = vp
                    .strip_prefix(session_dir)
                    .unwrap_or(&vp)
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = fs::read(&vp)?;
                volume_files.push((relative, bytes));
            }
        }
    }
    files.extend(volume_files);
    // extra_files 是本次新写的文件（如新封卷），已包含在 volume_files 枚举中，
    // 但如果 extra_files 含非 volume 文件则仍需追加。
    for (path, content) in extra_files {
        if !files.iter().any(|(p, _)| p == &path) {
            files.push((path, content));
        }
    }
    let source_hash_hex = {
        let mut hasher = Sha256::new();
        hasher.update(&current_bytes);
        hasher.update(&index_bytes);
        for (_, content) in &files[2..] {
            hasher.update(content);
        }
        format!("{:x}", hasher.finalize())
    };
    let now = chrono::Utc::now().to_rfc3339();
    let staged = StagedRevision {
        content_revision,
        asset_kind: AssetKind::Memory,
        asset_id: session_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("memory")
            .to_string(),
        created_at: now.clone(),
        source: AssetSource {
            source_kind: source_kind.to_string(),
            source_hash: Some(source_hash_hex),
            source_filename: None,
            converter_version: None,
            imported_at: Some(now),
            parent_revision: if content_revision > 1 {
                Some(content_revision - 1)
            } else {
                None
            },
        },
        files,
    };
    let commit_opts = CommitOptions::new(session_dir);
    commit_revision(&staged, &commit_opts)?;
    Ok(content_revision)
}

/// 追加文本到 current.md。
///
/// M5.5：使用 `OpenOptions::append` 进行 O(1) 追加，避免 read-all-write-all
/// 模式下并发 finalizer 任务相互覆盖。多余的空行对 Markdown 渲染无害。
pub fn append_to_current(session_dir: &Path, text: &str) -> Result<(), AirpError> {
    ensure_session_dirs(session_dir)?;
    let cp = current_path(session_dir);

    // 已有非空内容时，先补一个换行作为段落分隔；省去 seek+read 末字节的开销。
    let needs_leading_newline = fs::metadata(&cp).map(|m| m.len() > 0).unwrap_or(false);

    let mut f = fs::OpenOptions::new().append(true).open(&cp)?;

    if needs_leading_newline {
        f.write_all(b"\n")?;
    }
    f.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    // Gemini #3: sync_data 确保追加内容落盘，与后续 revision commit 一致。
    f.sync_data()?;
    drop(f);
    // #115 Phase 2f：追加后 commit memory revision。
    commit_memory_revision(session_dir, Vec::new(), "derived")?;
    Ok(())
}

/// 读取 current.md 内容；若不存在返回空串。
pub fn read_current(session_dir: &Path) -> Result<String, AirpError> {
    let cp = current_path(session_dir);
    if !cp.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(&cp)?)
}

/// 估算 current.md 当前的 token 数。
pub fn count_tokens_current(session_dir: &Path) -> usize {
    match read_current(session_dir) {
        Ok(s) => estimate_tokens(&s),
        Err(_) => 0,
    }
}

/// 清空 current.md（保留文件但内容置空）。
pub fn clear_current(session_dir: &Path) -> Result<(), AirpError> {
    let cp = current_path(session_dir);
    Ok(fs::write(&cp, "")?)
}

/// 列出 volumes 目录下已存在的卷编号，按升序返回。
pub fn list_volume_numbers(session_dir: &Path) -> Vec<u32> {
    let vd = volumes_dir(session_dir);
    let mut result = Vec::new();
    if !vd.exists() {
        return result;
    }
    if let Ok(entries) = fs::read_dir(&vd) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                // 匹配 vol_XXX.md
                if let Some(rest) = name.strip_prefix("vol_") {
                    if let Some(num_str) = rest.strip_suffix(".md") {
                        if let Ok(n) = num_str.parse::<u32>() {
                            result.push(n);
                        }
                    }
                }
            }
        }
    }
    result.sort();
    result
}

/// 返回下一个可用的卷编号（已有最大值 + 1，初始为 1）。
///
/// R4: 用 `saturating_add` 防止 u32 溢出。若磁盘上出现 `vol_4294967295.md`
/// （u32::MAX），返回值会停在 u32::MAX 而不是回绕到 0；后续 `write_volume`
/// 会原地把 `vol_4294967295.md` 替换为最新内容，避免静默覆盖 `vol_000.md`。
/// 这与 `revision::atomic::next_content_revision` 的 `checked_add` 纪律一致。
pub fn next_volume_number(session_dir: &Path) -> u32 {
    list_volume_numbers(session_dir)
        .last()
        .copied()
        .map(|n| n.saturating_add(1))
        .unwrap_or(1)
}

/// 写入一卷的完整内容（含 [卷索引] 头部）。
pub fn write_volume(session_dir: &Path, number: u32, content: &str) -> Result<(), AirpError> {
    ensure_session_dirs(session_dir)?;
    let vp = volume_path(session_dir, number);
    fs::write(&vp, content)?;
    // #115 Phase 2f：封卷后 commit memory revision（含新卷文件）。
    let vol_relative = format!("volumes/vol_{:03}.md", number);
    commit_memory_revision(
        session_dir,
        vec![(vol_relative, content.as_bytes().to_vec())],
        "derived",
    )?;
    Ok(())
}

/// 读取某卷的完整内容。
pub fn read_volume_full(session_dir: &Path, number: u32) -> Result<String, AirpError> {
    let vp = volume_path(session_dir, number);
    if !vp.exists() {
        return Err(AirpError::NotFound(format!("卷 {} 不存在", number)));
    }
    Ok(fs::read_to_string(&vp)?)
}

/// 只读取某卷的 [卷索引] 头部（从文件开头到第一个 `---` 分隔线之前）。
/// 若无分隔线，则返回整个文件（兼容退化情况）。
pub fn read_volume_header(session_dir: &Path, number: u32) -> Result<String, AirpError> {
    let full = read_volume_full(session_dir, number)?;
    if let Some(idx) = full.find("\n---") {
        Ok(full[..idx].to_string())
    } else {
        Ok(full)
    }
}

/// 读取全局 index.md。
pub fn read_index(session_dir: &Path) -> Result<String, AirpError> {
    let ip = index_path(session_dir);
    if !ip.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(&ip)?)
}

/// 写入全局 index.md。
pub fn write_index(session_dir: &Path, content: &str) -> Result<(), AirpError> {
    ensure_session_dirs(session_dir)?;
    let ip = index_path(session_dir);
    Ok(fs::write(&ip, content)?)
}

/// 递增并持久化 turn_counter.txt 中的计数，用于触发周期性维护。
/// 返回递增后的新值。
///
/// **M0 F-23 / 0.10**：读取/解析失败时 tracing::warn 后归 0，而非完全静默。
/// 这种降级是有意的（计数损坏不应阻塞主对话流程），但应留有日志线索。
pub fn increment_turn_counter(session_dir: &Path) -> Result<u64, AirpError> {
    ensure_session_dirs(session_dir)?;
    let path = session_dir.join("turn_counter.txt");
    let current: u64 = if path.exists() {
        match fs::read_to_string(&path) {
            Ok(s) => match s.trim().parse::<u64>() {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(
                        path = ?path,
                        err = %e,
                        raw = %s.trim(),
                        "turn_counter 解析失败，重置为 0"
                    );
                    0
                }
            },
            Err(e) => {
                tracing::warn!(path = ?path, err = %e, "turn_counter 读取失败，重置为 0");
                0
            }
        }
    } else {
        0
    };
    let new_count = current + 1;
    fs::write(&path, new_count.to_string())?;
    Ok(new_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revision::atomic::read_current_revision;
    use tempfile::tempdir;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 11 / 4); // 2
                                                            // 中文字符按 1.5 tokens 估算
        let cjk = "你好世界"; // 4 chars
        assert_eq!(estimate_tokens(cjk), 4 * 3 / 2); // 6
    }

    #[test]
    fn test_estimate_tokens_documented_bounds() {
        // M0 F-22 / 6.0f：明确文档化的误差范围。
        // 空串返回 0
        assert_eq!(estimate_tokens(""), 0);
        // 纯 ASCII 长文本：1000 chars → 250 tokens
        let ascii = "a".repeat(1000);
        assert_eq!(estimate_tokens(&ascii), 250);
        // 纯 CJK：100 chars → 150 tokens
        let cjk = "字".repeat(100);
        assert_eq!(estimate_tokens(&cjk), 150);
        // 混合：100 ASCII + 100 CJK = 25 + 150 = 175
        let mixed = "a".repeat(100) + &"字".repeat(100);
        assert_eq!(estimate_tokens(&mixed), 175);
    }

    #[test]
    fn test_ensure_session_dirs() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("session1");

        ensure_session_dirs(&session_dir).unwrap();

        assert!(session_dir.exists());
        assert!(session_dir.join("volumes").exists());
        assert!(session_dir.join("current.md").exists());
        assert!(session_dir.join("index.md").exists());

        // index.md 应当被初始化为带分类骨架
        let idx = read_index(&session_dir).unwrap();
        assert!(idx.contains("## 人物"));
        assert!(idx.contains("## 悬挂线索"));
    }

    #[test]
    fn test_append_and_read_current() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("session1");

        ensure_session_dirs(&session_dir).unwrap();
        append_to_current(&session_dir, "第一段剧情").unwrap();
        append_to_current(&session_dir, "第二段剧情").unwrap();

        let content = read_current(&session_dir).unwrap();
        assert!(content.contains("第一段剧情"));
        assert!(content.contains("第二段剧情"));

        let tokens = count_tokens_current(&session_dir);
        assert!(tokens > 0);
    }

    #[test]
    fn test_clear_current() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("session1");
        ensure_session_dirs(&session_dir).unwrap();
        append_to_current(&session_dir, "测试内容").unwrap();
        clear_current(&session_dir).unwrap();
        let content = read_current(&session_dir).unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn test_volume_write_read_and_header() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("session1");
        ensure_session_dirs(&session_dir).unwrap();

        let vol_content = "# 卷1：开端\n\n## [卷索引]\n- 登场: 玩家, 艾莉娅\n- 事件: 初次相遇\n\n---\n\n这是完整叙事的正文部分。\n";
        write_volume(&session_dir, 1, vol_content).unwrap();

        let full = read_volume_full(&session_dir, 1).unwrap();
        assert_eq!(full, vol_content);

        let header = read_volume_header(&session_dir, 1).unwrap();
        assert!(header.contains("[卷索引]"));
        assert!(!header.contains("完整叙事"));
    }

    #[test]
    fn test_next_volume_number() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("session1");
        ensure_session_dirs(&session_dir).unwrap();

        assert_eq!(next_volume_number(&session_dir), 1);
        write_volume(&session_dir, 1, "vol1").unwrap();
        assert_eq!(next_volume_number(&session_dir), 2);
        write_volume(&session_dir, 2, "vol2").unwrap();
        assert_eq!(next_volume_number(&session_dir), 3);

        let nums = list_volume_numbers(&session_dir);
        assert_eq!(nums, vec![1, 2]);
    }

    #[test]
    fn test_next_volume_number_saturates_at_u32_max() {
        // R4: 旧实现用 `n + 1`，u32::MAX 时 debug 构建会 panic、release 构建
        // 会回绕到 0，导致 `write_volume(session_dir, 0, ...)` 静默覆盖
        // `vol_000.md`。saturating_add 保证停在 u32::MAX。
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("saturate");
        ensure_session_dirs(&session_dir).unwrap();

        // 直接构造 u32::MAX 编号的卷文件，绕过 write_volume 的正常路径。
        let max_path = session_dir.join("volumes").join("vol_4294967295.md");
        std::fs::write(&max_path, "max").unwrap();

        assert_eq!(
            next_volume_number(&session_dir),
            u32::MAX,
            "saturating_add must not wrap to 0"
        );
    }

    #[test]
    fn test_increment_turn_counter() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("s1");
        ensure_session_dirs(&session_dir).unwrap();

        assert_eq!(increment_turn_counter(&session_dir).unwrap(), 1);
        assert_eq!(increment_turn_counter(&session_dir).unwrap(), 2);
        assert_eq!(increment_turn_counter(&session_dir).unwrap(), 3);
    }

    #[test]
    fn test_index_write_read() {
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("session1");
        ensure_session_dirs(&session_dir).unwrap();

        let custom_index = "## 人物\n- 艾莉娅: 卷2\n";
        write_index(&session_dir, custom_index).unwrap();
        let result = read_index(&session_dir).unwrap();
        assert_eq!(result, custom_index);
    }

    #[test]
    fn append_to_current_creates_and_bumps_memory_revision() {
        // Phase 2f：append_to_current 后应创建 revision 目录 + current_revision 指针
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("mem-rev");
        ensure_session_dirs(&session_dir).unwrap();

        append_to_current(&session_dir, "第一段").unwrap();
        assert_eq!(
            read_current_revision(&session_dir).unwrap(),
            Some(1),
            "首次 append 应创建 revision 1"
        );
        assert!(session_dir.join("revisions").join("1").is_dir());
        assert!(session_dir
            .join("revisions")
            .join("1")
            .join("current.md")
            .is_file());
        assert!(session_dir
            .join("revisions")
            .join("1")
            .join("index.md")
            .is_file());

        append_to_current(&session_dir, "第二段").unwrap();
        assert_eq!(
            read_current_revision(&session_dir).unwrap(),
            Some(2),
            "第二次 append 应 bump 到 revision 2"
        );
        assert!(session_dir.join("revisions").join("2").is_dir());
        // 旧 revision 保留不可变
        assert!(session_dir.join("revisions").join("1").is_dir());
    }

    #[test]
    fn write_volume_creates_revision_with_volume_file() {
        // Phase 2f：封卷后 revision 应包含 volumes/vol_NNN.md
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("mem-vol");
        ensure_session_dirs(&session_dir).unwrap();

        write_volume(&session_dir, 1, "# 卷1\n内容").unwrap();
        assert_eq!(
            read_current_revision(&session_dir).unwrap(),
            Some(1),
            "封卷应创建 revision 1"
        );
        let rev_dir = session_dir.join("revisions").join("1");
        assert!(rev_dir.is_dir());
        assert!(rev_dir.join("current.md").is_file());
        assert!(rev_dir.join("index.md").is_file());
        assert!(
            rev_dir.join("volumes").join("vol_001.md").is_file(),
            "revision 应包含封卷文件"
        );
    }

    #[test]
    fn append_to_current_recovers_from_orphan_revision_dir() {
        // Memory orphan revision_dir 恢复测试。
        //
        // 模拟 commit_revision 第 5 步成功后崩溃（revision_dir 已 rename 但
        // current_revision 指针未更新）：预先创建 orphan `revisions/2/` 空目录，
        // 下次 `append_to_current` 应通过 `next_content_revision` 跳过 orphan，
        // 使用 revision 3 而非与 orphan 冲突的 revision 2。
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("mem-orphan");
        ensure_session_dirs(&session_dir).unwrap();

        // 第一次 append → revision 1
        append_to_current(&session_dir, "第一段").unwrap();
        assert_eq!(
            read_current_revision(&session_dir).unwrap(),
            Some(1),
            "首次 append 应创建 revision 1"
        );

        // 模拟 orphan：手动创建 revisions/2/ 空目录（current_revision 仍指向 1）
        std::fs::create_dir_all(session_dir.join("revisions").join("2")).unwrap();

        // 第二次 append 应跳过 orphan 2，使用 revision 3
        let result = append_to_current(&session_dir, "第二段");
        assert!(
            result.is_ok(),
            "append 应跳过 orphan revisions/2/ 并使用 revision 3，实际: {:?}",
            result.err()
        );
        assert_eq!(
            read_current_revision(&session_dir).unwrap(),
            Some(3),
            "current_revision 应为 3（跳过 orphan 2）"
        );
        assert!(
            session_dir.join("revisions").join("3").is_dir(),
            "revision 3 目录应存在"
        );
        // orphan 目录应保留（不可变快照原则）
        assert!(
            session_dir.join("revisions").join("2").is_dir(),
            "orphan revisions/2/ 应保留不删除"
        );
    }
}
