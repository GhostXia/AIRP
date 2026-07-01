use crate::error::AirpError;
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
pub fn next_volume_number(session_dir: &Path) -> u32 {
    list_volume_numbers(session_dir)
        .last()
        .copied()
        .map(|n| n + 1)
        .unwrap_or(1)
}

/// 写入一卷的完整内容（含 [卷索引] 头部）。
pub fn write_volume(session_dir: &Path, number: u32, content: &str) -> Result<(), AirpError> {
    ensure_session_dirs(session_dir)?;
    let vp = volume_path(session_dir, number);
    Ok(fs::write(&vp, content)?)
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
}
