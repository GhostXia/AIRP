//! 常驻有界记忆（Resident Memory）：每角色/每 session 一份有界 markdown。
//!
//! 存储路径：`session_dir/resident.md`（session_dir 已由 data_dir 解析为
//! `characters/{id}/memory/` 或 `characters/{id}/sessions/{sid}/memory/`）。
//!
//! 容量上限默认 ~2000 字符（可配置）；超限触发 `compress::compress_resident_memory`。

use crate::error::AirpError;
use std::fs;
use std::path::Path;

/// 默认容量上限（字符数）。
pub const RESIDENT_MEMORY_DEFAULT_CAP: usize = 2000;

///  resident memory 配置。
#[derive(Debug, Clone)]
pub struct ResidentMemoryConfig {
    /// 容量上限（字符数）。超限触发压缩。
    pub capacity_chars: usize,
    /// 是否启用自动抽取。
    pub auto_extract: bool,
}

impl Default for ResidentMemoryConfig {
    fn default() -> Self {
        Self {
            capacity_chars: RESIDENT_MEMORY_DEFAULT_CAP,
            auto_extract: true,
        }
    }
}

/// 返回 session 目录下 resident.md 的完整路径。
fn resident_path(session_dir: &Path) -> std::path::PathBuf {
    session_dir.join("resident.md")
}

/// 读取 resident memory 内容。文件不存在返回空字符串。
pub fn read_resident_memory(session_dir: &Path) -> Result<String, AirpError> {
    let path = resident_path(session_dir);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 写入 resident memory（覆盖）。
pub fn write_resident_memory(session_dir: &Path, content: &str) -> Result<(), AirpError> {
    let path = resident_path(session_dir);
    // 确保目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

/// 追加内容到 resident memory。
pub fn append_resident_memory(session_dir: &Path, content: &str) -> Result<(), AirpError> {
    let mut existing = read_resident_memory(session_dir)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(content);
    write_resident_memory(session_dir, &existing)
}

/// 把 resident.md 注入到 System Prompt 的 `[Resident Memory]` 段。
///
/// Frozen snapshot 语义：本轮抽取落盘，下轮 prepare 才注入（自然满足，
/// 因为 prepare 在 finalize 之前执行）。
pub fn inject_resident_memory(session_dir: &Path, prompt: &mut String) {
    let Ok(content) = read_resident_memory(session_dir) else {
        return;
    };
    if content.trim().is_empty() {
        return;
    }
    prompt.push_str("\n[Resident Memory]\n");
    prompt.push_str(&content);
    if !content.ends_with('\n') {
        prompt.push('\n');
    }
}

/// 检查 resident memory 是否超过容量上限。
pub fn is_over_capacity(session_dir: &Path, config: &ResidentMemoryConfig) -> bool {
    let Ok(content) = read_resident_memory(session_dir) else {
        return false;
    };
    content.chars().count() > config.capacity_chars
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_nonexistent_returns_empty() {
        let tmp = tempdir().unwrap();
        let content = read_resident_memory(tmp.path()).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_and_read() {
        let tmp = tempdir().unwrap();
        write_resident_memory(tmp.path(), "# 记忆\n\n- 用户喜欢猫").unwrap();
        let content = read_resident_memory(tmp.path()).unwrap();
        assert!(content.contains("用户喜欢猫"));
    }

    #[test]
    fn test_append() {
        let tmp = tempdir().unwrap();
        write_resident_memory(tmp.path(), "- 第一条").unwrap();
        append_resident_memory(tmp.path(), "- 第二条").unwrap();
        let content = read_resident_memory(tmp.path()).unwrap();
        assert!(content.contains("第一条"));
        assert!(content.contains("第二条"));
    }

    #[test]
    fn test_inject_resident_memory() {
        let tmp = tempdir().unwrap();
        let mut prompt = String::from("Base prompt.");

        // 空文件不注入
        inject_resident_memory(tmp.path(), &mut prompt);
        assert_eq!(prompt, "Base prompt.");

        // 有内容时注入
        write_resident_memory(tmp.path(), "用户偏好：简洁回复").unwrap();
        inject_resident_memory(tmp.path(), &mut prompt);
        assert!(prompt.contains("[Resident Memory]"));
        assert!(prompt.contains("用户偏好：简洁回复"));
    }

    #[test]
    fn test_is_over_capacity() {
        let tmp = tempdir().unwrap();
        let config = ResidentMemoryConfig {
            capacity_chars: 10,
            auto_extract: true,
        };

        write_resident_memory(tmp.path(), "短").unwrap();
        assert!(!is_over_capacity(tmp.path(), &config));

        write_resident_memory(tmp.path(), "这是一段超过十个字符的记忆内容").unwrap();
        assert!(is_over_capacity(tmp.path(), &config));
    }
}
