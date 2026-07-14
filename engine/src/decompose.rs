//! 角色卡/预设/世界书 → Markdown 拆解器。
//!
//! 设计依据：
//! - ASSET-SPEC.md §导入流程："重组成规格文件 = 代码，不是 Agent"
//! - MCP-SERVER-ABSORPTION.md §1：decompose_character / decompose_preset 🆕 需移植
//!
//! 两段式：
//! 1. 本模块（代码确定性）：读已解析的 `TavernCardV2` / `TavernPreset` / `Lorebook`
//!    生成 MD 骨架文件，含 `<!-- Agent分析后填充 -->` 占位符。不调 LLM。
//! 2. `EnhanceAnalysisTool`（agent 调 LLM）：填充占位符。本模块不涉及。
//!
//! 不变式守护：
//! - 不变式6：decompose 零 LLM 调用，输入是已解析结构化数据，不看原始大 blob
//! - 不变式①：MD 产物只含 RP 数据 + 占位符，零 agent 脚手架

use crate::error::AirpError;
use crate::orchestrator::card::{TavernCardV2, TavernPreset};
use crate::orchestrator::lorebook::Lorebook;
use std::path::Path;

/// 拆解结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DecomposeResult {
    /// B2 修复：统一字段名 `asset_id`，角色卡场景存 character_id，预设场景存 preset_id。
    pub asset_id: String,
    /// 资产类型（"character" 或 "preset"），供调用方区分。
    pub asset_type: String,
    pub target_dir: String,
    pub files_written: Vec<String>,
    /// 是否已包含世界书拆解（仅角色卡 decompose 用）。
    pub lorebook_decomposed: bool,
}

// ── CharacterDecomposer ──────────────────────────────────────────────────────

/// 角色卡拆解器。无状态，可复用。
pub struct CharacterDecomposer;

impl CharacterDecomposer {
    pub fn new() -> Self {
        Self
    }

    /// 拆解角色卡为 7 份 MD 骨架文件（+ 可选世界书子目录）。
    ///
    /// C3 修复：`raw_meta` 是从 `card/raw.json` 提取的顶层 JSON（仅取
    /// `data.creator` / `data.character_version` / `data.tags` 三个小字段，
    /// 不灌整个 blob，守不变式6）。`None` 时显示"（未定义）"。
    pub async fn decompose(
        &self,
        character_id: &str,
        card: &TavernCardV2,
        lorebook: Option<&Lorebook>,
        analysis_dir: &Path,
        raw_meta: Option<&serde_json::Value>,
    ) -> Result<DecomposeResult, AirpError> {
        tokio::fs::create_dir_all(analysis_dir).await?;

        let mut files_written = Vec::new();

        let files = [
            (
                "basic_info.md",
                Self::generate_basic_info(character_id, card, raw_meta),
            ),
            ("personality.md", Self::generate_personality(card)),
            ("world_setting.md", Self::generate_world_setting(card)),
            ("speech_style.md", Self::generate_speech_style(card)),
            ("greetings.md", Self::generate_greetings(card)),
            ("state_schema.md", Self::generate_state_schema(card)),
        ];

        for (filename, content) in &files {
            let path = analysis_dir.join(filename);
            tokio::fs::write(&path, content).await?;
            files_written.push(filename.to_string());
        }

        let mut lorebook_decomposed = false;
        if let Some(lb) = lorebook {
            if !lb.entries.is_empty() {
                let wb_dir = analysis_dir.join("world_book");
                tokio::fs::create_dir_all(&wb_dir).await?;
                let wb_files = Self::decompose_lorebook_entries(lb, &wb_dir).await?;
                files_written.extend(wb_files);
                lorebook_decomposed = true;
            }
        }

        // README.md（索引）放最后，需要 files 列表
        let readme = Self::generate_readme(character_id, card, &files_written);
        let readme_path = analysis_dir.join("README.md");
        tokio::fs::write(&readme_path, readme).await?;
        files_written.push("README.md".to_string());

        Ok(DecomposeResult {
            asset_id: character_id.to_string(),
            asset_type: "character".to_string(),
            target_dir: analysis_dir.display().to_string(),
            files_written,
            lorebook_decomposed,
        })
    }

    fn generate_basic_info(
        character_id: &str,
        card: &TavernCardV2,
        raw_meta: Option<&serde_json::Value>,
    ) -> String {
        let d = &card.data;
        // C3 修复：从 raw_meta 提取 creator/character_version/tags
        let creator = raw_meta
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("creator"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("（未定义）");
        let version = raw_meta
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("character_version"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("（未定义）");
        let tags = raw_meta
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("tags"))
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "（未定义）".to_string());

        format!(
            r#"# 基础信息

## 角色ID
{character_id}

## 名称
{name}

## 完整描述
{description}

## 创作者
{creator}

## 版本
{version}

## 标签
{tags}

<!-- Agent分析后填充：角色核心定位、受众、整体调性 -->
"#,
            character_id = character_id,
            name = d.name.as_deref().unwrap_or("（未定义）"),
            description = d.description.as_deref().unwrap_or("（未定义）"),
            creator = creator,
            version = version,
            tags = tags,
        )
    }

    fn generate_personality(card: &TavernCardV2) -> String {
        let d = &card.data;
        format!(
            r#"# 性格特征

## 性格描述
{personality}

## 场景中的行为模式
<!-- Agent分析后填充：从 description + personality + mes_example 提炼行为模式 -->

## 人际关系
<!-- Agent分析后填充：角色与他人的关系 -->
"#,
            personality = d.personality.as_deref().unwrap_or("（未定义）"),
        )
    }

    fn generate_world_setting(card: &TavernCardV2) -> String {
        let d = &card.data;
        format!(
            r#"# 世界观设定

## 场景
{scenario}

## 世界规则
<!-- Agent分析后填充：从 description + scenario 提炼世界规则 -->

## 时间线
<!-- Agent分析后填充：故事发生的时间背景 -->
"#,
            scenario = d.scenario.as_deref().unwrap_or("（未定义）"),
        )
    }

    fn generate_speech_style(card: &TavernCardV2) -> String {
        let d = &card.data;
        let examples = d.mes_example.as_deref().unwrap_or("（未定义）");
        format!(
            r#"# 说话风格

## 示例对话
{examples}

## 语气特征
<!-- Agent分析后填充：从 mes_example 提炼语气、用词习惯 -->

## 口头禅
<!-- Agent分析后填充：角色常用表达 -->
"#,
            examples = examples,
        )
    }

    fn generate_greetings(card: &TavernCardV2) -> String {
        let d = &card.data;
        let mut content = String::from("# 开场白\n\n");

        if let Some(first) = &d.first_mes {
            content.push_str("## 开场白 1（first_mes）\n\n");
            content.push_str(first);
            content.push_str("\n\n");
        }

        for (i, alt) in d.alternate_greetings.iter().enumerate() {
            content.push_str(&format!(
                "## 开场白 {}（alternate_greeting {}）\n\n",
                i + 2,
                i + 1
            ));
            content.push_str(alt);
            content.push_str("\n\n");
        }

        if d.first_mes.is_none() && d.alternate_greetings.is_empty() {
            content.push_str("（无开场白）\n\n");
        }

        content.push_str("<!-- Agent分析后填充：各开场白的情境分析、适用场景 -->\n");
        content
    }

    fn generate_state_schema(card: &TavernCardV2) -> String {
        let d = &card.data;
        format!(
            r#"# 状态定义

## 系统提示词
{system_prompt}

## 消息模板
{mes_template}

## 状态字段
<!-- Agent分析后填充：从 system_prompt + mes_template 提炼角色状态字段 -->
"#,
            system_prompt = d.system_prompt.as_deref().unwrap_or("（未定义）"),
            mes_template = d.mes_template.as_deref().unwrap_or("（未定义）"),
        )
    }

    /// 拆解世界书条目为独立 MD 文件 + 索引。
    ///
    /// B1 修复：文件名用 `entry_{:03}.md`（entry index 唯一标识），
    /// 避免中文 comment 全部下划线化导致文件名冲突。
    ///
    /// A2 修复：世界书条目不参与 Agent enhance（issue #87 精度约束），
    /// 故 MD 文件不含 `<!-- Agent分析后填充 -->` 占位符，仅作只读展示。
    async fn decompose_lorebook_entries(
        lorebook: &Lorebook,
        wb_dir: &Path,
    ) -> Result<Vec<String>, AirpError> {
        let mut files = Vec::new();

        for (idx, entry) in lorebook.entries.iter().enumerate() {
            let entry_md = Self::generate_lorebook_entry(entry, idx);
            let filename = format!("entry_{:03}.md", idx + 1);
            let path = wb_dir.join(&filename);
            tokio::fs::write(&path, entry_md).await?;
            files.push(format!("world_book/{}", filename));
        }

        // 索引
        let index_md = Self::generate_lorebook_index(lorebook);
        let index_path = wb_dir.join("index.md");
        tokio::fs::write(&index_path, index_md).await?;
        files.push("world_book/index.md".to_string());

        Ok(files)
    }

    fn generate_lorebook_entry(
        entry: &crate::orchestrator::lorebook::LorebookEntry,
        idx: usize,
    ) -> String {
        // E4 修复（CR4）：先 bind 到本地变量，避免 unwrap_or(&format!()) 借用临时值
        let name_fallback = format!("entry_{}", idx);
        let name = entry.comment.as_deref().unwrap_or(&name_fallback);
        format!(
            r#"# 世界书条目 {idx}

## 注释名
{name}

## 关键词
{keys}

## 优先级
{priority}

## 启用状态
{enabled}

## 内容
{content}

---

> 本条目为只读展示，不参与 Agent enhance（issue #87 精度约束）。
"#,
            idx = idx + 1,
            name = name,
            keys = entry.keys.join(", "),
            priority = entry.priority.unwrap_or(10),
            enabled = entry.enabled.unwrap_or(true),
            content = entry.content,
        )
    }

    fn generate_lorebook_index(lorebook: &Lorebook) -> String {
        let mut content = String::from("# 世界书条目索引\n\n");
        content.push_str("| 序号 | 注释名 | 关键词 | 链接 |\n");
        content.push_str("|------|--------|--------|------|\n");

        for (idx, entry) in lorebook.entries.iter().enumerate() {
            let filename = format!("entry_{:03}.md", idx + 1);
            let name_fallback = format!("entry_{}", idx);
            let name = entry.comment.as_deref().unwrap_or(&name_fallback);
            content.push_str(&format!(
                "| {:03} | {} | {} | [查看](./{}) |\n",
                idx + 1,
                name,
                entry.keys.join(", "),
                filename,
            ));
        }

        content
            .push_str("\n> 世界书条目为只读展示，不参与 Agent enhance（issue #87 精度约束）。\n");
        content
    }

    fn generate_readme(character_id: &str, card: &TavernCardV2, files: &[String]) -> String {
        let name = card.data.name.as_deref().unwrap_or("(unnamed)");
        let desc_short: String = card
            .data
            .description
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(100)
            .collect();

        let file_list = files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n");

        // C2 修复：世界书链接条件渲染
        let has_world_book = files.iter().any(|f| f.starts_with("world_book/"));
        let world_book_line = if has_world_book {
            "- [世界书](./world_book/index.md)\n"
        } else {
            ""
        };

        format!(
            r#"# {name}

> 角色ID: {character_id}
> 拆解产物：decompose 阶段生成的 Markdown 骨架。占位符 `<!-- Agent分析后填充 -->` 待 enhance_analysis 阶段填充。

## 快速引用

- [基础信息](./basic_info.md)
- [性格特征](./personality.md)
- [世界观设定](./world_setting.md)
- [说话风格](./speech_style.md)
- [开场白](./greetings.md)
- [状态定义](./state_schema.md)
{world_book_line}
## 一句话描述
{desc_short}

## 文件列表
共 {file_count} 个文件：
{file_list}
"#,
            name = name,
            character_id = character_id,
            desc_short = desc_short,
            file_count = files.len(),
            file_list = file_list,
            world_book_line = world_book_line,
        )
    }
}

impl Default for CharacterDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

// ── PresetDecomposer ─────────────────────────────────────────────────────────

/// 预设拆解器。无状态，可复用。
pub struct PresetDecomposer;

impl PresetDecomposer {
    pub fn new() -> Self {
        Self
    }

    pub async fn decompose(
        &self,
        preset_id: &str,
        preset: &TavernPreset,
        analysis_dir: &Path,
    ) -> Result<DecomposeResult, AirpError> {
        tokio::fs::create_dir_all(analysis_dir).await?;

        let mut files_written = Vec::new();

        // system_prompt.md
        let system_prompt = Self::generate_system_prompt(preset);
        let path = analysis_dir.join("system_prompt.md");
        tokio::fs::write(&path, system_prompt).await?;
        files_written.push("system_prompt.md".to_string());

        // parameters.md
        let parameters = Self::generate_parameters(preset);
        let path = analysis_dir.join("parameters.md");
        tokio::fs::write(&path, parameters).await?;
        files_written.push("parameters.md".to_string());

        // README.md
        let readme = Self::generate_readme(preset_id, preset, &files_written);
        let path = analysis_dir.join("README.md");
        tokio::fs::write(&path, readme).await?;
        files_written.push("README.md".to_string());

        Ok(DecomposeResult {
            asset_id: preset_id.to_string(),
            asset_type: "preset".to_string(),
            target_dir: analysis_dir.display().to_string(),
            files_written,
            lorebook_decomposed: false,
        })
    }

    fn generate_system_prompt(preset: &TavernPreset) -> String {
        let mut content = String::from("# 系统提示词\n\n");

        if let Some(prompts) = &preset.prompts {
            for (i, p) in prompts.iter().enumerate() {
                if !p.enabled {
                    continue;
                }
                content.push_str(&format!("## Prompt {} ({})\n\n", i + 1, p.name));
                content.push_str(&format!("- identifier: `{}`\n", p.identifier));
                content.push_str(&format!("- role: `{}`\n", p.role));
                if let Some(c) = &p.content {
                    content.push_str("\n```\n");
                    content.push_str(c);
                    content.push_str("\n```\n\n");
                } else {
                    content.push_str("\n（无内容）\n\n");
                }
            }
        } else {
            content.push_str("（无 prompts）\n\n");
        }

        content.push_str("<!-- Agent分析后填充：prompt 组合策略、注入顺序分析 -->\n");
        content
    }

    fn generate_parameters(preset: &TavernPreset) -> String {
        let mut content = String::from("# 参数\n\n");
        content.push_str(&format!(
            "| 参数 | 值 |\n|------|----|\n| temperature | {} |\n| max_tokens | {} |\n| model | {} |\n\n",
            preset.temperature.map(|t| t.to_string()).unwrap_or_else(|| "（未定义）".into()),
            preset.max_tokens.map(|t| t.to_string()).unwrap_or_else(|| "（未定义）".into()),
            preset.model.as_deref().unwrap_or("（未定义）"),
        ));
        content.push_str("<!-- Agent分析后填充：参数调优建议、模型兼容性 -->\n");
        content
    }

    fn generate_readme(preset_id: &str, preset: &TavernPreset, files: &[String]) -> String {
        let model = preset.model.as_deref().unwrap_or("(未指定 model)");
        let file_list = files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"# 预设 {preset_id}

> model: {model}
> 拆解产物：decompose 阶段生成的 Markdown 骨架。

## 快速引用

- [系统提示词](./system_prompt.md)
- [参数](./parameters.md)

## 文件列表
共 {file_count} 个文件：
{file_list}
"#,
            preset_id = preset_id,
            model = model,
            file_count = files.len(),
            file_list = file_list,
        )
    }
}

impl Default for PresetDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::card::{CharacterData, TavernCardV2, TavernPreset, TavernPrompt};
    use crate::orchestrator::lorebook::{Lorebook, LorebookEntry};
    use tempfile::tempdir;

    fn make_test_card() -> TavernCardV2 {
        TavernCardV2 {
            spec: Some("chara_card_v2".into()),
            spec_version: Some("2.0".into()),
            data: CharacterData {
                name: Some("林晚晴".into()),
                description: Some("一位温柔的图书馆管理员".into()),
                personality: Some("温和、细心、喜欢安静".into()),
                scenario: Some("市立图书馆".into()),
                first_mes: Some("你好，欢迎来到图书馆。".into()),
                mes_template: None,
                system_prompt: Some("你是一位图书馆管理员。".into()),
                mes_example: Some("用户：你好\n林晚晴：你好，需要帮忙找书吗？".into()),
                alternate_greetings: vec!["嗨，今天有什么想看的？".into()],
                character_book: None,
            },
        }
    }

    fn make_test_lorebook() -> Lorebook {
        Lorebook {
            entries: vec![
                LorebookEntry {
                    keys: vec!["图书馆".into()],
                    content: "市立图书馆有三层楼".into(),
                    enabled: Some(true),
                    priority: Some(10),
                    constant: None,
                    comment: Some("图书馆设定".into()),
                    secondary_keys: Vec::new(),
                    case_sensitive: None,
                    extensions: None,
                },
                LorebookEntry {
                    keys: vec!["书".into(), "阅读".into()],
                    content: "林晚晴喜欢推理小说".into(),
                    enabled: Some(true),
                    priority: Some(5),
                    constant: None,
                    comment: None,
                    secondary_keys: Vec::new(),
                    case_sensitive: None,
                    extensions: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn decompose_character_writes_seven_md_files() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = make_test_card();
        let result = CharacterDecomposer::new()
            .decompose("linwanqing", &card, None, &analysis_dir, None)
            .await
            .unwrap();

        assert_eq!(result.asset_id, "linwanqing");
        assert_eq!(result.asset_type, "character");
        assert!(result.files_written.contains(&"basic_info.md".to_string()));
        assert!(result.files_written.contains(&"personality.md".to_string()));
        assert!(result
            .files_written
            .contains(&"world_setting.md".to_string()));
        assert!(result
            .files_written
            .contains(&"speech_style.md".to_string()));
        assert!(result.files_written.contains(&"greetings.md".to_string()));
        assert!(result
            .files_written
            .contains(&"state_schema.md".to_string()));
        assert!(result.files_written.contains(&"README.md".to_string()));
        assert!(!result.lorebook_decomposed);
    }

    #[tokio::test]
    async fn decompose_character_with_lorebook() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = make_test_card();
        let lb = make_test_lorebook();
        let result = CharacterDecomposer::new()
            .decompose("linwanqing", &card, Some(&lb), &analysis_dir, None)
            .await
            .unwrap();

        assert!(result.lorebook_decomposed);
        assert!(result
            .files_written
            .contains(&"world_book/index.md".to_string()));
        // B1 修复：文件名用 entry_{:03}.md
        assert!(result
            .files_written
            .contains(&"world_book/entry_001.md".to_string()));
        assert!(result
            .files_written
            .contains(&"world_book/entry_002.md".to_string()));
    }

    #[tokio::test]
    async fn decompose_character_with_raw_meta() {
        // C3 修复：从 raw_meta 读 creator/version/tags
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = make_test_card();
        let raw_meta = serde_json::json!({
            "data": {
                "creator": "test_author",
                "character_version": "1.0",
                "tags": ["原创", "现代"]
            }
        });
        let _result = CharacterDecomposer::new()
            .decompose("linwanqing", &card, None, &analysis_dir, Some(&raw_meta))
            .await
            .unwrap();

        let basic_info = std::fs::read_to_string(analysis_dir.join("basic_info.md")).unwrap();
        assert!(basic_info.contains("test_author"));
        assert!(basic_info.contains("1.0"));
        assert!(basic_info.contains("原创, 现代"));
    }

    #[tokio::test]
    async fn decompose_character_empty_fields() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = TavernCardV2 {
            spec: None,
            spec_version: None,
            data: CharacterData {
                name: None,
                description: None,
                personality: None,
                scenario: None,
                first_mes: None,
                mes_template: None,
                system_prompt: None,
                mes_example: None,
                alternate_greetings: vec![],
                character_book: None,
            },
        };
        let _result = CharacterDecomposer::new()
            .decompose("emptychar", &card, None, &analysis_dir, None)
            .await
            .unwrap();

        let basic_info = std::fs::read_to_string(analysis_dir.join("basic_info.md")).unwrap();
        assert!(basic_info.contains("（未定义）"));
    }

    #[tokio::test]
    async fn decompose_character_readme_no_world_book_link_when_absent() {
        // C2 修复：无 lorebook 时 README 不含世界书链接
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = make_test_card();
        CharacterDecomposer::new()
            .decompose("linwanqing", &card, None, &analysis_dir, None)
            .await
            .unwrap();

        let readme = std::fs::read_to_string(analysis_dir.join("README.md")).unwrap();
        assert!(!readme.contains("[世界书]"));
    }

    #[tokio::test]
    async fn decompose_character_readme_has_world_book_link_when_present() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = make_test_card();
        let lb = make_test_lorebook();
        CharacterDecomposer::new()
            .decompose("linwanqing", &card, Some(&lb), &analysis_dir, None)
            .await
            .unwrap();

        let readme = std::fs::read_to_string(analysis_dir.join("README.md")).unwrap();
        assert!(readme.contains("[世界书]"));
    }

    #[test]
    fn entry_filename_is_pure_ascii_with_zero_padded_index() {
        // E2 修复后文件名清洗规则：固定 entry_{:03}.md
        for idx in 0..3 {
            let filename = format!("entry_{:03}.md", idx + 1);
            assert!(
                filename.chars().all(|c| c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, '_' | '.'))
                    && filename.ends_with(".md")
            );
        }
    }

    #[tokio::test]
    async fn decompose_lorebook_entry_files_have_no_enhance_placeholder() {
        // A2 修复：世界书条目不含 <!-- Agent分析后填充 --> 占位符
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = make_test_card();
        let lb = make_test_lorebook();
        CharacterDecomposer::new()
            .decompose("linwanqing", &card, Some(&lb), &analysis_dir, None)
            .await
            .unwrap();

        let entry_001 =
            std::fs::read_to_string(analysis_dir.join("world_book/entry_001.md")).unwrap();
        assert!(!entry_001.contains("Agent分析后填充"));
        assert!(entry_001.contains("只读展示"));
    }

    #[tokio::test]
    async fn decompose_preset_writes_md_files() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let preset = TavernPreset {
            prompts: Some(vec![TavernPrompt {
                identifier: "main".into(),
                name: "Main Prompt".into(),
                enabled: true,
                role: "system".into(),
                content: Some("You are an assistant.".into()),
                system_prompt: Some(true),
            }]),
            temperature: Some(0.8),
            max_tokens: Some(2048),
            model: Some("gpt-4".into()),
        };
        let result = PresetDecomposer::new()
            .decompose("mypreset", &preset, &analysis_dir)
            .await
            .unwrap();

        assert_eq!(result.asset_id, "mypreset");
        assert_eq!(result.asset_type, "preset");
        assert!(result
            .files_written
            .contains(&"system_prompt.md".to_string()));
        assert!(result.files_written.contains(&"parameters.md".to_string()));
        assert!(result.files_written.contains(&"README.md".to_string()));
        assert!(!result.lorebook_decomposed);
    }
}
