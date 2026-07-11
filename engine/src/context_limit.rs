//! Byte-bounded text returned to model context.

use std::sync::OnceLock;

const DEFAULT_MAX_READ_BYTES: usize = 32 * 1024;
const MIN_MAX_READ_BYTES: usize = 1024;

pub(crate) fn max_read_bytes() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("AIRP_MAX_READ_BYTES")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value >= MIN_MAX_READ_BYTES)
            .unwrap_or(DEFAULT_MAX_READ_BYTES)
    })
}

/// Truncate model-facing text without splitting a UTF-8 code point.
pub(crate) fn truncate_for_context(content: &str) -> String {
    truncate_with_notice(
        content,
        "content exceeds the single-read cap; use /v1/characters/:id and /v1/characters/:id/analysis for structured access",
    )
}

pub(crate) fn truncate_with_notice(content: &str, notice: &str) -> String {
    let max_len = max_read_bytes();
    if content.len() <= max_len {
        return content.to_string();
    }
    let mut end = max_len;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "[PARTIAL: total={}, limit={} — {}]\n{}",
        content.len(),
        end,
        notice,
        &content[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_cap_is_unchanged() {
        assert_eq!(truncate_for_context("short content"), "short content");
    }

    #[test]
    fn over_cap_has_partial_marker_and_airp_hint() {
        let content = "x".repeat(max_read_bytes() + 100);
        let result = truncate_for_context(&content);
        assert!(result.starts_with("[PARTIAL:"));
        assert!(result.contains("/v1/characters/:id/analysis"));
        assert!(result.split_once('\n').unwrap().1.len() <= max_read_bytes());
    }

    #[test]
    fn truncation_preserves_utf8_boundary() {
        let content = "界".repeat(max_read_bytes());
        let result = truncate_for_context(&content);
        assert!(result.starts_with("[PARTIAL:"));
        assert!(result
            .lines()
            .skip(1)
            .all(|line| line.is_char_boundary(line.len())));
    }

    #[test]
    fn custom_notice_replaces_default_hint() {
        let content = "y".repeat(max_read_bytes() + 10);
        let result = truncate_with_notice(&content, "use paged access");
        assert!(result.contains("use paged access"));
        assert!(!result.contains("/v1/characters/:id/analysis"));
    }
}
