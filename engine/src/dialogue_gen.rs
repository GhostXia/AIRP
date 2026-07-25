//! Phase 4.3: 对话示例生成器（dialogue example generator）。
//!
//! 触发方式：`POST /v1/characters/:id/dialogue-examples` 端点
//!
//! 流程：
//! 1. 读取角色卡 `card/card.json`（或 fallback 到 `card.json`）
//! 2. 提取 description / personality / scenario / first_mes / name
//! 3. 调用 LLM 生成符合 SillyTavern `<START>` 分隔符格式的对话示例
//! 4. 用户确认后写入角色卡 `data.mes_example` 字段
//!
//! ## 设计要点
//! - **独立实现**：prompt 与解析逻辑由 AIRP 自行设计，不复用任何第三方代码
//! - **幂等覆盖**：默认覆盖 `mes_example`，请求体可选 `append=true` 追加
//! - **格式契约**：LLM 输出必须是 `<START>\n{{user}}: ...\n{{char}}: ...` 形式
//! - **dry_run 模式**：仅返回生成内容不写盘，便于用户预览
//! - **不破坏用户资产**：写入前保留原 mes_example 到 `mes_example.bak` 字段

use crate::adapter::{ChatMessage, GenerationParams, MessageRole, ProviderConfig};
use crate::error::AirpError;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 对话示例生成请求。
#[derive(Debug, Clone, Deserialize)]
pub struct DialogueExampleRequest {
    /// 生成几轮对话（1~10，默认 3）。
    #[serde(default = "default_turns")]
    pub turns: u32,
    /// 用户视角提示（如：「学生提问」「朋友闲聊」「对手挑衅」）。
    #[serde(default)]
    pub user_stance: Option<String>,
    /// 可选场景覆盖（默认用角色卡 scenario）。
    #[serde(default)]
    pub scenario_override: Option<String>,
    /// dry_run=true 仅返回生成内容，不写角色卡。
    #[serde(default)]
    pub dry_run: bool,
    /// append=true 追加到现有 mes_example 末尾（默认 false=覆盖）。
    #[serde(default)]
    pub append: bool,
    /// 可选用户 persona 名字（替换 {{user}} 默认占位符说明）。
    #[serde(default)]
    pub user_name: Option<String>,
}

fn default_turns() -> u32 {
    3
}

/// 对话示例生成响应。
#[derive(Debug, Clone, Serialize)]
pub struct DialogueExampleResponse {
    /// 是否写入角色卡（dry_run=true 时为 false）。
    pub written: bool,
    /// 角色 ID。
    pub character_id: String,
    /// 生成的对话示例文本（SillyTavern `<START>` 格式）。
    pub mes_example: String,
    /// 生成轮数。
    pub turns_generated: u32,
    /// 写入前的旧 mes_example（便于审计回滚；dry_run 时不填）。
    pub previous_mes_example: Option<String>,
}

/// 生成对话示例的 system prompt。
///
/// 设计原则（AIRP 独立编写，非第三方复制）：
/// - 强制 `<START>` 分隔符格式，兼容 SillyTavern mes_example
/// - 强制 `{{user}}` / `{{char}}` 占位符
/// - 限定输出长度，避免污染上下文
/// - 显式禁止生成 NSFW / 暴力 / 违法内容
const DIALOGUE_SYSTEM_PROMPT: &str = r#"你是 RP 对话示例生成助手。根据角色设定生成 SillyTavern 兼容的对话示例（mes_example 字段）。

格式契约（严格遵守）：
- 每段对话以 `<START>` 单独成行开头
- 用户发言：`{{user}}: 内容`
- 角色发言：`{{char}}: 内容`
- 角色动作/神态用 *asterisks* 包裹
- 不要输出任何解释、标题、JSON 或额外说明，只输出对话本身

内容要求：
- 严格符合角色 personality 与 scenario
- 体现角色独特语气与说话习惯
- 每条发言 1-3 句，避免冗长独白
- 用户发言要给角色创造发挥空间
- 禁止 NSFW、性暗示、极端暴力、违法行为
- 若 user_stance 指定用户视角，用户发言须符合该视角

输出长度：生成指定轮数的 `<START>` 块，每块一组 user+char 交互。"#;

/// 执行对话示例生成。
pub async fn run_dialogue_gen(
    client: &reqwest::Client,
    provider_config: Arc<ProviderConfig>,
    gen_params: GenerationParams,
    character_card: &serde_json::Value,
    req: &DialogueExampleRequest,
) -> Result<String, AirpError> {
    // 校验 turns
    if req.turns == 0 || req.turns > 10 {
        return Err(AirpError::BadRequest(
            "turns 必须在 1~10 之间".to_string(),
        ));
    }

    // 提取角色卡字段（支持 v2 data 嵌套 + v1 flat）
    let data = character_card.get("data").unwrap_or(character_card);
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("{{char}}");
    let description = data
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let personality = data
        .get("personality")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let scenario = req
        .scenario_override
        .as_deref()
        .or_else(|| data.get("scenario").and_then(|v| v.as_str()))
        .unwrap_or("");
    let first_mes = data
        .get("first_mes")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if description.is_empty() && personality.is_empty() {
        return Err(AirpError::BadRequest(
            "角色卡 description 和 personality 均为空，无法生成对话示例".to_string(),
        ));
    }

    let user_label = req.user_name.as_deref().unwrap_or("{{user}}");
    let stance_line = req
        .user_stance
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("用户视角：{}", s))
        .unwrap_or_default();

    let user_prompt = format!(
        "## 角色设定\n姓名：{}\n描述：{}\n性格：{}\n场景：{}\n开场白：{}\n\n## 生成要求\n{}\n用户占位符：{}\n生成轮数：{}",
        name,
        description,
        personality,
        scenario,
        first_mes,
        stance_line,
        user_label,
        req.turns
    );

    let messages = vec![ChatMessage {
        role: MessageRole::User,
        content: user_prompt,
    }];

    let mut gen = gen_params;
    gen.temperature = Some(0.7);
    // 每轮 ~200 tokens × turns + 缓冲
    gen.max_tokens = Some((300 * req.turns + 200).min(4000));

    let mut stream = Box::pin(crate::adapter::call_streaming_api(
        client.clone(),
        provider_config,
        gen,
        DIALOGUE_SYSTEM_PROMPT.to_string(),
        messages,
    ));

    let mut result = String::new();
    while let Some(chunk) = stream.next().await {
        let text = chunk.map_err(|e| AirpError::Upstream { status: 0, body: e })?;
        result.push_str(&text);
    }

    // 后处理：清理多余前缀/后缀，确保以 <START> 开头
    let cleaned = clean_dialogue_output(&result);
    if cleaned.trim().is_empty() {
        return Err(AirpError::Internal(
            "LLM 返回空内容，对话示例生成失败".to_string(),
        ));
    }
    Ok(cleaned)
}

/// 清理 LLM 输出：剥离 markdown 代码块、解释性前缀。
fn clean_dialogue_output(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // 去除 markdown 代码块
    if s.starts_with("```") {
        if let Some(start) = s.find('\n') {
            s = s[start + 1..].to_string();
        }
        if s.ends_with("```") {
            s.truncate(s.len() - 3);
        }
        s = s.trim().to_string();
    }
    // 截断到第一个 <START> 之前的内容
    if let Some(idx) = s.find("<START>") {
        s = s[idx..].to_string();
    }
    s.trim().to_string()
}

/// 统计 `<START>` 数量。
pub fn count_starts(text: &str) -> usize {
    text.matches("<START>").count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_markdown_codeblock() {
        let raw = "```markdown\n<START>\n{{user}}: hi\n{{char}}: hello\n```";
        let cleaned = clean_dialogue_output(raw);
        assert!(cleaned.starts_with("<START>"));
        assert!(!cleaned.contains("```"));
    }

    #[test]
    fn clean_strips_explanatory_prefix() {
        let raw = "这是生成的对话示例：\n\n<START>\n{{user}}: hi";
        let cleaned = clean_dialogue_output(raw);
        assert!(cleaned.starts_with("<START>"));
        assert!(!cleaned.contains("这是生成的对话示例"));
    }

    #[test]
    fn clean_handles_plain_start() {
        let raw = "<START>\n{{user}}: hi\n{{char}}: hello";
        let cleaned = clean_dialogue_output(raw);
        assert_eq!(cleaned, raw);
    }

    #[test]
    fn count_starts_works() {
        assert_eq!(count_starts("<START>\n<START>\n<START>"), 3);
        assert_eq!(count_starts("no starts here"), 0);
    }

    #[test]
    fn default_turns_is_three() {
        assert_eq!(default_turns(), 3);
    }

    #[test]
    fn request_defaults() {
        let req: DialogueExampleRequest = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(req.turns, 3);
        assert!(!req.dry_run);
        assert!(!req.append);
        assert!(req.user_stance.is_none());
    }

    #[test]
    fn request_with_options() {
        let json = r#"{"turns":5,"user_stance":"学生提问","dry_run":true,"append":true,"user_name":"小明"}"#;
        let req: DialogueExampleRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.turns, 5);
        assert_eq!(req.user_stance.as_deref(), Some("学生提问"));
        assert!(req.dry_run);
        assert!(req.append);
        assert_eq!(req.user_name.as_deref(), Some("小明"));
    }
}
