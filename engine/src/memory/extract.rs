//! 自动事实抽取：从对话中抽取关键事实写入 resident memory。
//!
//! 抽取走控制平面（独立 LLM 调用，不污染角色 prompt）。
//! 在 `finalize_generation` 后异步触发。

use crate::adapter::{ChatMessage, GenerationParams, MessageRole, ProviderConfig};
use crate::error::AirpError;
use futures_util::StreamExt;
use std::sync::Arc;

/// 抽取配置。
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// 是否启用自动抽取。
    pub enabled: bool,
    /// 抽取用的 model（None 则复用主 model）。
    pub model: Option<String>,
    /// 抽取用的 temperature（低温保证确定性）。
    pub temperature: f32,
    /// 最大抽取 token 数。
    pub max_tokens: u32,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            temperature: 0.1,
            max_tokens: 500,
        }
    }
}

/// 抽取 prompt 模板。
const EXTRACTION_SYSTEM_PROMPT: &str = r#"你是一个记忆抽取助手。从对话中抽取关键事实，用于长期记忆。

抽取规则：
1. 只抽取持久性事实（用户偏好、角色关系、重要事件、世界设定）
2. 忽略临时性内容（问候、过渡句、重复内容）
3. 用简洁的条目格式输出，每条一行，以 "- " 开头
4. 如果没有值得记录的内容，输出空字符串

输出格式示例：
- 用户喜欢简洁的回复风格
- 角色艾莉娅是用户的妹妹
- 用户讨厌被叫"主人"
"#;

/// 从对话中抽取关键事实。
///
/// 返回抽取到的事实条目（markdown 列表格式），若无值得记录的内容则返回空字符串。
pub async fn extract_facts(
    client: &reqwest::Client,
    provider_config: Arc<ProviderConfig>,
    gen_params: GenerationParams,
    user_message: &str,
    assistant_message: &str,
    config: &ExtractionConfig,
) -> Result<String, AirpError> {
    if !config.enabled {
        return Ok(String::new());
    }

    // 构建抽取请求
    let conversation = format!(
        "用户: {}\n\n角色: {}",
        user_message, assistant_message
    );

    let messages = vec![ChatMessage {
        role: MessageRole::User,
        content: format!("请从以下对话中抽取关键事实：\n\n{}", conversation),
    }];

    // 派生抽取参数
    let mut extract_params = gen_params;
    if let Some(ref model) = config.model {
        extract_params.model = model.clone();
    }
    extract_params.temperature = Some(config.temperature);
    extract_params.max_tokens = Some(config.max_tokens);

    // 调用 LLM（使用 Direct 引擎）
    let mut stream = Box::pin(crate::adapter::call_streaming_api(
        client.clone(),
        provider_config,
        extract_params,
        EXTRACTION_SYSTEM_PROMPT.to_string(),
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

    Ok(cleaned.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_config_default() {
        let config = ExtractionConfig::default();
        assert!(config.enabled);
        assert!(config.model.is_none());
        assert!((config.temperature - 0.1).abs() < f32::EPSILON);
        assert_eq!(config.max_tokens, 500);
    }
}
