//! Soul-Drift 动态人格：Base + drift 双层 overlay。
//!
//! 存储路径：`data/characters/{id}/soul_drift.md`（每角色一份）
//! 格式：markdown 条目列表（`- ` 开头），与 resident memory 同构
//! 容量上限：~1500 字符（可配置）；超限触发 LLM 合并压缩

use crate::error::AirpError;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 默认容量上限（字符数）。
pub const SOUL_DRIFT_DEFAULT_CAP: usize = 1500;

/// 每角色串行化锁：防止并发 read-modify-write 互相覆盖（审计修复）。
static DRIFT_LOCKS: Lazy<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 获取角色的串行化锁。
fn drift_lock(character_id: &str) -> Arc<Mutex<()>> {
    let mut locks = DRIFT_LOCKS.lock().expect("drift locks poisoned");
    locks
        .entry(character_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Soul-Drift 配置。
#[derive(Debug, Clone)]
pub struct SoulDriftConfig {
    /// 容量上限（字符数）。超限触发压缩。
    pub capacity_chars: usize,
}

impl Default for SoulDriftConfig {
    fn default() -> Self {
        Self {
            capacity_chars: SOUL_DRIFT_DEFAULT_CAP,
        }
    }
}

/// 返回角色的 soul_drift.md 路径。
fn drift_path(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("soul_drift.md")
}

/// 读取 soul drift 内容。文件不存在返回空字符串。
pub fn read_soul_drift(data_root: &Path, character_id: &str) -> Result<String, AirpError> {
    let path = drift_path(data_root, character_id);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 写入 soul drift（覆盖）。
///
/// 审计修复：写入前强制容量上限，超限截断到最近完整行，防止超量内容被
/// 整体注入后续 system prompt。
pub fn write_soul_drift(
    data_root: &Path,
    character_id: &str,
    content: &str,
) -> Result<(), AirpError> {
    let config = SoulDriftConfig::default();
    let content = enforce_capacity(content, config.capacity_chars);
    let path = drift_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

/// 截断到容量上限，尽量保留完整行。
fn enforce_capacity(content: &str, capacity: usize) -> String {
    if content.chars().count() <= capacity {
        return content.to_string();
    }
    // 逐行累加，直到超过容量，保留之前的完整行。
    let mut result = String::new();
    let mut count = 0;
    for line in content.lines() {
        let line_len = line.chars().count() + 1; // +1 for newline
        if count + line_len > capacity {
            break;
        }
        result.push_str(line);
        result.push('\n');
        count += line_len;
    }
    result
}

/// 追加内容到 soul drift。
///
/// 审计修复：整个 read-modify-write 过程持有每角色锁，防止并发丢失更新。
pub fn append_soul_drift(
    data_root: &Path,
    character_id: &str,
    content: &str,
) -> Result<(), AirpError> {
    let _guard = drift_lock(character_id);
    let mut existing = read_soul_drift(data_root, character_id)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(content);
    write_soul_drift(data_root, character_id, &existing)
}

/// 把 soul_drift.md 注入到 System Prompt 的 `[Soul Drift]` 段。
///
/// 注入位置：card_details 之后、lorebook 之前。
/// Frozen snapshot 语义：本轮写入，下轮 prepare 才注入。
pub fn inject_soul_drift(data_root: &Path, character_id: &str, prompt: &mut String) {
    let Ok(content) = read_soul_drift(data_root, character_id) else {
        return;
    };
    if content.trim().is_empty() {
        return;
    }
    prompt.push_str("\n[Soul Drift]\n");
    prompt.push_str(&content);
    if !content.ends_with('\n') {
        prompt.push('\n');
    }
}

/// 检查 soul drift 是否超过容量上限。
pub fn is_over_capacity(data_root: &Path, character_id: &str, config: &SoulDriftConfig) -> bool {
    let Ok(content) = read_soul_drift(data_root, character_id) else {
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
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_and_read() {
        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", "- 语气更温柔").unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.contains("语气更温柔"));
    }

    #[test]
    fn test_append() {
        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", "- 第一条").unwrap();
        append_soul_drift(tmp.path(), "hero", "- 第二条").unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.contains("第一条"));
        assert!(content.contains("第二条"));
    }

    #[test]
    fn test_inject_soul_drift() {
        let tmp = tempdir().unwrap();
        let mut prompt = String::from("Base prompt.");

        inject_soul_drift(tmp.path(), "hero", &mut prompt);
        assert_eq!(prompt, "Base prompt.");

        write_soul_drift(tmp.path(), "hero", "- 更活泼").unwrap();
        inject_soul_drift(tmp.path(), "hero", &mut prompt);
        assert!(prompt.contains("[Soul Drift]"));
        assert!(prompt.contains("更活泼"));
    }

    #[test]
    fn test_is_over_capacity() {
        let tmp = tempdir().unwrap();
        let config = SoulDriftConfig { capacity_chars: 10 };

        write_soul_drift(tmp.path(), "hero", "短").unwrap();
        assert!(!is_over_capacity(tmp.path(), "hero", &config));

        write_soul_drift(tmp.path(), "hero", "这是一段超过十个字符的漂移内容").unwrap();
        assert!(is_over_capacity(tmp.path(), "hero", &config));
    }
}
