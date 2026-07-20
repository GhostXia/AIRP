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

    // 清理输出：只保留以 "- " 开头的行
    let cleaned: Vec<&str> = result
        .lines()
        .filter(|line| line.trim().starts_with("- "))
        .collect();

    let compressed = cleaned.join("\n");

    // 若压缩结果为空或比原文更长，返回原文
    if compressed.is_empty() || compressed.chars().count() >= content.chars().count() {
        Ok(content.to_string())
    } else {
        Ok(compressed)
    }
}

#[cfg(test)]
mod tests {
    // 压缩功能需要 LLM 调用，单元测试仅验证配置和边界条件
    // 集成测试在 chat_pipeline/tests.rs 中进行
}
