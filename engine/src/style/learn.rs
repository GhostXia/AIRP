//! Phase 4.2: 风格迁移 — 从用户粘贴的文本样本提取风格特征。
//!
//! 触发方式：`POST /v1/style/learn` 端点
//!
//! 流程：
//! 1. 用户粘贴一段文本（小说片段 / 对话 / 散文等，1k~20k 字符）
//! 2. Engine 调用 LLM 提取结构化风格特征（语气/视角/节奏/修辞/词汇偏好）
//! 3. 提取结果以 markdown 条目形式写入 `styles/profiles/{profile_id}.md`
//! 4. 后续 `/v1/style/review` 与 prompt assembly 可读取该 profile 做风格对齐
//!
//! ## 设计要点
//! - **独立实现**：所有 prompt 与解析逻辑由 AIRP 自行设计，不复用任何第三方代码
//! - **profile_id 防注入**：handler 层校验 profile_id 仅允许 `[A-Za-z0-9_-]`，
//!   防止路径遍历写入任意文件
//! - **写入原子性**：临时文件 + rename，避免半写状态
//! - **幂等覆盖**：同一 profile_id 重复学习时覆盖旧 profile，不追加
//!   （drift 才是追加语义；profile 是覆盖语义）
//! - **不写 drift**：profile 是参考指南，drift 是动态修正；二者解耦

use crate::adapter::{ChatMessage, GenerationParams, MessageRole, ProviderConfig};
use crate::error::AirpError;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 风格学习请求。
#[derive(Debug, Clone, Deserialize)]
pub struct StyleLearnRequest {
    /// 待学习的文本样本（1k~20k 字符）。
    pub text: String,
    /// 可选 profile ID；默认 "default"。
    /// 校验规则：仅允许 `[A-Za-z0-9_-]`，1~64 字符。
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    /// 可选角色 ID；若提供，则同时写入角色专属 profile
    /// （`characters/{id}/style-profile.md`）。
    pub character_id: Option<String>,
}

fn default_profile_id() -> String {
    "default".to_string()
}

/// 风格学习响应。
#[derive(Debug, Clone, Serialize)]
pub struct StyleLearnResponse {
    /// 是否成功。
    pub success: bool,
    /// 写入的 profile 文件路径（相对 data_root）。
    pub profile_path: String,
    /// 提取的风格特征条目数。
    pub features_count: usize,
    /// 写入的 profile 内容（markdown 文本）。
    pub profile_content: String,
}

/// 提取出的结构化风格特征。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StyleFeatures {
    /// 语气倾向描述。
    pub tone: String,
    /// 叙事视角（第一/第三人称、全知/限制等）。
    pub perspective: String,
    /// 节奏特征（紧凑/舒缓/张弛有度等）。
    pub pacing: String,
    /// 修辞偏好（比喻多/白描/排比等）。
    pub rhetoric: String,
    /// 词汇层次（口语化/书面/古风等）。
    pub vocabulary: String,
    /// 其他显著特征列表。
    pub other_notes: Vec<String>,
}

/// 学习 prompt 模板。
///
/// 设计原则（AIRP 独立编写，非第三方复制）：
/// - 要求 LLM 输出严格 JSON，便于解析
/// - 限定 6 个维度，避免发散
/// - 显式约束不要复制原文，只描述风格特征（版权保护）
const LEARN_SYSTEM_PROMPT: &str = r#"你是 RP 风格分析助手。分析给定文本样本的风格特征，输出结构化 JSON。

分析维度：
1. tone: 语气倾向（如：冷峻、温暖、戏谑、庄重）
2. perspective: 叙事视角（如：第三人称限制、第一人称、全知视角）
3. pacing: 节奏特征（如：紧凑、舒缓、张弛有度、跳跃）
4. rhetoric: 修辞偏好（如：比喻密集、白描为主、排比有力）
5. vocabulary: 词汇层次（如：口语化、书面正式、古风、技术性）
6. other_notes: 其他显著风格特征数组

要求：
- 只描述风格，不要复述内容或引用原文
- 描述要具体可操作，便于 RP 模仿
- 每条 20-50 字
- 若样本太短无法判断，对应字段留空字符串或空数组

输出严格 JSON（不要 markdown 代码块）：
{
  "tone": "语气描述",
  "perspective": "视角描述",
  "pacing": "节奏描述",
  "rhetoric": "修辞描述",
  "vocabulary": "词汇描述",
  "other_notes": ["其他特征1", "其他特征2"]
}"#;

/// 执行风格学习，返回提取的特征。
pub async fn run_style_learn(
    client: &reqwest::Client,
    provider_config: Arc<ProviderConfig>,
    gen_params: GenerationParams,
    text: &str,
) -> Result<StyleFeatures, AirpError> {
    if text.trim().is_empty() {
        return Err(AirpError::BadRequest("text 样本不能为空".to_string()));
    }
    // 上限保护：超长样本截断到 20k 字符，避免 prompt 过长。
    let truncated: String = text.chars().take(20_000).collect();
    if truncated.chars().count() < 100 {
        return Err(AirpError::BadRequest(
            "text 样本过短（至少 100 字符）".to_string(),
        ));
    }

    let messages = vec![ChatMessage {
        role: MessageRole::User,
        content: format!("## 文本样本\n{}", truncated),
    }];

    let mut learn_params = gen_params;
    learn_params.temperature = Some(0.3);
    learn_params.max_tokens = Some(800);

    let mut stream = Box::pin(crate::adapter::call_streaming_api(
        client.clone(),
        provider_config,
        learn_params,
        LEARN_SYSTEM_PROMPT.to_string(),
        messages,
    ));

    let mut result = String::new();
    while let Some(chunk) = stream.next().await {
        let text = chunk.map_err(|e| AirpError::Upstream { status: 0, body: e })?;
        result.push_str(&text);
    }

    parse_style_features(&result)
}

/// 解析 LLM 输出的风格特征 JSON。
fn parse_style_features(text: &str) -> Result<StyleFeatures, AirpError> {
    let trimmed = text.trim();
    if let Ok(features) = serde_json::from_str::<StyleFeatures>(trimmed) {
        return Ok(features);
    }
    // 容错：从 markdown 代码块或混杂文本中提取 JSON
    let json_start = trimmed.find('{');
    let json_end = trimmed.rfind('}');
    if let (Some(start), Some(end)) = (json_start, json_end) {
        if end > start {
            let json_str = &trimmed[start..=end];
            if let Ok(features) = serde_json::from_str::<StyleFeatures>(json_str) {
                return Ok(features);
            }
        }
    }
    tracing::warn!("风格特征 JSON 解析失败，返回空特征");
    Ok(StyleFeatures::default())
}

/// 把风格特征渲染为 markdown profile 内容。
///
/// 输出格式示例：
/// ```markdown
/// # Style Profile: default
///
/// 来源：用户文本样本学习（2026-07-25T12:00:00Z）
///
/// - 语气：冷峻克制，少形容词
/// - 视角：第三人称限制视角
/// - 节奏：紧凑，短句为主
/// - 修辞：白描为主，少比喻
/// - 词汇：书面正式，技术性词汇
/// - 其他：对话简洁，避免感叹号
/// ```
pub fn render_profile_markdown(features: &StyleFeatures, profile_id: &str) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let mut md = String::with_capacity(512);
    md.push_str(&format!("# Style Profile: {}\n\n", profile_id));
    md.push_str(&format!("来源：用户文本样本学习（{}）\n\n", now));

    if !features.tone.is_empty() {
        md.push_str(&format!("- 语气：{}\n", features.tone));
    }
    if !features.perspective.is_empty() {
        md.push_str(&format!("- 视角：{}\n", features.perspective));
    }
    if !features.pacing.is_empty() {
        md.push_str(&format!("- 节奏：{}\n", features.pacing));
    }
    if !features.rhetoric.is_empty() {
        md.push_str(&format!("- 修辞：{}\n", features.rhetoric));
    }
    if !features.vocabulary.is_empty() {
        md.push_str(&format!("- 词汇：{}\n", features.vocabulary));
    }
    for note in &features.other_notes {
        if !note.trim().is_empty() {
            md.push_str(&format!("- 其他：{}\n", note));
        }
    }
    md
}

/// 校验 profile_id：仅允许 `[A-Za-z0-9_-]`，1~64 字符。
pub fn validate_profile_id(profile_id: &str) -> Result<(), AirpError> {
    if profile_id.is_empty() || profile_id.len() > 64 {
        return Err(AirpError::BadRequest(
            "profile_id 长度必须在 1~64 之间".to_string(),
        ));
    }
    if !profile_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AirpError::BadRequest(
            "profile_id 仅允许字母、数字、下划线、连字符".to_string(),
        ));
    }
    Ok(())
}

/// 返回全局 profile 路径：`{data_root}/styles/profiles/{profile_id}.md`
pub fn global_profile_path(data_root: &Path, profile_id: &str) -> PathBuf {
    data_root
        .join("styles")
        .join("profiles")
        .join(format!("{}.md", profile_id))
}

/// 返回角色专属 profile 路径：`{data_root}/characters/{cid}/style-profile.md`
pub fn character_profile_path(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("style-profile.md")
}

/// 写入 profile 文件（原子替换）。
///
/// 返回相对 data_root 的路径字符串。
pub fn write_profile(
    data_root: &Path,
    profile_path: &Path,
    content: &str,
) -> Result<String, AirpError> {
    if let Some(parent) = profile_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::data_dir::replace_file(profile_path, content.as_bytes())?;
    // 返回相对路径（统一使用正斜杠，跨平台一致，便于 HTTP 响应与 WebUI URL 拼接）
    let relative = profile_path
        .strip_prefix(data_root)
        .unwrap_or(profile_path)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(relative)
}

/// 计算非空特征条目数。
pub fn features_count(features: &StyleFeatures) -> usize {
    let mut count = 0;
    if !features.tone.is_empty() {
        count += 1;
    }
    if !features.perspective.is_empty() {
        count += 1;
    }
    if !features.pacing.is_empty() {
        count += 1;
    }
    if !features.rhetoric.is_empty() {
        count += 1;
    }
    if !features.vocabulary.is_empty() {
        count += 1;
    }
    count += features
        .other_notes
        .iter()
        .filter(|n| !n.trim().is_empty())
        .count();
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_features() {
        let json = r#"{
            "tone": "冷峻",
            "perspective": "第三人称限制",
            "pacing": "紧凑",
            "rhetoric": "白描",
            "vocabulary": "书面",
            "other_notes": ["对话简洁"]
        }"#;
        let features = parse_style_features(json).unwrap();
        assert_eq!(features.tone, "冷峻");
        assert_eq!(features.perspective, "第三人称限制");
        assert_eq!(features.other_notes.len(), 1);
    }

    #[test]
    fn test_parse_empty_features() {
        let json = r#"{
            "tone": "",
            "perspective": "",
            "pacing": "",
            "rhetoric": "",
            "vocabulary": "",
            "other_notes": []
        }"#;
        let features = parse_style_features(json).unwrap();
        assert!(features.tone.is_empty());
        assert!(features.other_notes.is_empty());
    }

    #[test]
    fn test_parse_invalid_returns_default() {
        let features = parse_style_features("not json").unwrap();
        assert!(features.tone.is_empty());
    }

    #[test]
    fn test_parse_json_in_markdown_block() {
        let text = "```json\n{\"tone\":\"test\",\"perspective\":\"\",\"pacing\":\"\",\"rhetoric\":\"\",\"vocabulary\":\"\",\"other_notes\":[]}\n```";
        let features = parse_style_features(text).unwrap();
        assert_eq!(features.tone, "test");
    }

    #[test]
    fn test_render_profile_markdown() {
        let features = StyleFeatures {
            tone: "冷峻".to_string(),
            perspective: "第三人称".to_string(),
            pacing: String::new(),
            rhetoric: String::new(),
            vocabulary: String::new(),
            other_notes: vec!["对话简洁".to_string()],
        };
        let md = render_profile_markdown(&features, "default");
        assert!(md.contains("# Style Profile: default"));
        assert!(md.contains("- 语气：冷峻"));
        assert!(md.contains("- 视角：第三人称"));
        assert!(!md.contains("- 节奏"));
        assert!(md.contains("- 其他：对话简洁"));
    }

    #[test]
    fn test_validate_profile_id_accepts_normal() {
        assert!(validate_profile_id("default").is_ok());
        assert!(validate_profile_id("my-style-1").is_ok());
        assert!(validate_profile_id("user_001").is_ok());
    }

    #[test]
    fn test_validate_profile_id_rejects_invalid() {
        assert!(validate_profile_id("").is_err());
        assert!(validate_profile_id("a/b").is_err());
        assert!(validate_profile_id("..").is_err());
        assert!(validate_profile_id("a:b").is_err());
        assert!(validate_profile_id("a b").is_err());
        // 长度超限
        let long = "a".repeat(65);
        assert!(validate_profile_id(&long).is_err());
    }

    #[test]
    fn test_features_count() {
        let features = StyleFeatures {
            tone: "x".to_string(),
            perspective: String::new(),
            pacing: "y".to_string(),
            rhetoric: String::new(),
            vocabulary: String::new(),
            other_notes: vec!["a".to_string(), "".to_string(), "b".to_string()],
        };
        assert_eq!(features_count(&features), 4);
    }

    #[test]
    fn test_write_and_read_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let path = global_profile_path(tmp.path(), "test");
        let written = write_profile(tmp.path(), &path, "# Test Profile\n\n- 语气：冷峻\n").unwrap();
        assert!(written.ends_with("styles/profiles/test.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("冷峻"));
    }

    #[test]
    fn test_character_profile_path() {
        let path = character_profile_path(Path::new("/data"), "hero");
        assert_eq!(
            path,
            PathBuf::from("/data/characters/hero/style-profile.md")
        );
    }

    #[test]
    fn test_global_profile_path() {
        let path = global_profile_path(Path::new("/data"), "default");
        assert_eq!(path, PathBuf::from("/data/styles/profiles/default.md"));
    }
}
