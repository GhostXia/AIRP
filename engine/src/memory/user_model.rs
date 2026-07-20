//! 用户模型自动学习：每用户一份偏好模型（`user_model.md`）。
//!
//! 存储路径：`data/users/{uid}/user_model.md`
//! 从对话中抽取用户偏好信号（"我喜欢/不喜欢"、纠正、风格反馈）。

use crate::error::AirpError;
use std::fs;
use std::path::{Path, PathBuf};

/// 返回用户模型文件路径。
fn user_model_path(data_root: &Path, user_id: &str) -> PathBuf {
    data_root
        .join("users")
        .join(user_id)
        .join("user_model.md")
}

/// 读取用户模型内容。文件不存在返回空字符串。
pub fn read_user_model(data_root: &Path, user_id: &str) -> Result<String, AirpError> {
    let path = user_model_path(data_root, user_id);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 写入用户模型（覆盖）。
pub fn write_user_model(
    data_root: &Path,
    user_id: &str,
    content: &str,
) -> Result<(), AirpError> {
    let path = user_model_path(data_root, user_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

/// 追加内容到用户模型。
pub fn append_user_model(
    data_root: &Path,
    user_id: &str,
    content: &str,
) -> Result<(), AirpError> {
    let mut existing = read_user_model(data_root, user_id)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(content);
    write_user_model(data_root, user_id, &existing)
}

/// 把用户模型注入到 System Prompt 的 `[User Preferences]` 段。
pub fn inject_user_model(data_root: &Path, user_id: &str, prompt: &mut String) {
    let Ok(content) = read_user_model(data_root, user_id) else {
        return;
    };
    if content.trim().is_empty() {
        return;
    }
    prompt.push_str("\n[User Preferences]\n");
    prompt.push_str(&content);
    if !content.ends_with('\n') {
        prompt.push('\n');
    }
}

/// 用户偏好抽取 prompt 模板。
pub const USER_PREFERENCE_EXTRACTION_PROMPT: &str = r#"你是一个用户偏好抽取助手。从对话中抽取用户的写作偏好和习惯。

抽取规则：
1. 只抽取持久性偏好（文风喜好、雷点、习惯用语、纠正反馈）
2. 忽略临时性内容（具体剧情讨论、角色扮演内容）
3. 用简洁的条目格式输出，每条一行，以 "- " 开头
4. 如果没有值得记录的偏好，输出空字符串

输出格式示例：
- 用户喜欢第三人称叙事
- 用户不喜欢过多的心理描写
- 用户偏好简洁的对话风格
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_nonexistent_returns_empty() {
        let tmp = tempdir().unwrap();
        let content = read_user_model(tmp.path(), "user1").unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_and_read() {
        let tmp = tempdir().unwrap();
        write_user_model(tmp.path(), "user1", "- 喜欢简洁回复").unwrap();
        let content = read_user_model(tmp.path(), "user1").unwrap();
        assert!(content.contains("喜欢简洁回复"));
    }

    #[test]
    fn test_append() {
        let tmp = tempdir().unwrap();
        write_user_model(tmp.path(), "user1", "- 第一条").unwrap();
        append_user_model(tmp.path(), "user1", "- 第二条").unwrap();
        let content = read_user_model(tmp.path(), "user1").unwrap();
        assert!(content.contains("第一条"));
        assert!(content.contains("第二条"));
    }

    #[test]
    fn test_inject_user_model() {
        let tmp = tempdir().unwrap();
        let mut prompt = String::from("Base prompt.");

        // 空文件不注入
        inject_user_model(tmp.path(), "user1", &mut prompt);
        assert_eq!(prompt, "Base prompt.");

        // 有内容时注入
        write_user_model(tmp.path(), "user1", "- 偏好：简洁").unwrap();
        inject_user_model(tmp.path(), "user1", &mut prompt);
        assert!(prompt.contains("[User Preferences]"));
        assert!(prompt.contains("偏好：简洁"));
    }
}
