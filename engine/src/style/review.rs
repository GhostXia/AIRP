//! Style Review：RP 风格审查（4.1）。
//!
//! 触发方式：
//! - 自动：finalize 后每 N 轮（默认 10）异步触发
//! - 手动：`POST /v1/style/review` 端点
//!
//! 审查流程：收集最近 N 条 assistant 消息 → 对比风格 profile → LLM 输出结构化报告

use crate::adapter::{ChatMessage, GenerationParams, MessageRole, ProviderConfig};
use crate::error::AirpError;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 风格审查报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StyleReviewReport {
    /// 语气偏移描述。
    pub tone_drift: String,
    /// 叙事视角问题。
    pub perspective_issues: Vec<String>,
    /// 节奏问题。
    pub pacing_notes: String,
    /// 修正建议列表。
    pub suggestions: Vec<String>,
    /// 建议追加到 soul_drift.md 的修正条目。
    pub drift_patch: String,
}

/// 审查 prompt 模板。
const REVIEW_SYSTEM_PROMPT: &str = r#"你是 RP 风格审查助手。对比风格指南和最近生成内容，检查一致性。

检查维度：
1. 角色语气一致性（是否符合角色设定）
2. 叙事视角（是否保持一致的人称/视角）
3. 节奏（是否过快/过慢/突兀）
4. 与用户偏好对齐度

输出严格 JSON（不要 markdown 代码块）：
{
  "tone_drift": "语气偏移描述，无问题则为空字符串",
  "perspective_issues": ["视角问题1", "视角问题2"],
  "pacing_notes": "节奏问题，无问题则为空字符串",
  "suggestions": ["建议1", "建议2"],
  "drift_patch": "- 建议追加到 soul_drift 的修正条目"
}

若完全无问题，所有字段为空字符串/空数组。"#;

/// 执行风格审查。
pub async fn run_style_review(
    client: &reqwest::Client,
    provider_config: Arc<ProviderConfig>,
    gen_params: GenerationParams,
    style_profile: &str,
    recent_messages: &[String],
    current_drift: &str,
) -> Result<StyleReviewReport, AirpError> {
    if recent_messages.is_empty() {
        return Ok(StyleReviewReport::default());
    }

    let messages_text = recent_messages
        .iter()
        .enumerate()
        .map(|(i, msg)| format!("[{}]\n{}", i + 1, msg))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let mut user_content = format!(
        "## 风格指南\n{}\n\n## 最近生成内容\n{}",
        style_profile, messages_text
    );

    if !current_drift.trim().is_empty() {
        user_content.push_str(&format!("\n\n## 当前 Soul Drift\n{}", current_drift));
    }

    let messages = vec![ChatMessage {
        role: MessageRole::User,
        content: user_content,
    }];

    let mut review_params = gen_params;
    review_params.temperature = Some(0.2);
    review_params.max_tokens = Some(1000);

    let mut stream = Box::pin(crate::adapter::call_streaming_api(
        client.clone(),
        provider_config,
        review_params,
        REVIEW_SYSTEM_PROMPT.to_string(),
        messages,
    ));

    let mut result = String::new();
    while let Some(chunk) = stream.next().await {
        // 审计修复：传播流错误，而非静默丢弃导致空报告。
        let text = chunk.map_err(|e| AirpError::Upstream { status: 0, body: e })?;
        result.push_str(&text);
    }

    parse_review_report(&result)
}

/// 解析审查报告 JSON。
fn parse_review_report(text: &str) -> Result<StyleReviewReport, AirpError> {
    if let Ok(report) = serde_json::from_str::<StyleReviewReport>(text.trim()) {
        return Ok(report);
    }

    let json_start = text.find('{');
    let json_end = text.rfind('}');
    if let (Some(start), Some(end)) = (json_start, json_end) {
        if end > start {
            let json_str = &text[start..=end];
            if let Ok(report) = serde_json::from_str::<StyleReviewReport>(json_str) {
                return Ok(report);
            }
        }
    }

    tracing::warn!("风格审查报告解析失败，返回空报告");
    Ok(StyleReviewReport::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_report() {
        let json = r#"{
            "tone_drift": "语气偏正式",
            "perspective_issues": ["第三人称混入第一人称"],
            "pacing_notes": "",
            "suggestions": ["保持口语化"],
            "drift_patch": "- 语气应更口语化"
        }"#;
        let report = parse_review_report(json).unwrap();
        assert_eq!(report.tone_drift, "语气偏正式");
        assert_eq!(report.perspective_issues.len(), 1);
        assert!(report.pacing_notes.is_empty());
    }

    #[test]
    fn test_parse_empty_report() {
        let json = r#"{
            "tone_drift": "",
            "perspective_issues": [],
            "pacing_notes": "",
            "suggestions": [],
            "drift_patch": ""
        }"#;
        let report = parse_review_report(json).unwrap();
        assert!(report.tone_drift.is_empty());
        assert!(report.perspective_issues.is_empty());
    }

    #[test]
    fn test_parse_invalid_returns_default() {
        let report = parse_review_report("not json at all").unwrap();
        assert!(report.tone_drift.is_empty());
    }

    #[test]
    fn test_parse_json_in_markdown_block() {
        let text = "```json\n{\"tone_drift\": \"test\", \"perspective_issues\": [], \"pacing_notes\": \"\", \"suggestions\": [], \"drift_patch\": \"\"}\n```";
        let report = parse_review_report(text).unwrap();
        assert_eq!(report.tone_drift, "test");
    }
}
