use crate::error::AirpError;
use std::fs;
use std::path::{Path, PathBuf};

pub fn safe_resolve_under_data_root(
    data_root: &Path,
    user_path: &str,
) -> Result<PathBuf, AirpError> {
    let trimmed = user_path.trim();
    if trimmed.is_empty() {
        return Err(AirpError::BadRequest("路径为空".to_string()));
    }
    let lower = trimmed.to_ascii_lowercase();
    let looks_absolute = trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || (lower.len() >= 2 && lower.as_bytes()[1] == b':');
    if looks_absolute {
        return Err(AirpError::BadRequest(format!(
            "拒绝绝对路径: {}",
            user_path
        )));
    }
    if trimmed.contains('\0') {
        return Err(AirpError::BadRequest("路径包含空字节".to_string()));
    }

    let candidate = data_root.join(trimmed);
    let canon_root = fs::canonicalize(data_root)?;
    let canon_candidate = fs::canonicalize(&candidate)?;
    if !canon_candidate.starts_with(&canon_root) {
        return Err(AirpError::PathEscape(canon_candidate));
    }
    Ok(canon_candidate)
}

/// 写路径安全解析：允许目标文件不存在。
///
/// 与 [`safe_resolve_under_data_root`] 的区别：
/// - `data_root`（基目录）必须存在（做 canonicalize 锚点）。
/// - 目标文件/目录**可以不存在**（通过组件级展开替代 canonicalize）。
/// - 仍拒绝绝对路径、`..` 穿越、空字节。
///
/// 用于 `write_preset_artifact` / `write_character_artifact` 等写新文件场景。
pub fn safe_resolve_for_write(base_dir: &Path, user_path: &str) -> Result<PathBuf, AirpError> {
    let trimmed = user_path.trim();
    if trimmed.is_empty() {
        return Err(AirpError::BadRequest("路径为空".to_string()));
    }
    let lower = trimmed.to_ascii_lowercase();
    let looks_absolute = trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || (lower.len() >= 2 && lower.as_bytes()[1] == b':');
    if looks_absolute {
        return Err(AirpError::BadRequest(format!(
            "拒绝绝对路径: {}",
            user_path
        )));
    }
    if trimmed.contains('\0') {
        return Err(AirpError::BadRequest("路径包含空字节".to_string()));
    }

    // 仅对基目录做 canonicalize（基目录必须存在）
    let canon_base = fs::canonicalize(base_dir)?;

    // 组件级展开：逐段处理 user_path，`..` 弹栈；超出根则拒绝
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    for comp in Path::new(trimmed).components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if stack.pop().is_none() {
                    return Err(AirpError::PathEscape(canon_base.join(trimmed)));
                }
            }
            std::path::Component::Normal(s) => stack.push(s.to_owned()),
            _ => {
                return Err(AirpError::BadRequest(format!(
                    "非法路径组件: {}",
                    user_path
                )))
            }
        }
    }
    if stack.is_empty() {
        return Err(AirpError::BadRequest("路径解析为空".to_string()));
    }
    let resolved = stack.iter().fold(canon_base.clone(), |acc, c| acc.join(c));
    // 双重保险：即使组件展开有漏洞，starts_with 仍阻挡穿越
    if !resolved.starts_with(&canon_base) {
        return Err(AirpError::PathEscape(resolved));
    }
    Ok(resolved)
}

pub fn validate_id_segment(id: &str) -> Result<(), AirpError> {
    if id.is_empty() {
        return Err(AirpError::BadRequest("ID 为空".to_string()));
    }
    if id == "." || id == ".." {
        return Err(AirpError::BadRequest(format!("非法 ID: {}", id)));
    }
    if id.starts_with('.') {
        return Err(AirpError::BadRequest(format!("ID 不允许以点开头: {}", id)));
    }
    for c in id.chars() {
        match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => {
                return Err(AirpError::BadRequest(format!(
                    "ID 含非法字符 {:?}: {}",
                    c, id
                )));
            }
            _ => {}
        }
    }
    if id.contains("..") {
        return Err(AirpError::BadRequest(format!("ID 含 ..: {}", id)));
    }
    Ok(())
}

// AUDIT-4: property tests for the path-traversal guards. These functions
// gate every filesystem write performed by AIRP — anything getting through
// here means user-controlled data could escape `data/`. Property-based tests
// exercise the input space more aggressively than fixed examples.
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tempfile::tempdir;

    // ── validate_id_segment property tests ─────────────────────────────

    proptest! {
        /// AUDIT-4: any string containing a path separator must be rejected
        /// regardless of where the separator appears or what else is in the
        /// string.
        #[test]
        fn prop_validate_id_rejects_path_separators(
            prefix in "[a-zA-Z0-9_-]{0,16}",
            sep in r"[/\\]",
            suffix in "[a-zA-Z0-9_-]{0,16}",
        ) {
            let id = format!("{}{}{}", prefix, sep, suffix);
            prop_assert!(validate_id_segment(&id).is_err(),
                "ID {:?} containing separator must be rejected", id);
        }

        /// AUDIT-4: any string containing a null byte must be rejected.
        #[test]
        fn prop_validate_id_rejects_null_byte(
            prefix in "[a-zA-Z0-9_]{0,8}",
            suffix in "[a-zA-Z0-9_]{0,8}",
        ) {
            let id = format!("{}\0{}", prefix, suffix);
            prop_assert!(validate_id_segment(&id).is_err());
        }

        /// AUDIT-4: any string containing ".." substring must be rejected.
        #[test]
        fn prop_validate_id_rejects_double_dot(
            prefix in "[a-zA-Z0-9_]{0,8}",
            suffix in "[a-zA-Z0-9_]{0,8}",
        ) {
            let id = format!("{}..{}", prefix, suffix);
            prop_assert!(validate_id_segment(&id).is_err());
        }

        /// AUDIT-4: any string with a Windows-reserved filesystem character
        /// must be rejected. These can corrupt path interpretation on Windows
        /// even if validation isn't strictly required on POSIX.
        #[test]
        fn prop_validate_id_rejects_windows_reserved(
            prefix in "[a-zA-Z0-9_]{0,8}",
            bad in r#"[:*?"<>|]"#,
            suffix in "[a-zA-Z0-9_]{0,8}",
        ) {
            let id = format!("{}{}{}", prefix, bad, suffix);
            prop_assert!(validate_id_segment(&id).is_err());
        }

        /// AUDIT-4: clean ASCII / Unicode names without separators or special
        /// characters should pass validation.
        #[test]
        fn prop_validate_id_accepts_clean_names(
            id in r"[a-zA-Z0-9_一-鿿][a-zA-Z0-9_一-鿿]{0,30}",
        ) {
            prop_assert!(validate_id_segment(&id).is_ok(),
                "clean ID {:?} should be accepted", id);
        }
    }

    // ── safe_resolve_for_write property tests ──────────────────────────

    proptest! {
        /// AUDIT-4: safe_resolve_for_write must never produce a path that
        /// escapes the base directory, regardless of how many `..` segments
        /// the user supplies. Either it returns a path under base, or it
        /// errors — never returns Ok with an escaping path.
        #[test]
        fn prop_safe_resolve_for_write_never_escapes(
            dots in 1usize..16,
            tail in "[a-zA-Z0-9_]{1,16}",
        ) {
            let base = tempdir().unwrap();
            let user_path: String = std::iter::repeat_n("..", dots)
                .collect::<Vec<_>>()
                .join("/")
                + "/"
                + &tail;
            let result = safe_resolve_for_write(base.path(), &user_path);
            match result {
                Err(_) => {} // expected outcome — traversal detected
                Ok(resolved) => {
                    let canon_base = std::fs::canonicalize(base.path()).unwrap();
                    prop_assert!(
                        resolved.starts_with(&canon_base),
                        "safe_resolve returned escaping path: {:?} not under {:?}",
                        resolved, canon_base
                    );
                }
            }
        }

        /// AUDIT-4: absolute paths (POSIX `/`, Windows backslash, Windows
        /// drive letters) must always be rejected.
        #[test]
        fn prop_safe_resolve_rejects_absolute(
            tail in "[a-zA-Z0-9_/]{1,32}",
        ) {
            let base = tempdir().unwrap();
            let posix = format!("/{}", tail);
            let win_bs = format!("\\{}", tail);
            let win_drive = format!("C:/{}", tail);
            prop_assert!(safe_resolve_for_write(base.path(), &posix).is_err());
            prop_assert!(safe_resolve_for_write(base.path(), &win_bs).is_err());
            prop_assert!(safe_resolve_for_write(base.path(), &win_drive).is_err());
        }

        /// AUDIT-4: paths with null bytes must always be rejected.
        #[test]
        fn prop_safe_resolve_rejects_null_byte(
            prefix in "[a-zA-Z0-9_/]{0,16}",
            suffix in "[a-zA-Z0-9_/]{0,16}",
        ) {
            let base = tempdir().unwrap();
            let path = format!("{}\0{}", prefix, suffix);
            prop_assert!(safe_resolve_for_write(base.path(), &path).is_err());
        }
    }

    // ── targeted edge cases that fuzzing might miss ────────────────────

    #[test]
    fn audit_4_unicode_homoglyph_passes_validation() {
        // U+2215 DIVISION SLASH looks like '/' but isn't ASCII '/' — verify
        // we don't accidentally allow it via the homoglyph route. Either
        // accept it (it's literally a Unicode letter to the FS) or reject.
        // Current implementation accepts; document this is intentional.
        let id = "alice\u{2215}bob";
        // We accept this — the FS treats it as a normal char, not a separator.
        // This test exists to detect behavioral changes.
        assert!(validate_id_segment(id).is_ok());
    }

    #[test]
    fn audit_4_empty_after_trim_rejected() {
        let base = tempdir().unwrap();
        assert!(safe_resolve_for_write(base.path(), "").is_err());
        assert!(safe_resolve_for_write(base.path(), "   ").is_err());
    }

    #[test]
    fn audit_4_single_dot_components_resolved_under_base() {
        let base = tempdir().unwrap();
        let r = safe_resolve_for_write(base.path(), "./a/./b/./c").unwrap();
        let canon = std::fs::canonicalize(base.path()).unwrap();
        assert!(r.starts_with(&canon));
        assert!(r.ends_with("c"));
    }

    #[test]
    fn audit_4_balanced_parent_dirs_stay_under_base() {
        let base = tempdir().unwrap();
        // "a/b/../c" should resolve to base/a/c, NOT escape
        let r = safe_resolve_for_write(base.path(), "a/b/../c").unwrap();
        let canon = std::fs::canonicalize(base.path()).unwrap();
        assert!(r.starts_with(&canon));
        assert!(r.ends_with("c"));
    }
}
