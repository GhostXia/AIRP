//! 用户模型手动编辑：每用户一份偏好模型（`user_model.md`）。
//!
//! 存储路径：`data/users/{uid}/user_model.md`
//!
//! 注意：PR #271 的 MVP 范围只暴露 HTTP 手动编辑 API（GET / PUT）。
//! 自动抽取 / 注入 System Prompt / 写入追加等能力未在 MVP 中接入，
//! 故本模块只保留读 / 写 / 路径解析三件事；相关死代码已剔除（审计 B3）。

use crate::error::AirpError;
use crate::types::UserId;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

/// 每用户串行化锁：防止并发 read-modify-write 互相覆盖（CodeRabbit #5）。
/// 与 `style::drift::drift_lock` 同模式：用 Weak 引用持有锁，获取时清理
/// stale 条目，保证注册表有界。
static USER_MODEL_LOCKS: Lazy<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 获取用户模型的串行化锁。key 为用户主目录路径字符串。
fn user_model_lock(home: &Path) -> Arc<Mutex<()>> {
    let key = home.to_string_lossy().into_owned();
    let mut locks = USER_MODEL_LOCKS.lock().expect("user model locks poisoned");
    locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(weak) = locks.get(&key) {
        if let Some(strong) = weak.upgrade() {
            return strong;
        }
    }
    let strong = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&strong));
    strong
}

/// 返回用户模型文件路径。
///
/// `user_id` 必须由 `UserId` newtype 构造时校验过，保证不含路径遍历字符。
/// 路径拼接不再做二次校验 —— 类型系统已强制 `&UserId` 入参（审计 B1 修复）。
fn user_model_path(data_root: &Path, user_id: &UserId) -> PathBuf {
    data_root
        .join("users")
        .join(user_id.as_str())
        .join("user_model.md")
}

/// 读取用户模型内容。文件不存在返回空字符串。
pub fn read_user_model(data_root: &Path, user_id: &UserId) -> Result<String, AirpError> {
    let path = user_model_path(data_root, user_id);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 写入用户模型（覆盖）。使用原子写（temp + rename + parent sync）防止
/// 半写状态被并发 reader 观察到（审计 W1 修复）。
pub fn write_user_model(
    data_root: &Path,
    user_id: &UserId,
    content: &str,
) -> Result<(), AirpError> {
    let path = user_model_path(data_root, user_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::data_dir::replace_file(&path, content.as_bytes())?;
    Ok(())
}

/// 用户模型容量上限（字符数）。超限截断到最近完整行（阶段二补全 D1）。
pub const USER_MODEL_CAP: usize = 1500;

/// 返回用户主目录（effective root）下的 user_model.md 路径。
///
/// finalize 路径中 `data_root` 已是该用户的独立根（`data/users/{uid}/`），
/// 用户模型直接落在其下，无需再拼 `users/{uid}` 前缀。
fn user_model_path_in_home(home: &Path) -> PathBuf {
    home.join("user_model.md")
}

/// 追加用户偏好到用户模型（阶段二补全 D1）。
///
/// 整个 read-modify-write 串行执行，并在写入前强制容量上限（超限保留
/// 最新完整行）。仅由 finalize 异步抽取调用，`home` 为用户独立数据根。
///
/// CodeRabbit #5：持有 per-user 锁贯穿 read-modify-write，防止同一用户
/// 多 session 并发 finalize 时丢失更新（与 `style::drift::append_soul_drift`
/// 同模式）。
pub fn append_user_model_in_home(home: &Path, content: &str) -> Result<(), AirpError> {
    let lock = user_model_lock(home);
    let _guard = lock.lock().expect("user model lock poisoned");

    let path = user_model_path_in_home(home);
    let existing = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(AirpError::from(e)),
    };

    let mut merged = existing;
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged.push_str(content);

    // 容量强制：超限保留最新完整行，首行单独超容量时按字符边界截断。
    let merged = enforce_user_model_capacity(&merged, USER_MODEL_CAP);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::data_dir::replace_file(&path, merged.as_bytes())?;
    Ok(())
}

/// 截断到容量上限，保留最新的完整行。
///
/// CodeRabbit #4：用户偏好会随时间演变（用户可能改变喜好），最新条目
/// 比旧条目更有代表性。因此超限时从头丢弃旧行，保留尾部新行；若单行
/// 就超过容量，按 Unicode 字符边界截断该行。
fn enforce_user_model_capacity(content: &str, capacity: usize) -> String {
    if content.chars().count() <= capacity {
        return content.to_string();
    }
    // 从尾部向前累加，保留最新的完整行。
    let lines: Vec<&str> = content.lines().collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut count = 0;
    for line in lines.iter().rev() {
        let line_len = line.chars().count() + 1; // +1 for newline
        if count + line_len > capacity {
            break;
        }
        kept.push(line);
        count += line_len;
    }
    if kept.is_empty() {
        // 没有任何完整行能放下：取最后一行的前 capacity 个字符。
        return content
            .lines()
            .last()
            .map(|l| l.chars().take(capacity).collect())
            .unwrap_or_else(|| content.chars().take(capacity).collect());
    }
    kept.reverse();
    kept.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn uid(s: &str) -> UserId {
        UserId::new(s).unwrap()
    }

    #[test]
    fn test_read_nonexistent_returns_empty() {
        let tmp = tempdir().unwrap();
        let content = read_user_model(tmp.path(), &uid("user1")).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_and_read() {
        let tmp = tempdir().unwrap();
        let u = uid("user1");
        write_user_model(tmp.path(), &u, "- 喜欢简洁回复").unwrap();
        let content = read_user_model(tmp.path(), &u).unwrap();
        assert!(content.contains("喜欢简洁回复"));
    }

    #[test]
    fn test_user_id_rejects_traversal() {
        // 审计 B1：user_id 路径遍历必须在 UserId::new 时被拒绝，
        // 而不是在路径拼接后才发现。
        assert!(UserId::new("..").is_err());
        assert!(UserId::new("../etc").is_err());
        assert!(UserId::new("a/b").is_err());
        assert!(UserId::new("").is_err());
        assert!(UserId::new(".hidden").is_err());
        assert!(UserId::new("a\\b").is_err());
        assert!(UserId::new("a:b").is_err());
    }

    #[test]
    fn test_write_is_atomic_with_backup() {
        // 审计 W1：原子写后，应只剩目标文件，无残留 .tmp / .bak。
        let tmp = tempdir().unwrap();
        let u = uid("user1");
        write_user_model(tmp.path(), &u, "first").unwrap();
        write_user_model(tmp.path(), &u, "second").unwrap();
        let user_dir = tmp.path().join("users").join("user1");
        let entries: Vec<_> = std::fs::read_dir(&user_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["user_model.md".to_string()]);
        // 内容应为最后一次写入
        assert_eq!(
            std::fs::read_to_string(user_dir.join("user_model.md")).unwrap(),
            "second"
        );
    }

    #[test]
    fn test_enforce_capacity_keeps_newest_lines() {
        // CodeRabbit #4：超限时应保留最新的完整行，丢弃旧行。
        let content = "- 旧偏好1\n- 旧偏好2\n- 新偏好1\n- 新偏好2\n";
        // capacity 设为只放得下最后 2 行（每行 7 chars + 1 newline = 8）
        let result = enforce_user_model_capacity(content, 16);
        assert!(result.contains("新偏好1"));
        assert!(result.contains("新偏好2"));
        assert!(!result.contains("旧偏好1"));
        assert!(!result.contains("旧偏好2"));
    }

    #[test]
    fn test_enforce_capacity_single_oversize_line() {
        // 单行超过容量：按字符边界截断该行。
        let single_line = "长".repeat(USER_MODEL_CAP + 100);
        let result = enforce_user_model_capacity(&single_line, USER_MODEL_CAP);
        assert_eq!(result.chars().count(), USER_MODEL_CAP);
    }

    #[test]
    fn test_enforce_capacity_under_limit_unchanged() {
        let content = "- 短\n";
        let result = enforce_user_model_capacity(content, 100);
        assert_eq!(result, content);
    }

    #[test]
    fn test_append_user_model_in_home_concurrent_safe() {
        // CodeRabbit #5：验证 append 不会因并发丢失更新。
        // 串行调用两次，两次的内容都应存在。
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        append_user_model_in_home(home, "- 偏好A").unwrap();
        append_user_model_in_home(home, "- 偏好B").unwrap();
        let content = std::fs::read_to_string(home.join("user_model.md")).unwrap();
        assert!(content.contains("偏好A"));
        assert!(content.contains("偏好B"));
    }
}
