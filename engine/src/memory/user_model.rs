//! 用户模型手动编辑：每用户一份偏好模型（`user_model.md`）。
//!
//! 存储路径：`data/users/{uid}/user_model.md`
//!
//! 注意：PR #271 的 MVP 范围只暴露 HTTP 手动编辑 API（GET / PUT）。
//! 自动抽取 / 注入 System Prompt / 写入追加等能力未在 MVP 中接入，
//! 故本模块只保留读 / 写 / 路径解析三件事；相关死代码已剔除（审计 B3）。

use crate::error::AirpError;
use crate::types::UserId;
use std::fs;
use std::path::{Path, PathBuf};

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
}
