//! M_LS-1: `<state>…</state>` tag extraction.
//!
//! 流式输出与持久化之前，把 `<state>` 块从可见文本中剥离，并把最后一个有效
//! JSON 状态返回给 finalizer 写入 `live.json`。本模块独立成文件便于聚焦
//! 解析不变量（unclosed tag 保留、无效 JSON 不更新 state、多 tag 末个生效）。

/// Strips all `<state>…</state>` blocks from `text`.
///
/// Returns `(text_without_state_tags, last_valid_state_json)`.
/// - All `<state>…</state>` blocks are removed from output text.
/// - The **last** block whose content parses as valid JSON is returned as `Some(Value)`.
/// - Unclosed `<state>` tag: kept in text as-is (graceful degradation).
/// - Invalid JSON inside tag: block still removed, but `last_state` not updated.
pub(crate) fn extract_state_content(text: &str) -> (String, Option<serde_json::Value>) {
    const OPEN: &str = "<state>";
    const CLOSE: &str = "</state>";

    let mut result = String::with_capacity(text.len());
    let mut last_state: Option<serde_json::Value> = None;
    let mut pos = 0;

    loop {
        match text[pos..].find(OPEN) {
            None => {
                result.push_str(&text[pos..]);
                break;
            }
            Some(tag_start) => {
                result.push_str(&text[pos..pos + tag_start]);
                let after_open = pos + tag_start + OPEN.len();
                match text[after_open..].find(CLOSE) {
                    None => {
                        // Unclosed tag — keep from <state> onward, stop scanning
                        result.push_str(&text[pos + tag_start..]);
                        break;
                    }
                    Some(content_len) => {
                        let json_str = &text[after_open..after_open + content_len];
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str.trim()) {
                            last_state = Some(v);
                        }
                        pos = after_open + content_len + CLOSE.len();
                    }
                }
            }
        }
    }

    (result, last_state)
}
