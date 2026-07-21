//! 记忆压缩：当 resident memory 超过容量上限时，用 LLM 合并压缩。

use crate::adapter::{ChatMessage, GenerationParams, MessageRole, ProviderConfig};
use crate::error::AirpError;
use futures_util::StreamExt;
use std::sync::Arc;

/// 压缩 prompt 模板。
const COMPRESSION_SYSTEM_PROMPT: &str = r#"你是一个记忆整理助手。将以下记忆条目合并压缩，保持关键信息不丢失。

压缩规则：
1. 合并相似或重复的条目
2. 删除过时或矛盾的信息（保留最新的）
3. 用简洁的语言重述
4. 保持条目格式，每条一行，以 "- " 开头
5. 压缩后的总字符数应明显少于原文

直接输出压缩后的条目，不要解释。
"#;

/// 清理 LLM 压缩输出：只保留以 "- " 开头的行（含其续行）。
///
/// 审计 W3 修复：原实现 `filter(|line| line.trim().starts_with("- "))` 会丢弃
/// 续行（被 LLM 换行的单条目），且当 LLM 完全不输出 "- " 前缀时返回空字符串，
/// 导致上层 `compress_resident_memory` 误判"压缩失败回退原文"，但其实
/// LLM 输出可能只是格式不严格。改进后：
/// 1. 续行（以空白开头且紧跟在 "- " 行后）保留
/// 2. 若过滤后为空，回退到 trim 后的原始输出（让 caller 仍能用 length 比较）
pub(crate) fn cleanup_compression_output(raw: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut prev_was_bullet = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ") {
            kept.push(line);
            prev_was_bullet = true;
        } else if prev_was_bullet && trimmed.is_empty() {
            // 空行保留为条目间分隔
            kept.push(line);
            prev_was_bullet = false;
        } else if prev_was_bullet && line.starts_with(char::is_whitespace) {
            // 续行：缩进的非空行紧跟 bullet，视为同一条目的一部分
            kept.push(line);
            // 续行后不再特殊处理，下一行若仍是缩进视为续行
        } else {
            // 非 bullet 行（如 LLM 解释文字）：丢弃
            prev_was_bullet = false;
        }
    }
    let cleaned = kept.join("\n");
    if cleaned.trim().is_empty() {
        // 回退：LLM 没产出符合格式的输出，返回 trim 后的原文让 caller 判断
        raw.trim().to_string()
    } else {
        cleaned
    }
}

/// 压缩 resident memory 内容。
///
/// 返回压缩后的内容。若压缩失败或结果为空，返回原内容。
pub async fn compress_resident_memory(
    client: &reqwest::Client,
    provider_config: Arc<ProviderConfig>,
    gen_params: GenerationParams,
    content: &str,
    target_chars: usize,
) -> Result<String, AirpError> {
    if content.trim().is_empty() {
        return Ok(String::new());
    }

    let messages = vec![ChatMessage {
        role: MessageRole::User,
        content: format!(
            "请压缩以下记忆条目到约 {} 字符以内：\n\n{}",
            target_chars, content
        ),
    }];

    // 派生压缩参数
    let mut compress_params = gen_params;
    compress_params.temperature = Some(0.2);
    compress_params.max_tokens = Some((target_chars as u32 * 2).max(1000));

    // 调用 LLM
    let mut stream = Box::pin(crate::adapter::call_streaming_api(
        client.clone(),
        provider_config,
        compress_params,
        COMPRESSION_SYSTEM_PROMPT.to_string(),
        messages,
    ));

    // 收集完整响应
    let mut result = String::new();
    while let Some(chunk) = stream.next().await {
        if let Ok(text) = chunk {
            result.push_str(&text);
        }
    }

    let compressed = cleanup_compression_output(&result);

    // #274 F-1: 膨胀校验 + warn
    let (final_result, fallback_reason) =
        decide_compression_result(content, &compressed, target_chars);
    if let Some(reason) = fallback_reason {
        tracing::warn!(
            reason = reason,
            original_len = content.chars().count(),
            compressed_len = compressed.chars().count(),
            target_chars,
            "compress_resident_memory: LLM 输出未达压缩目标，保留原内容"
        );
    }
    Ok(final_result)
}

/// 决定压缩结果：接受 compressed 还是回退到原内容。
///
/// #274 F-1 抽出为独立函数便于单测。返回 `(最终结果, 回退原因)`；
/// `回退原因 = None` 表示接受 compressed，`Some(reason)` 表示回退到原内容。
///
/// 回退条件（任一满足即回退）：
/// 1. `compressed` 为空（LLM 完全没产出有效输出）
/// 2. `compressed_len >= original_len`（LLM 反而膨胀）
/// 3. `compressed_len > target_chars`（LLM 没压到容量内，避免半压缩结果
///    在下一轮再次触发压缩，形成"压缩-未达标-再压缩"循环）
fn decide_compression_result(
    content: &str,
    compressed: &str,
    target_chars: usize,
) -> (String, Option<&'static str>) {
    if compressed.trim().is_empty() {
        return (content.to_string(), Some("empty_compressed_output"));
    }
    let compressed_len = compressed.chars().count();
    let original_len = content.chars().count();
    if compressed_len >= original_len {
        return (
            content.to_string(),
            Some("compressed_not_shorter_than_original"),
        );
    }
    if compressed_len > target_chars {
        return (
            content.to_string(),
            Some("compressed_exceeds_target_capacity"),
        );
    }
    (compressed.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_keeps_bullet_lines() {
        let raw = "以下是压缩后的记忆：\n- 用户喜欢猫\n- 角色叫艾莉娅\n";
        let cleaned = cleanup_compression_output(raw);
        assert!(!cleaned.contains("以下是压缩后的记忆"));
        assert!(cleaned.contains("- 用户喜欢猫"));
        assert!(cleaned.contains("- 角色叫艾莉娅"));
    }

    #[test]
    fn cleanup_keeps_continuation_lines() {
        // 审计 W3：续行不应丢失
        let raw = "- 用户喜欢猫，\n  尤其是橘猫\n- 角色叫艾莉娅\n";
        let cleaned = cleanup_compression_output(raw);
        assert!(cleaned.contains("- 用户喜欢猫"));
        assert!(cleaned.contains("  尤其是橘猫"));
        assert!(cleaned.contains("- 角色叫艾莉娅"));
    }

    #[test]
    fn cleanup_falls_back_to_trimmed_raw_when_no_bullets() {
        // 审计 W3：LLM 没输出 "- " 前缀时，回退到 trim 后原文，避免空字符串误判
        let raw = "  用户喜欢猫。角色叫艾莉娅。  \n";
        let cleaned = cleanup_compression_output(raw);
        assert_eq!(cleaned, "用户喜欢猫。角色叫艾莉娅。");
    }

    #[test]
    fn cleanup_returns_empty_for_empty_input() {
        let cleaned = cleanup_compression_output("");
        assert_eq!(cleaned, "");
    }

    #[test]
    fn cleanup_returns_empty_for_whitespace_only_input() {
        let cleaned = cleanup_compression_output("   \n  \n");
        assert_eq!(cleaned, "");
    }

    #[test]
    fn cleanup_preserves_blank_line_between_bullets() {
        let raw = "- 第一条\n\n- 第二条\n";
        let cleaned = cleanup_compression_output(raw);
        assert!(cleaned.contains("- 第一条"));
        assert!(cleaned.contains("\n\n- 第二条"));
    }

    // #274 F-1: decide_compression_result 单测
    #[test]
    fn decide_accepts_compressed_within_capacity() {
        let content = "abcdefghij"; // 10 chars
        let compressed = "abcde"; // 5 chars
        let (result, reason) = decide_compression_result(content, compressed, 8);
        assert_eq!(result, "abcde");
        assert!(reason.is_none(), "expected accept, got {:?}", reason);
    }

    #[test]
    fn decide_falls_back_when_compressed_is_empty() {
        let content = "abcdefghij";
        let (result, reason) = decide_compression_result(content, "", 8);
        assert_eq!(result, "abcdefghij");
        assert_eq!(reason, Some("empty_compressed_output"));
    }

    #[test]
    fn decide_falls_back_when_compressed_is_whitespace_only() {
        let content = "abcdefghij";
        let (result, reason) = decide_compression_result(content, "   \n  ", 8);
        assert_eq!(result, "abcdefghij");
        assert_eq!(reason, Some("empty_compressed_output"));
    }

    #[test]
    fn decide_falls_back_when_compressed_not_shorter_than_original() {
        let content = "abcde"; // 5 chars
        let compressed = "abcdef"; // 6 chars（比原文长）
        let (result, reason) = decide_compression_result(content, compressed, 100);
        assert_eq!(result, "abcde");
        assert_eq!(reason, Some("compressed_not_shorter_than_original"));
    }

    #[test]
    fn decide_falls_back_when_compressed_equals_original_length() {
        // 等长也算未压缩（>= 而不是 >）
        let content = "abcde";
        let compressed = "edcba";
        let (result, reason) = decide_compression_result(content, compressed, 100);
        assert_eq!(result, "abcde");
        assert_eq!(reason, Some("compressed_not_shorter_than_original"));
    }

    #[test]
    fn decide_falls_back_when_compressed_exceeds_target_capacity() {
        // compressed 比原文短，但仍超过 target_chars
        let content = "abcdefghij"; // 10 chars
        let compressed = "abcdefg"; // 7 chars
        let (result, reason) = decide_compression_result(content, compressed, 5);
        assert_eq!(result, "abcdefghij");
        assert_eq!(reason, Some("compressed_exceeds_target_capacity"));
    }

    #[test]
    fn decide_accepts_compressed_equal_to_target_capacity() {
        // compressed_len == target_chars：边界条件，接受
        let content = "abcdefghij"; // 10 chars
        let compressed = "abcde"; // 5 chars
        let (result, reason) = decide_compression_result(content, compressed, 5);
        assert_eq!(result, "abcde");
        assert!(reason.is_none());
    }

    #[test]
    fn decide_handles_multibyte_chars_correctly() {
        // 中文字符：chars().count() 按 Unicode 标量值计数，不是字节
        let content = "用户喜欢猫和狗"; // 7 chars
        let compressed = "爱猫狗"; // 3 chars
        let (result, reason) = decide_compression_result(content, compressed, 5);
        assert_eq!(result, "爱猫狗");
        assert!(reason.is_none());
    }
}
