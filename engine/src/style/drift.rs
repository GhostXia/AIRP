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
use std::sync::{Arc, Mutex, Weak};

/// 默认容量上限（字符数）。
pub const SOUL_DRIFT_DEFAULT_CAP: usize = 1500;

/// 每角色串行化锁：防止并发 read-modify-write 互相覆盖（审计修复）。
///
/// 审计再修复：用 Weak 引用持有锁，获取时清理已无持有者的 stale 条目，
/// 防止长生命周期进程中注册表无界增长。
static DRIFT_LOCKS: Lazy<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 获取角色的串行化锁。
fn drift_lock(character_id: &str) -> Arc<Mutex<()>> {
    let mut locks = DRIFT_LOCKS.lock().expect("drift locks poisoned");
    // 清理已无强引用的 stale 条目，保证注册表有界。
    locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(weak) = locks.get(character_id) {
        if let Some(strong) = weak.upgrade() {
            return strong;
        }
    }
    let strong = Arc::new(Mutex::new(()));
    locks.insert(character_id.to_string(), Arc::downgrade(&strong));
    strong
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
///
/// 审计再修复（CodeRabbit 22:26）：直接写入也持有每角色锁，与 append
/// 互斥，防止 write-vs-append 竞态。
pub fn write_soul_drift(
    data_root: &Path,
    character_id: &str,
    content: &str,
) -> Result<(), AirpError> {
    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    write_soul_drift_unlocked(data_root, character_id, content)
}

/// 内部写入实现（不加锁）。调用方必须已持有每角色锁。
fn write_soul_drift_unlocked(
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
///
/// 审计再修复：若首行单独就超过容量（逐行截断会得到空串），回退为按
/// Unicode 字符边界截断，保证不会把超量单行输入清空。
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
    // 首行单独超容量时 result 为空：回退为按字符边界截断，避免清空内容。
    if result.is_empty() {
        return content.chars().take(capacity).collect();
    }
    result
}

/// 追加内容到 soul drift。
///
/// 审计修复：整个 read-modify-write 过程持有每角色锁，防止并发丢失更新。
///
/// 审计再修复（CodeRabbit 22:26）：原实现 `let _guard = drift_lock(...)` 只持有
/// Arc 而未调用 `.lock()`，锁从未生效。现在真正获取 MutexGuard，且内部
/// 调用无锁版写入避免重入死锁。
pub fn append_soul_drift(
    data_root: &Path,
    character_id: &str,
    content: &str,
) -> Result<(), AirpError> {
    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    let mut existing = read_soul_drift(data_root, character_id)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(content);
    write_soul_drift_unlocked(data_root, character_id, &existing)
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

/// #290 F-3：Soul-Drift 超容量时调用 LLM 合并压缩。
///
/// 复用 `memory::compress_resident_memory` 的压缩 prompt。压缩结果必须真的
/// 变小才落盘，否则保留原内容（enforce_capacity 已在写入时截断兜底）。
pub async fn compress_soul_drift_if_needed(
    client: &reqwest::Client,
    provider_config: Arc<crate::adapter::ProviderConfig>,
    gen_params: crate::adapter::GenerationParams,
    data_root: &Path,
    character_id: &str,
) -> Result<bool, AirpError> {
    let config = SoulDriftConfig::default();
    let content = read_soul_drift(data_root, character_id)?;
    if content.chars().count() <= config.capacity_chars {
        return Ok(false);
    }
    let compressed = crate::memory::compress_resident_memory(
        client,
        provider_config,
        gen_params,
        &content,
        config.capacity_chars,
    )
    .await?;
    if !compressed.is_empty() && compressed.chars().count() < content.chars().count() {
        write_soul_drift(data_root, character_id, &compressed)?;
        Ok(true)
    } else {
        Ok(false)
    }
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
    fn test_write_enforces_capacity() {
        let tmp = tempdir().unwrap();
        // 默认容量 1500，写入超长内容应被截断。
        let long_content = "- 条目\n".repeat(1000);
        write_soul_drift(tmp.path(), "hero", &long_content).unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.chars().count() <= SOUL_DRIFT_DEFAULT_CAP);
    }

    #[test]
    fn test_write_oversize_single_line_not_emptied() {
        let tmp = tempdir().unwrap();
        // 单行超过容量：不应被清空，应按字符边界截断。
        let single_line = "长".repeat(SOUL_DRIFT_DEFAULT_CAP + 500);
        write_soul_drift(tmp.path(), "hero", &single_line).unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(!content.is_empty());
        assert_eq!(content.chars().count(), SOUL_DRIFT_DEFAULT_CAP);
    }
}
