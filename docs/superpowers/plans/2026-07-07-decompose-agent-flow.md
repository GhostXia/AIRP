# Decompose Agent Flow Implementation Plan

> **历史实施计划**：PR #91 与 #93 已实现并修复主要 decompose/analysis 路径。本文的未勾选 checkbox 和“Execution Handoff”不再表示待执行任务；其中 `airp-engine-console/` WebUI 落点也已由 issue #105 / PR #106 另行处理。当前事实见 [../../PROJECT-AUDIT-2026-07-10.md](../../PROJECT-AUDIT-2026-07-10.md)。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 AIRP-MCP-Server 的 `decompose_character` / `decompose_preset` / `decompose_lorebook` / `enhance_analysis` 四工具移植到 engine，让 UI 显示 agent 整理后的 Markdown 文档（而非 raw JSON），同时为未来 Tauri UI 复用提供 HTTP 端点。

**Architecture:** 两段式：
1. **decompose 阶段（代码确定性，不调 LLM）** — 读已解析的 `TavernCardV2` / `TavernPreset` / `Lorebook` 结构化数据，生成 MD 骨架文件（含 `<!-- Agent分析后填充 -->` 占位符），写入 `data/characters/{id}/analysis/` sidecar 目录。符合 ASSET-SPEC.md §导入流程的"主干 = 代码归一化"原则，不烧 token。
2. **enhance_analysis 阶段（agent 调 LLM，旁路 sidecar）** — agent 读 MD 骨架 + 已解析字段（不灌原始大 blob，守不变式6），调 LLM 填充占位符。**不直接覆盖原 MD**：先生成 diff 预览，用户逐字段确认后才写入（对齐 issue #87 Agent-first 安全设计）。用户主动触发，不在导入主干。

**与 issue #87 的对齐（C1）**：本计划对应 issue #87 Phase 2 的"Agent 编辑 API"前置依赖。Task 6（enhance diff）对应 #87 的"生成 diff 预览，用户逐字段确认或拒绝"；Task 8 WebUI 对应 #87 的"右侧面板结构化编辑"初步集成。世界书条目**不纳入 Agent 自动编辑**（A2，对齐 #87 精度约束）。

触发方式：导入后 **不自动** decompose，UI 引导用户到工作台手动点"拆解"和"增强"按钮。HTTP 端点供 WebUI 和未来 Tauri UI 共用。

**Tech Stack:** Rust（engine 内部，axum + tokio + serde），无外部依赖新增。WebUI 沿用现有 `airp-engine-console/pages/workbench.html` 静态页 + fetch。

---

## 关键设计决策（写代码前必读）

### 1. 目录布局（沿用 ASSET-SPEC.md "analysis/ sidecar"约定）

```
data/characters/{id}/
├── card/
│   ├── card.json              # canonical 卡（已有）
│   ├── raw.json               # 原始导入 sidecar（已有）
│   └── greetings/00.md 等     # 单条开场白（已有，extract_card_assets 写）
├── world/
│   ├── lorebook.json          # 主世界书（已有）
│   └── extra/*.md             # 额外世界书（auto_converter 处理）
├── analysis/                  # 🆕 decompose 产物 sidecar
│   ├── basic_info.md
│   ├── personality.md
│   ├── world_setting.md
│   ├── speech_style.md
│   ├── greetings.md           # 聚合视图（first_mes + alternate_greetings + 占位符）
│   ├── state_schema.md
│   ├── README.md              # 索引
│   └── world_book/            # 世界书 decompose 产物（仅当有 character_book）
│       ├── index.md
│       └── entry_001_xxx.md 等
├── state/                     # 已有
├── sessions/                  # 已有
└── ...

data/presets/{id}/
├── preset.json                # canonical 预设（已有）
└── analysis/                  # 🆕 decompose 产物 sidecar
    ├── system_prompt.md
    ├── regex_rules.md
    ├── parameters.md
    └── README.md
```

**与现有 `card/greetings/` 的关系**：`card/greetings/00.md` 是单条开场白文件（供 orchestrator 装配时按 index 读），`analysis/greetings.md` 是聚合视图（含全部开场白 + agent 增强占位符）。两者并存，不冲突。

### 2. 不变式守护

- **不变式6（不烧 token）**：decompose 阶段零 LLM 调用；enhance 阶段只读 MD 骨架 + 已解析字段（不读 raw.json 原始大 blob）。
- **不变式①（干净提示词）**：decompose 产物只含 RP 数据 + 占位符，零 agent 脚手架。enhance 阶段的 LLM 调用走控制平面（系统提示词走 `chat_completion` 端点，不污染角色平面）。
- **路径沙箱**：analysis 目录路径函数复用 `data_dir::paths` 的 `validate_id_segment` 守护；HTTP 端点读 analysis 文件时，filename 走白名单（仅允许 `[a-z0-9_/.-]+\.md`）。

### 3. 模型映射（AIRP-MCP-Server → engine）

| MCP-Server 模型 | engine 现有等价 |
|---|---|
| `Character` (含 `card: TavernCardV2`, `data: CharacterData` 元信息) | `TavernCardV2` 直接用（engine 无独立 `Character` 包装，直接读 `card.json` 反序列化） |
| `Lorebook` / `LorebookEntry` | `engine::orchestrator::lorebook::{Lorebook, LorebookEntry}`（已存在） |
| `Preset` (含 `config: PresetConfig`) | `TavernPreset`（engine 现有，字段更少；decompose 时对缺失字段写"（未定义）"） |
| `AnalysisTier` | 不移植（engine 不维护分析等级元数据；README.md 中以"已拆解"/"已增强"二态显示，依据文件 mtime） |

---

## File Structure

### 新建文件

| 路径 | 职责 |
|---|---|
| `engine/src/decompose.rs` | 主 decompose 模块：`CharacterDecomposer` + `PresetDecomposer` + `decompose_lorebook`。纯函数 + async fs 写盘。（E2：原 `sanitize_filename` 已删除，文件名清洗由 `entry_{:03}.md` 固定模式 + `char_analysis_file_path` ASCII 白名单替代） |
| `engine/src/daemon/decompose_handlers.rs` | HTTP handlers：`decompose_character` / `decompose_preset` / `list_analysis_files` / `get_analysis_file` / `enhance_character_analysis`。 |
| `engine/tests/decompose_integration.rs` | 端到端集成测试：HTTP 端点 + 文件落盘验证。 |

### 修改文件

| 路径 | 修改内容 |
|---|---|
| `engine/src/lib.rs` | `pub mod decompose;` |
| `engine/src/data_dir/paths.rs` | 添加 `char_analysis_dir` / `char_analysis_file_path` / `preset_analysis_dir` / `preset_analysis_file_path`。 |
| `engine/src/agent/tools.rs` | 注册 4 个新工具：`DecomposeCharacterTool` / `DecomposePresetTool` / `DecomposeLorebookTool` / `EnhanceAnalysisTool`。 |
| `engine/src/daemon/mod.rs` | 注册新 HTTP 路由 + 在 `DaemonState` 中暴露 agent 工具注册表引用（供 enhance 复用）。 |
| `engine/src/daemon/handlers.rs` | `pub use` re-export decompose_handlers 中的 handler 函数。 |
| `airp-engine-console/pages/workbench.html` | 工作台添加"拆解角色卡"和"Agent 增强"按钮 + MD 显示区。 |
| `airp-engine-console/pages/characters.html` | 导入成功后显示"前往工作台拆解"提示链接。 |

---

## Task 1: 添加 analysis 目录路径函数

**Files:**
- Modify: `engine/src/data_dir/paths.rs`（在现有 `char_world_lorebook_path` 之后添加）
- Test: `engine/src/data_dir/paths.rs` 的 `#[cfg(test)] mod tests`

- [ ] **Step 1.1: 写失败测试**

在 `engine/src/data_dir/paths.rs` 末尾的 `#[cfg(test)]` 模块中添加（**E1 修复后**测试覆盖纯路径版 + 写路径版分离）：

```rust
#[test]
fn char_analysis_dir_path_does_not_create_directory() {
    // E1：纯路径版不得创建目录
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = char_analysis_dir_path(root, "alice").unwrap();
    assert!(!dir.exists(), "char_analysis_dir_path 不得创建目录");
    assert_eq!(
        dir,
        root.join("characters").join("alice").join("analysis")
    );
}

#[test]
fn ensure_char_analysis_dir_creates_directory() {
    // E1：写路径版会创建目录
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = ensure_char_analysis_dir(root, "alice").unwrap();
    assert!(dir.is_dir());
    assert_eq!(
        dir,
        root.join("characters").join("alice").join("analysis")
    );
}

#[test]
fn char_analysis_file_path_rejects_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let result = char_analysis_file_path(root, "alice", "../escape.md");
    assert!(result.is_err(), "路径穿越必须被拒");
    // E1：校验失败不得创建目录
    assert!(
        !root.join("characters").join("alice").join("analysis").exists(),
        "char_analysis_file_path 校验失败不得创建目录"
    );
}

#[test]
fn char_analysis_file_path_rejects_non_md_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let result = char_analysis_file_path(root, "alice", "basic_info.txt");
    assert!(result.is_err(), "仅允许 .md 扩展");
}

#[test]
fn preset_analysis_dir_path_does_not_create_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = preset_analysis_dir_path(root, "mypreset").unwrap();
    assert!(!dir.exists(), "preset_analysis_dir_path 不得创建目录");
    assert_eq!(dir, root.join("presets").join("mypreset").join("analysis"));
}

#[test]
fn ensure_preset_analysis_dir_creates_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = ensure_preset_analysis_dir(root, "mypreset").unwrap();
    assert!(dir.is_dir());
    assert_eq!(dir, root.join("presets").join("mypreset").join("analysis"));
}
```

- [ ] **Step 1.2: 运行测试验证失败**

Run: `cargo test --lib -p airp-engine paths::tests::char_analysis`
Expected: 编译错误（函数未定义）

- [ ] **Step 1.3: 实现路径函数**

**E1 修复（G1 + CR1）**：`engine/src/data_dir/paths.rs:258` 已存在 `pub(crate) fn char_analysis_dir(root, character_id) -> Result<PathBuf, AirpError>`，会调用 `create_dir_all`。本 Task **不复用** 该函数（避免在 list/get 读端点上意外创建空 sidecar 目录），而是新增一对纯路径 vs 写路径辅助函数：

- `char_analysis_dir_path`（纯路径解析，不创建目录）— 供 list/get/enhance 读端点用
- `ensure_char_analysis_dir`（封装 `char_analysis_dir` + `create_dir_all`）— 仅供 decompose/apply 写端点用

同理为 preset 新增 `preset_analysis_dir_path` + `ensure_preset_analysis_dir`。`char_analysis_file_path` / `preset_analysis_file_path` 内部调用纯路径版，避免读端点创建目录。

在 `engine/src/data_dir/paths.rs` 的 `char_world_lorebook_path` 之后添加：

```rust
/// `characters/{id}/analysis/` 目录路径（**纯解析，不创建目录**）。
///
/// E1 修复（G1+CR1）：list/get/enhance 读端点不得触发目录创建，
/// 否则用户访问不存在的 character 的 analysis 时会留下空 sidecar。
/// 写端点（decompose/apply）请改用 `ensure_char_analysis_dir`。
pub fn char_analysis_dir_path(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    // 复用现有 validate_id_segment 守护 character_id
    crate::data_dir::paths::validate_id_segment(character_id)?;
    Ok(root
        .join("characters")
        .join(character_id)
        .join("analysis"))
}

/// `characters/{id}/analysis/` 目录，**会创建**。
///
/// 仅供 decompose / apply 等写端点使用。读端点用 `char_analysis_dir_path`。
/// 注：现有 `pub(crate) fn char_analysis_dir`（paths.rs:258）行为与此一致，
/// 此处 `ensure_*` 函数为对外暴露的 `pub` 版本，供 decompose_handlers 调用。
pub fn ensure_char_analysis_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = char_analysis_dir_path(root, character_id)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `characters/{id}/analysis/{filename}` 路径，带白名单校验（**不创建目录**）。
///
/// 仅允许 `[a-z0-9_/.-]+\.md`，拒路径穿越、拒非 .md 扩展。
/// `filename` 例：`"basic_info.md"` / `"world_book/index.md"`。
///
/// E1 修复：内部调用 `char_analysis_dir_path`（不创建），保证 list/get/enhance
/// 读端点不会因校验文件名而留下空 analysis/ 目录。
pub fn char_analysis_file_path(
    root: &Path,
    character_id: &str,
    filename: &str,
) -> Result<PathBuf, AirpError> {
    use std::path::Component;

    // 白名单：仅小写字母、数字、_ / . -，且必须 .md 结尾
    let valid = filename
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '/' | '.' | '-'))
        && filename.ends_with(".md")
        && !filename.starts_with('/')
        && !filename.contains("..");
    if !valid {
        return Err(AirpError::BadRequest(format!(
            "invalid analysis filename: {} (only [a-z0-9_/.-]+.md allowed, no .. or leading /)",
            filename
        )));
    }

    let dir = char_analysis_dir_path(root, character_id)?;
    let path = dir.join(filename);

    // 二次防御：解析后所有 component 必须是 Normal
    let normal_check: bool = path
        .strip_prefix(&dir)
        .map_err(|_| AirpError::BadRequest("path escape".into()))?
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !normal_check {
        return Err(AirpError::BadRequest(
            "invalid analysis filename: path traversal detected".into(),
        ));
    }
    Ok(path)
}

/// `presets/{id}/analysis/` 目录路径（**纯解析，不创建目录**）。
pub fn preset_analysis_dir_path(root: &Path, preset_id: &str) -> Result<PathBuf, AirpError> {
    crate::data_dir::paths::validate_id_segment(preset_id)?;
    Ok(root.join("presets").join(preset_id).join("analysis"))
}

/// `presets/{id}/analysis/` 目录，**会创建**。仅供写端点使用。
pub fn ensure_preset_analysis_dir(root: &Path, preset_id: &str) -> Result<PathBuf, AirpError> {
    let dir = preset_analysis_dir_path(root, preset_id)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `presets/{id}/analysis/{filename}` 路径，带白名单校验（**不创建目录**）。
pub fn preset_analysis_file_path(
    root: &Path,
    preset_id: &str,
    filename: &str,
) -> Result<PathBuf, AirpError> {
    use std::path::Component;

    let valid = filename
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '/' | '.' | '-'))
        && filename.ends_with(".md")
        && !filename.starts_with('/')
        && !filename.contains("..");
    if !valid {
        return Err(AirpError::BadRequest(format!(
            "invalid analysis filename: {} (only [a-z0-9_/.-]+.md allowed)",
            filename
        )));
    }

    let dir = preset_analysis_dir_path(root, preset_id)?;
    let path = dir.join(filename);
    let normal_check: bool = path
        .strip_prefix(&dir)
        .map_err(|_| AirpError::BadRequest("path escape".into()))?
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !normal_check {
        return Err(AirpError::BadRequest(
            "invalid analysis filename: path traversal detected".into(),
        ));
    }
    Ok(path)
}
```

**执行者注意**：现有 `pub(crate) fn char_analysis_dir`（paths.rs:258）已 `#[allow(dead_code)]`，本 Task 不删除它（避免触动无关代码）。新增的 `ensure_char_analysis_dir` 与之语义等价但 `pub`，decompose_handlers 应调用 `ensure_*` 版本。若 `validate_id_segment` 不存在或签名不同，执行者需核对 `data_dir::paths` 现有 ID 校验函数名（可能叫 `validate_character_id` 或类似），按实际命名调用。

- [ ] **Step 1.4: 运行测试验证通过**

Run: `cargo test --lib -p airp-engine paths::tests`
Expected: PASS（包含新增 6 个测试，覆盖 E1 纯路径 vs 写路径分离）

- [ ] **Step 1.5: 提交**

```bash
git add engine/src/data_dir/paths.rs
git commit -m "feat(engine): 添加 analysis sidecar 目录路径函数 + 白名单校验"
```

---

## Task 2: 移植 CharacterDecomposer 到 engine

**Files:**
- Create: `engine/src/decompose.rs`
- Modify: `engine/src/lib.rs`（添加 `pub mod decompose;`）
- Test: `engine/src/decompose.rs` 的 `#[cfg(test)] mod tests`

- [ ] **Step 2.1: 创建 decompose.rs 骨架 + 写失败测试**

创建 `engine/src/decompose.rs`：

```rust
//! 角色卡/预设/世界书 → Markdown 拆解器。
//!
//! 设计依据：
//! - ASSET-SPEC.md §导入流程："重组成规格文件 = 代码，不是 Agent"
//! - MCP-SERVER-ABSORPTION.md §1：decompose_character / decompose_preset 🆕 需移植
//! - 移植自 `D:\airp-mcp-server\src\mcp\decompose.rs`，适配 engine 现有模型
//!
//! 两段式：
//! 1. 本模块（代码确定性）：读已解析的 `TavernCardV2` / `TavernPreset` / `Lorebook`
//!    生成 MD 骨架文件，含 `<!-- Agent分析后填充 -->` 占位符。不调 LLM。
//! 2. `EnhanceAnalysisTool`（agent 调 LLM）：填充占位符，覆盖原 MD。本模块不涉及。
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
    /// 旧字段名 `character_id` 在预设场景下误导，已废弃。
    pub asset_id: String,
    /// 资产类型（"character" 或 "preset"），供调用方区分。
    pub asset_type: String,
    pub target_dir: String,
    pub files_written: Vec<String>,
    /// 是否已包含世界书拆解（仅角色卡 decompose 用）。
    pub lorebook_decomposed: bool,
}

/// 角色卡拆解器。无状态，可复用。
pub struct CharacterDecomposer;

impl CharacterDecomposer {
    pub fn new() -> Self {
        Self
    }

    /// 拆解角色卡为 7 份 MD 骨架文件（+ 可选世界书子目录）。
    ///
    /// 输入：已解析的 `TavernCardV2`（不读 raw.json 原始 blob）。
    /// 输出：写入 `analysis_dir` 下的 MD 文件。
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

        // 1. basic_info.md
        let basic_info = Self::generate_basic_info(character_id, card, raw_meta);
        let path = analysis_dir.join("basic_info.md");
        tokio::fs::write(&path, basic_info).await?;
        files_written.push("basic_info.md".to_string());

        // 2. personality.md
        let personality = Self::generate_personality(card);
        let path = analysis_dir.join("personality.md");
        tokio::fs::write(&path, personality).await?;
        files_written.push("personality.md".to_string());

        // 3. world_setting.md
        let world_setting = Self::generate_world_setting(card);
        let path = analysis_dir.join("world_setting.md");
        tokio::fs::write(&path, world_setting).await?;
        files_written.push("world_setting.md".to_string());

        // 4. speech_style.md
        let speech_style = Self::generate_speech_style(card);
        let path = analysis_dir.join("speech_style.md");
        tokio::fs::write(&path, speech_style).await?;
        files_written.push("speech_style.md".to_string());

        // 5. greetings.md
        let greetings = Self::generate_greetings(card);
        let path = analysis_dir.join("greetings.md");
        tokio::fs::write(&path, greetings).await?;
        files_written.push("greetings.md".to_string());

        // 6. state_schema.md
        let state_schema = Self::generate_state_schema(card);
        let path = analysis_dir.join("state_schema.md");
        tokio::fs::write(&path, state_schema).await?;
        files_written.push("state_schema.md".to_string());

        // 7. README.md（索引，最后写）
        let readme = Self::generate_readme(character_id, card, &files_written);
        let path = analysis_dir.join("README.md");
        tokio::fs::write(&path, readme).await?;
        files_written.push("README.md".to_string());

        // 可选：世界书拆解
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
        // engine 的 CharacterData 不含这些字段，需从 raw.json 读取。
        // 只取这三个小字段，不灌整个 blob，守不变式6。
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
        format!(
            r#"# 性格特征

{personality}

## 性格关键词提取
<!-- Agent分析后填充 -->
<!-- 请分析上述性格描述，提取关键性格特征词 -->

## 行为模式
<!-- Agent分析后填充 -->
<!-- 请基于性格描述，推断角色的典型行为模式 -->
"#,
            personality = card
                .data
                .personality
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("（未定义）"),
        )
    }

    fn generate_world_setting(card: &TavernCardV2) -> String {
        format!(
            r#"# 世界观设定

## 场景背景
{scenario}

## 世界观要素
<!-- Agent分析后填充 -->
<!-- 请分析场景背景，提取以下要素： -->
<!-- - 时代背景 -->
<!-- - 地点设定 -->
<!-- - 社会环境 -->

## 关系网络
<!-- 如有定义，请在此描述角色与其他人物的关系 -->
"#,
            scenario = card
                .data
                .scenario
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("（未定义）"),
        )
    }

    fn generate_speech_style(card: &TavernCardV2) -> String {
        format!(
            r#"# 说话风格

## 示例对话
{examples}

## 语言特征
<!-- Agent分析后填充 -->
<!-- 请分析示例对话，提取以下特征： -->
<!-- - 语气特点 -->
<!-- - 常用表达 -->
<!-- - 禁忌话题 -->

## 对话注意事项
<!-- Agent分析后填充 -->
<!-- 请总结与该角色对话时需要注意的事项 -->
"#,
            examples = card
                .data
                .mes_example
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("（未定义）"),
        )
    }

    fn generate_greetings(card: &TavernCardV2) -> String {
        let mut content = String::from("# 开场白\n\n## 默认开场白\n");
        content.push_str(
            card.data
                .first_mes
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("（未定义）"),
        );
        content.push('\n');

        if !card.data.alternate_greetings.is_empty() {
            content.push_str("\n## 备选开场白\n");
            for (idx, alt) in card.data.alternate_greetings.iter().enumerate() {
                content.push_str(&format!("\n### 开场白 {}\n{}\n", idx + 1, alt));
            }
        }

        content.push_str(
            r#"
## 开场白选择建议
<!-- Agent分析后填充 -->
<!-- 请根据角色特点，给出不同场景下的开场白选择建议 -->
"#,
        );
        content
    }

    fn generate_state_schema(card: &TavernCardV2) -> String {
        // engine 不持久化 has_state_tracking，依据 character_book 是否存在推断
        let has_tracking = card.data.character_book.is_some();
        format!(
            r#"# 状态追踪定义

> 该角色是否支持状态追踪: {has_tracking}

## 状态字段

<!-- 如果角色支持状态追踪，请在此定义字段 -->
<!-- 格式：| 字段名 | 类型 | 当前值 | 最大值 | 说明 | -->

| 字段名 | 类型 | 当前值 | 最大值 | 说明 |
|--------|------|--------|--------|------|
<!-- 示例：-->
<!-- | hp | number | - | 100 | 生命值 | -->
<!-- | mp | number | - | 50 | 魔法值 | -->
<!-- | location | text | - | - | 当前位置 | -->

## 状态更新格式
在回复中使用以下格式更新状态：

```xml
<state>
{{
  "hp": {{"value": 75, "max": 100}},
  "location": "城镇广场"
}}
</state>
```

## 状态推断建议
<!-- Agent分析后填充 -->
<!-- 请根据角色卡内容，推断可能需要追踪的状态字段 -->
"#,
            has_tracking = if has_tracking { "是" } else { "否" },
        )
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

        // C2 修复：世界书链接条件渲染——仅当 files 含 world_book/ 开头文件时才显示
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

    async fn decompose_lorebook_entries(
        lorebook: &Lorebook,
        wb_dir: &Path,
    ) -> Result<Vec<String>, AirpError> {
        let mut files = Vec::new();
        // B1 修复：用 entry index 作为唯一标识，避免中文 comment 全部下划线化导致文件名冲突。
        // 文件名固定 `entry_{:03}.md`，人类可读名仅出现在 MD 内容标题和 index 表格中。
        for (idx, entry) in lorebook.entries.iter().enumerate() {
            let entry_md = Self::generate_lorebook_entry(entry, idx);
            let filename = format!("entry_{:03}.md", idx + 1);
            let path = wb_dir.join(&filename);
            tokio::fs::write(&path, entry_md).await?;
            files.push(format!("world_book/{}", filename));
        }

        let index = Self::generate_lorebook_index(&lorebook.entries);
        let path = wb_dir.join("index.md");
        tokio::fs::write(&path, index).await?;
        files.push("world_book/index.md".to_string());

        Ok(files)
    }

    fn generate_lorebook_entry(
        entry: &crate::orchestrator::lorebook::LorebookEntry,
        idx: usize,
    ) -> String {
        // A2（issue #87 精度约束）：世界书条目是高度结构化数据（keys/priority/enabled），
        // 不纳入 Agent 自动编辑范围，仅作只读展示。故 MD 骨架不含 `<!-- Agent分析后填充 -->` 占位符。
        format!(
            r#"# {name}

> 序号: {idx}
> 触发关键词: {keys}
> 优先级: {priority}
> 启用: {enabled}

## 内容

{content}

> 本条目为只读展示，不参与 Agent enhance（issue #87 精度约束）。
"#,
            // E4 修复（CR4）：原 `unwrap_or(&format!(...))` 借用临时 String，
            // 临时值在语句末尾销毁后 `name` 引用悬空，编译失败。先 bind 到本地变量。
            name = {
                let fallback = format!("entry_{}", idx);
                entry.comment.as_deref().unwrap_or(&fallback)
            },
            idx = idx,
            keys = entry.keys.join(", "),
            priority = entry.priority.unwrap_or(10),
            enabled = entry.enabled.unwrap_or(true),
            content = entry.content,
        )
    }

    fn generate_lorebook_index(
        entries: &[crate::orchestrator::lorebook::LorebookEntry],
    ) -> String {
        let mut content = format!(
            r#"# 世界书索引

> 共 {} 条条目

## 条目列表

| 编号 | 名称 | 触发关键词 | 文件 |
|------|------|------------|------|
"#,
            entries.len()
        );

        for (idx, entry) in entries.iter().enumerate() {
            // B1：文件名用 entry_{:03}.md，index 表格中的人类可读名仍取 comment。
            let filename = format!("entry_{:03}.md", idx + 1);
            // E4 修复（CR4）：同上，fallback bind 到本地变量。
            let name_fallback = format!("entry_{}", idx);
            content.push_str(&format!(
                "| {:03} | {} | {} | [查看](./{}) |\n",
                idx + 1,
                entry
                    .comment
                    .as_deref()
                    .unwrap_or(&name_fallback),
                entry.keys.join(", "),
                filename,
            ));
        }

        content.push_str(
            r#"
## 使用说明
当对话中出现触发关键词时，Agent应查阅对应条目获取背景信息。
"#,
        );

        content
    }
}

impl Default for CharacterDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

// E2 修复（G2+CR2）：原 `sanitize_filename` 函数已删除。
// 原因：B1 修复后世界书文件名固定为 `entry_{:03}.md`（纯 ASCII），不再需要 sanitize_filename；
// 且原实现用 `c.is_alphanumeric()` 接受中文，与 HTTP whitelist（`char_analysis_file_path`
// 仅允许 ASCII `[a-z0-9_/.-]`）不一致——中文文件名能写不能读。删除该死代码消除不一致风险。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::card::{CharacterData, TavernCardV2};
    use crate::orchestrator::lorebook::{Lorebook, LorebookEntry};
    use tempfile::tempdir;

    fn sample_card() -> TavernCardV2 {
        TavernCardV2 {
            spec: Some("chara_card_v2".into()),
            spec_version: Some("2.0".into()),
            data: CharacterData {
                name: Some("林婉清".into()),
                description: Some("温婉知性的古典文学研究者".into()),
                personality: Some("温柔、内敛、博学".into()),
                scenario: Some("午后书房".into()),
                first_mes: Some("你好，请进。".into()),
                mes_template: None,
                system_prompt: None,
                mes_example: Some("{{char}}：请坐。".into()),
                alternate_greetings: vec!["另一开场白".into()],
                character_book: None,
            },
        }
    }

    fn sample_lorebook() -> Lorebook {
        Lorebook {
            entries: vec![LorebookEntry {
                keys: vec!["天剑阁".into()],
                content: "天剑阁是江湖第一大派".into(),
                enabled: Some(true),
                priority: Some(10),
                comment: Some("tian_jian_ge".into()),
            }],
        }
    }

    #[test]
    fn entry_filename_is_pure_ascii_with_zero_padded_index() {
        // E2 修复后文件名清洗规则：固定 `entry_{:03}.md`，纯 ASCII，无 sanitize_filename 死代码。
        // 这里直接验证 B1 修复后的文件名生成规则。
        for idx in 0..3 {
            let filename = format!("entry_{:03}.md", idx + 1);
            assert_eq!(filename, format!("entry_{:03}.md", idx + 1));
            // 满足 char_analysis_file_path 的 ASCII 白名单
            assert!(filename
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.'))
                && filename.ends_with(".md"));
        }
    }

    #[tokio::test]
    async fn character_decompose_writes_seven_md_files() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = sample_card();

        let result = CharacterDecomposer::new()
            .decompose("linwanqing", &card, None, &analysis_dir, None)
            .await
            .unwrap();

        assert_eq!(result.files_written.len(), 7);
        assert!(result.files_written.contains(&"basic_info.md".to_string()));
        assert!(result.files_written.contains(&"README.md".to_string()));
        assert!(!result.lorebook_decomposed);

        let basic_info = tokio::fs::read_to_string(analysis_dir.join("basic_info.md"))
            .await
            .unwrap();
        assert!(basic_info.contains("林婉清"));
        assert!(basic_info.contains("linwanqing"));
    }

    #[tokio::test]
    async fn character_decompose_includes_lorebook_when_present() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = sample_card();
        let lb = sample_lorebook();

        let result = CharacterDecomposer::new()
            .decompose("linwanqing", &card, Some(&lb), &analysis_dir, None)
            .await
            .unwrap();

        assert!(result.lorebook_decomposed);
        assert!(result.files_written.iter().any(|f| f.starts_with("world_book/")));
        assert!(result.files_written.contains(&"world_book/index.md".to_string()));

        let index = tokio::fs::read_to_string(analysis_dir.join("world_book/index.md"))
            .await
            .unwrap();
        assert!(index.contains("天剑阁"));
    }

    #[tokio::test]
    async fn character_decompose_md_contains_placeholders() {
        let tmp = tempdir().unwrap();
        let analysis_dir = tmp.path().join("analysis");
        let card = sample_card();

        CharacterDecomposer::new()
            .decompose("linwanqing", &card, None, &analysis_dir, None)
            .await
            .unwrap();

        let personality = tokio::fs::read_to_string(analysis_dir.join("personality.md"))
            .await
            .unwrap();
        assert!(
            personality.contains("<!-- Agent分析后填充 -->"),
            "MD 骨架必须含占位符供 enhance 阶段填充"
        );
    }

    #[tokio::test]
    async fn character_decompose_handles_empty_fields() {
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

        let result = CharacterDecomposer::new()
            .decompose("emptychar", &card, None, &analysis_dir, None)
            .await
            .unwrap();

        assert_eq!(result.files_written.len(), 7);
        let basic_info = tokio::fs::read_to_string(analysis_dir.join("basic_info.md"))
            .await
            .unwrap();
        assert!(basic_info.contains("（未定义）"));
    }
}
```

- [ ] **Step 2.2: 在 lib.rs 注册模块**

修改 `engine/src/lib.rs`，在现有 `pub mod` 列表中添加：

```rust
pub mod decompose;
```

- [ ] **Step 2.3: 运行测试验证通过**

Run: `cargo test --lib -p airp-engine decompose::`
Expected: PASS（5 个测试全过）

- [ ] **Step 2.4: 提交**

```bash
git add engine/src/decompose.rs engine/src/lib.rs
git commit -m "feat(engine): 移植 CharacterDecomposer — 7份MD骨架+世界书子目录"
```

---

## Task 3: 移植 PresetDecomposer

**Files:**
- Modify: `engine/src/decompose.rs`（在 `CharacterDecomposer` 之后添加 `PresetDecomposer`）
- Test: `engine/src/decompose.rs` 的 `#[cfg(test)] mod tests`

- [ ] **Step 3.1: 写失败测试**

在 `engine/src/decompose.rs` 的 `#[cfg(test)] mod tests` 末尾添加：

```rust
fn sample_preset() -> TavernPreset {
    TavernPreset {
        prompts: Some(vec![crate::orchestrator::card::TavernPrompt {
            identifier: "main".into(),
            name: "Main Prompt".into(),
            enabled: true,
            role: "system".into(),
            content: Some("You are {{char}}.".into()),
            system_prompt: Some(true),
        }]),
        temperature: Some(0.8),
        max_tokens: Some(2048),
        model: Some("gpt-4".into()),
    }
}

#[tokio::test]
async fn preset_decompose_writes_four_md_files() {
    let tmp = tempdir().unwrap();
    let analysis_dir = tmp.path().join("analysis");
    let preset = sample_preset();

    let result = PresetDecomposer::new()
        .decompose("mypreset", &preset, &analysis_dir)
        .await
        .unwrap();

    assert_eq!(result.files_written.len(), 4);
    assert!(result.files_written.contains(&"system_prompt.md".to_string()));
    assert!(result.files_written.contains(&"parameters.md".to_string()));
    assert!(result.files_written.contains(&"README.md".to_string()));
}

#[tokio::test]
async fn preset_decompose_parameters_md_contains_temperature() {
    let tmp = tempdir().unwrap();
    let analysis_dir = tmp.path().join("analysis");
    let preset = sample_preset();

    PresetDecomposer::new()
        .decompose("mypreset", &preset, &analysis_dir)
        .await
        .unwrap();

    let params = tokio::fs::read_to_string(analysis_dir.join("parameters.md"))
        .await
        .unwrap();
    assert!(params.contains("temperature"));
    assert!(params.contains("0.8"));
}

#[tokio::test]
async fn preset_decompose_handles_empty_prompts() {
    let tmp = tempdir().unwrap();
    let analysis_dir = tmp.path().join("analysis");
    let preset = TavernPreset {
        prompts: None,
        temperature: None,
        max_tokens: None,
        model: None,
    };

    let result = PresetDecomposer::new()
        .decompose("empty", &preset, &analysis_dir)
        .await
        .unwrap();

    assert_eq!(result.files_written.len(), 4);
}
```

- [ ] **Step 3.2: 运行测试验证失败**

Run: `cargo test --lib -p airp-engine decompose::tests::preset_decompose`
Expected: 编译错误（`PresetDecomposer` 未定义）

- [ ] **Step 3.3: 实现 PresetDecomposer**

在 `engine/src/decompose.rs` 的 `CharacterDecomposer` impl 块之后、`mod tests` 之前添加：

```rust
/// 预设拆解器。无状态，可复用。
pub struct PresetDecomposer;

impl PresetDecomposer {
    pub fn new() -> Self {
        Self
    }

    /// 拆解预设为 4 份 MD 骨架文件。
    pub async fn decompose(
        &self,
        preset_id: &str,
        preset: &TavernPreset,
        analysis_dir: &Path,
    ) -> Result<DecomposeResult, AirpError> {
        tokio::fs::create_dir_all(analysis_dir).await?;

        let mut files_written = Vec::new();

        // 1. system_prompt.md
        let system_prompt = Self::generate_system_prompt(preset);
        let path = analysis_dir.join("system_prompt.md");
        tokio::fs::write(&path, system_prompt).await?;
        files_written.push("system_prompt.md".to_string());

        // 2. regex_rules.md
        let regex_rules = Self::generate_regex_rules(preset);
        let path = analysis_dir.join("regex_rules.md");
        tokio::fs::write(&path, regex_rules).await?;
        files_written.push("regex_rules.md".to_string());

        // 3. parameters.md
        let parameters = Self::generate_parameters(preset);
        let path = analysis_dir.join("parameters.md");
        tokio::fs::write(&path, parameters).await?;
        files_written.push("parameters.md".to_string());

        // 4. README.md
        let readme = Self::generate_readme(preset_id, preset);
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
        let prompts_count = preset.prompts.as_ref().map(|p| p.len()).unwrap_or(0);

        let mut content = format!(
            r#"# 系统提示词

> 共 {count} 条 prompt

## 启用的 prompt 内容

```
{prompts}
```

## 组装顺序说明
<!-- 由角色卡的各模块组合而成 -->
<!-- 组装顺序： -->
<!-- 1. 系统前缀（如有） -->
<!-- 2. 角色基础信息 -->
<!-- 3. 性格特征 -->
<!-- 4. 世界观设定 -->
<!-- 5. 当前状态（如有） -->
<!-- 6. 系统后缀（如有） -->

## Agent 增强占位
<!-- Agent分析后填充 -->
<!-- 请分析上述 prompt 是否需要按当前模型热调（temperature/max_tokens 等） -->
"#,
            count = prompts_count,
            prompts = preset
                .prompts
                .as_ref()
                .map(|ps| {
                    ps.iter()
                        .filter(|p| p.enabled)
                        .filter_map(|p| p.content.as_deref())
                        .collect::<Vec<_>>()
                        .join("\n---\n")
                })
                .unwrap_or_else(|| "（无启用的 prompt）".into()),
        );
        content
    }

    fn generate_regex_rules(preset: &TavernPreset) -> String {
        // engine 现有 TavernPreset 不持久化 regex_scripts（PARTS.md §F 列为待补字段）
        // 这里写出骨架结构，待 preset_regex 模块补齐后由 enhance 阶段填充
        format!(
            r#"# 正则过滤规则

> 当前预设的正则脚本：{status}

## 规则列表

<!-- Agent分析后填充 -->
<!-- engine 现有 TavernPreset 模型不持久化 regex_scripts 字段。 -->
<!-- 若该预设使用了 SillyTavern 正则脚本，请从 preset.json 的原始 raw 字段中 -->
<!-- 提取并在此列出。每条规则包含：findRegex / replaceString / placement / enabled -->

| 编号 | 名称 | 查找 | 替换 | 状态 |
|------|------|------|------|------|
<!-- 示例：-->
<!-- | 1 | trim_leading | `^\s+` | `` | 启用 | -->
"#,
            status = "engine 暂未持久化 regex_scripts 字段（待 Task 1.5 preset 正则补齐）",
        )
    }

    fn generate_parameters(preset: &TavernPreset) -> String {
        format!(
            r#"# 模型参数

| 参数 | 值 | 说明 |
|------|-----|------|
| temperature | {temperature} | 生成随机性 |
| max_tokens | {max_tokens} | 最大生成长度 |
| model | {model} | 模型名 |

## 停止序列
{stop_sequences}

## Agent 增强占位
<!-- Agent分析后填充 -->
<!-- 请根据当前模型特性，给出参数调优建议（如 temperature 是否需要调整） -->
"#,
            temperature = preset
                .temperature
                .map(|t| t.to_string())
                .unwrap_or_else(|| "（未定义）".into()),
            max_tokens = preset
                .max_tokens
                .map(|t| t.to_string())
                .unwrap_or_else(|| "（未定义）".into()),
            model = preset
                .model
                .as_deref()
                .unwrap_or("（未定义）"),
            stop_sequences = "（engine 现有 TavernPreset 暂未持久化 stop_sequences）",
        )
    }

    fn generate_readme(preset_id: &str, _preset: &TavernPreset) -> String {
        format!(
            r#"# 预设: {preset_id}

> 拆解产物：decompose 阶段生成的 Markdown 骨架。占位符待 enhance_analysis 阶段填充。

## 快速引用

- [系统提示词](./system_prompt.md)
- [正则规则](./regex_rules.md)
- [模型参数](./parameters.md)

## 说明
该预设定义了 AI 生成回复时的行为规范和参数设置。
"#
        )
    }
}

impl Default for PresetDecomposer {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 3.4: 运行测试验证通过**

Run: `cargo test --lib -p airp-engine decompose::`
Expected: PASS（角色卡 + 预设 全部测试过）

- [ ] **Step 3.5: 提交**

```bash
git add engine/src/decompose.rs
git commit -m "feat(engine): 移植 PresetDecomposer — 4份MD骨架"
```

---

## Task 4: 注册 decompose 为 agent 工具

**Files:**
- Modify: `engine/src/agent/tools.rs`（在 `DeleteCharacterTool` 之后添加 3 个新工具 + 在 `register_builtin_tools` 中注册）
- Test: `engine/src/agent/tools.rs` 的 `#[cfg(test)] mod tests`

- [ ] **Step 4.1: 写失败测试**

在 `engine/src/agent/tools.rs` 末尾的 `#[cfg(test)] mod tests` 中添加：

```rust
#[tokio::test]
async fn decompose_character_tool_writes_md_files() {
    use crate::orchestrator::card::{CharacterData, TavernCardV2};
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // 准备一个角色 card.json
    let char_dir = root.join("characters").join("alice");
    let card_dir = char_dir.join("card");
    fs::create_dir_all(&card_dir).unwrap();
    let card = TavernCardV2 {
        spec: Some("chara_card_v2".into()),
        spec_version: Some("2.0".into()),
        data: CharacterData {
            name: Some("Alice".into()),
            description: Some("test".into()),
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
    fs::write(
        card_dir.join("card.json"),
        serde_json::to_string(&card).unwrap(),
    )
    .unwrap();

    let state = std::sync::Arc::new(
        crate::daemon::DaemonState::for_test(root.to_path_buf()),
    );
    let tool = DecomposeCharacterTool { state: state.clone() };
    let result = tool
        .call(
            serde_json::json!({ "character_id": "alice" }),
            false,
        )
        .await
        .unwrap();
    assert!(!result.dry_run);
    let files = result.output["files_written"].as_array().unwrap();
    assert_eq!(files.len(), 7);
    assert!(char_dir.join("analysis/basic_info.md").is_file());
}

#[tokio::test]
async fn decompose_preset_tool_writes_md_files() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let preset_dir = root.join("presets").join("mypreset");
    fs::create_dir_all(&preset_dir).unwrap();
    fs::write(
        preset_dir.join("preset.json"),
        r#"{"prompts":[{"identifier":"main","name":"Main","enabled":true,"role":"system","content":"hi"}]}"#,
    )
    .unwrap();

    let state = std::sync::Arc::new(
        crate::daemon::DaemonState::for_test(root.to_path_buf()),
    );
    let tool = DecomposePresetTool { state: state.clone() };
    let result = tool
        .call(
            serde_json::json!({ "preset_id": "mypreset" }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        result.output["files_written"].as_array().unwrap().len(),
        4
    );
    assert!(preset_dir.join("analysis/system_prompt.md").is_file());
}
```

- [ ] **Step 4.2: 运行测试验证失败**

Run: `cargo test --lib -p airp-engine tools::tests::decompose`
Expected: 编译错误（`DecomposeCharacterTool` / `DecomposePresetTool` 未定义；`DaemonState::for_test` 可能也未定义）

- [ ] **Step 4.3: 确认 DaemonState 测试构造器存在**

先用 Grep 检查 `engine/src/daemon/mod.rs` 中是否已有 `DaemonState::for_test`：

```bash
# 检查方法
grep -n "for_test" engine/src/daemon/mod.rs
```

如果不存在，在 `engine/src/daemon/mod.rs` 的 `DaemonState` impl 中添加：

```rust
#[cfg(test)]
pub fn for_test(data_root: std::path::PathBuf) -> Self {
    use std::sync::Arc;
    Self {
        data_root,
        settings: Arc::new(crate::config::Settings::default()),
        // 其余字段按现有 DaemonState::new 的默认填充逻辑，按你仓内实际字段补齐
        // 如果 DaemonState 有 chat_store / volume_store 等字段，参考现有 #[cfg(test)] 测试
        // 中的构造方式。本计划假设有此构造器；若仓内已用其他方式构造测试 state，
        // 改用现有方式。
        ..Self::default_for_test()
    }
}
```

> **注意**：本步骤需要执行者确认 `DaemonState` 现有结构。如果仓内已有 `DaemonState::new_for_test` / `DaemonState::test_default` 等同义函数，直接复用并改测试代码。如果完全没有，参考 `engine/src/daemon/mod.rs` 内现有 `#[cfg(test)]` 模块中其它测试如何构造 state。

- [ ] **Step 4.4: 实现 3 个 decompose 工具**

在 `engine/src/agent/tools.rs` 的 `DeleteCharacterTool` impl 块之后添加：

```rust
// ── M_AGENT-2 第三批：decompose 工具（移植自 AIRP-MCP-Server） ─────────────
//
// decompose_character / decompose_preset：代码确定性生成 MD 骨架（含占位符）。
// 不调 LLM，符合 ASSET-SPEC.md §导入流程"重组成规格文件 = 代码"原则。
// enhance_analysis 是单独的工具（调 LLM 填充占位符），见 Task 7。

/// `decompose_character`：把角色卡拆解为 7 份 MD 骨架（+ 可选世界书子目录）。
/// params: `{ "character_id": string }` → `{ "files_written": string[], "target_dir": string, "lorebook_decomposed": bool }`
struct DecomposeCharacterTool {
    state: Arc<DaemonState>,
}

impl Tool for DecomposeCharacterTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "decompose_character",
            description: "Decompose a character card into 7 Markdown skeleton files (basic_info/personality/world_setting/speech_style/greetings/state_schema + README) under data/characters/{id}/analysis/. Code-deterministic, no LLM call. Placeholders are left for enhance_analysis.",
            side_effect: ToolSideEffect::Mutate,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let cid_str = params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let cid = CharacterId::new(cid_str)?;

            // 读 card.json（已解析的结构化数据，不读 raw.json 大 blob）
            let card_text = data_dir::get_character(&state.data_root, &cid)?;
            let card: crate::orchestrator::card::TavernCardV2 = serde_json::from_str(&card_text)
                .map_err(|e| {
                    AirpError::BadRequest(format!(
                        "character {} card.json is not a valid TavernCardV2: {}",
                        cid, e
                    ))
                })?;

            // 读主 lorebook（若存在）
            let lb_path = data_dir::paths::char_world_lorebook_path(&state.data_root, cid.as_str());
            let lorebook: Option<crate::orchestrator::lorebook::Lorebook> = if lb_path.exists() {
                match std::fs::read_to_string(&lb_path) {
                    Ok(text) => serde_json::from_str(&text).ok(),
                    Err(_) => None,
                }
            } else {
                None
            };

            let analysis_dir = data_dir::paths::ensure_char_analysis_dir(&state.data_root, cid.as_str())?;
            // C3 修复：读 raw.json 提取 creator/character_version/tags 元信息
            let raw_meta = {
                let raw_path = state.data_root.join("characters").join(cid.as_str()).join("card").join("raw.json");
                if raw_path.exists() {
                    std::fs::read_to_string(&raw_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                } else {
                    None
                }
            };
            let decomposer = crate::decompose::CharacterDecomposer::new();
            let result = decomposer
                .decompose(cid.as_str(), &card, lorebook.as_ref(), &analysis_dir, raw_meta.as_ref())
                .await?;

            Ok(ToolResult {
                output: serde_json::to_value(&result)?,
                dry_run: false,
            })
        })
    }
}

/// `decompose_preset`：把预设拆解为 4 份 MD 骨架。
/// params: `{ "preset_id": string }` → `{ "files_written": string[], "target_dir": string }`
struct DecomposePresetTool {
    state: Arc<DaemonState>,
}

impl Tool for DecomposePresetTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "decompose_preset",
            description: "Decompose a preset into 4 Markdown skeleton files (system_prompt/regex_rules/parameters + README) under data/presets/{id}/analysis/. Code-deterministic, no LLM call.",
            side_effect: ToolSideEffect::Mutate,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let pid_str = params
                .get("preset_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing preset_id".into()))?;
            let pid = crate::types::PresetId::new(pid_str)?;

            // 读 preset.json
            let preset_path = data_dir::paths::preset_json_path(&state.data_root, pid.as_str());
            if !preset_path.exists() {
                return Err(AirpError::NotFound(format!(
                    "preset {} has no preset.json at {}",
                    pid,
                    preset_path.display()
                )));
            }
            let preset_text = std::fs::read_to_string(&preset_path)?;
            let preset: crate::orchestrator::card::TavernPreset =
                serde_json::from_str(&preset_text).map_err(|e| {
                    AirpError::BadRequest(format!(
                        "preset {} preset.json is not a valid TavernPreset: {}",
                        pid, e
                    ))
                })?;

            let analysis_dir = data_dir::paths::ensure_preset_analysis_dir(&state.data_root, pid.as_str())?;
            let decomposer = crate::decompose::PresetDecomposer::new();
            let result = decomposer
                .decompose(pid.as_str(), &preset, &analysis_dir)
                .await?;

            Ok(ToolResult {
                output: serde_json::to_value(&result)?,
                dry_run: false,
            })
        })
    }
}

/// `decompose_lorebook`：单独拆解某角色的世界书（用于 character_book 之外的额外世界书）。
/// params: `{ "character_id": string, "lorebook_path": string? }` → `{ "files_written": string[] }`
/// lorebook_path 缺省时读 `world/lorebook.json`。
struct DecomposeLorebookTool {
    state: Arc<DaemonState>,
}

impl Tool for DecomposeLorebookTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "decompose_lorebook",
            description: "Decompose a character's lorebook into per-entry Markdown files + index.md under data/characters/{id}/analysis/world_book/. Code-deterministic, no LLM call.",
            side_effect: ToolSideEffect::Mutate,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let cid_str = params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let cid = CharacterId::new(cid_str)?;

            let lb_path = match params.get("lorebook_path").and_then(|v| v.as_str()) {
                Some(custom) => {
                    // E5 修复（CR5）：原 `Path::starts_with` 仅 lexical 检查，可被 symlink 绕过。
                    // 改为 component 白名单校验（与 char_analysis_file_path 同款）：
                    // 1. 必须是相对路径（拒绝对/绝对路径）
                    // 2. 所有 component 必须是 Normal（拒绝 `..` / `/` / RootDir 等）
                    // 3. canonicalize 后再校验仍在 character 目录下（防 symlink 跳出）
                    use std::path::Component;
                    let p = std::path::Path::new(custom);
                    if p.is_absolute() {
                        return Err(AirpError::BadRequest(
                            "lorebook_path must be relative (got absolute path)".into(),
                        ));
                    }
                    let char_root = state
                        .data_root
                        .join("characters")
                        .join(cid.as_str());
                    let resolved = char_root.join(p);
                    let all_normal = resolved
                        .strip_prefix(&char_root)
                        .map_err(|_| AirpError::BadRequest("lorebook_path escapes character dir".into()))?
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)));
                    if !all_normal {
                        return Err(AirpError::BadRequest(format!(
                            "lorebook_path must be under data/characters/{}/ and contain no '..' or root components",
                            cid
                        )));
                    }
                    // 防御 symlink：若文件已存在，canonicalize 后必须仍在 char_root 下
                    if let Ok(canon) = resolved.canonicalize() {
                        if !canon.starts_with(&char_root) {
                            return Err(AirpError::BadRequest(
                                "lorebook_path resolves outside character dir (symlink escape?)".into(),
                            ));
                        }
                    }
                    resolved
                }
                None => data_dir::paths::char_world_lorebook_path(&state.data_root, cid.as_str()),
            };

            if !lb_path.exists() {
                return Err(AirpError::NotFound(format!(
                    "lorebook not found at {}",
                    lb_path.display()
                )));
            }
            let lb_text = std::fs::read_to_string(&lb_path)?;
            let lb: crate::orchestrator::lorebook::Lorebook = serde_json::from_str(&lb_text)
                .map_err(|e| {
                    AirpError::BadRequest(format!("lorebook JSON parse failed: {}", e))
                })?;

            let analysis_dir = data_dir::paths::ensure_char_analysis_dir(&state.data_root, cid.as_str())?;
            let wb_dir = analysis_dir.join("world_book");
            tokio::fs::create_dir_all(&wb_dir).await?;

            let decomposer = crate::decompose::CharacterDecomposer::new();
            // 复用 CharacterDecomposer 的内部方法（已 pub(crate) 暴露）
            // 通过 decompose 重新调用整卡拆解，传入 lorebook，files_written 含 world_book/*
            // 这里只需 lorebook 部分，单独走静态方法
            let files = crate::decompose::decompose_lorebook_standalone(&lb, &wb_dir).await?;

            Ok(ToolResult {
                output: serde_json::json!({
                    // E3 修复（G3）：原 typo `"character_id:"` 多冒号，已修正
                    "character_id": cid.as_str(),
                    "files_written": files,
                    "target_dir": wb_dir.display().to_string(),
                }),
                dry_run: false,
            })
        })
    }
}
```

- [ ] **Step 4.5: 暴露 standalone decompose_lorebook 函数**

在 `engine/src/decompose.rs` 末尾、`mod tests` 之前添加：

```rust
/// 独立拆解世界书（不重写整卡 decompose）。
/// 
/// 供 `DecomposeLorebookTool` 单独调用：当用户只想刷新世界书拆解、不重做整卡时。
pub async fn decompose_lorebook_standalone(
    lorebook: &Lorebook,
    wb_dir: &Path,
) -> Result<Vec<String>, AirpError> {
    tokio::fs::create_dir_all(wb_dir).await?;
    let decomposer = CharacterDecomposer::new();
    // 复用 CharacterDecomposer 的私有静态方法 — 通过新加 pub(crate) 包装
    decomposer.decompose_lorebook_pub(lorebook, wb_dir).await
}
```

并在 `CharacterDecomposer` impl 中加一个 `pub(crate)` 包装：

```rust
impl CharacterDecomposer {
    // ... 现有方法 ...

    /// `pub(crate)` 包装，供 standalone 调用复用。
    pub(crate) async fn decompose_lorebook_pub(
        &self,
        lorebook: &Lorebook,
        wb_dir: &Path,
    ) -> Result<Vec<String>, AirpError> {
        Self::decompose_lorebook_entries(lorebook, wb_dir).await
    }
}
```

- [ ] **Step 4.6: 在工具注册表中注册新工具**

在 `engine/src/agent/tools.rs` 的 `register_builtin_tools` 函数（或等效注册点）中添加：

```rust
// decompose 工具（MCP-SERVER-ABSORPTION §1 移植）
registry.register(Box::new(DecomposeCharacterTool { state: state.clone() }))?;
registry.register(Box::new(DecomposePresetTool { state: state.clone() }))?;
registry.register(Box::new(DecomposeLorebookTool { state: state.clone() }))?;
```

> **注意**：执行者需先用 Grep 找到 `register_builtin_tools` 的实际位置和签名（仓内可能叫别的名字，如 `default_registry` / `build_registry`）。如果现有注册函数不接 `state` 参数，按现有模式适配。

- [ ] **Step 4.7: 运行测试验证通过**

Run: `cargo test --lib -p airp-engine tools::tests::decompose`
Expected: PASS

- [ ] **Step 4.8: 提交**

```bash
git add engine/src/agent/tools.rs engine/src/decompose.rs
git commit -m "feat(agent): 注册 decompose_character/decompose_preset/decompose_lorebook 工具"
```

---

## Task 5: 添加 decompose HTTP 端点

**Files:**
- Create: `engine/src/daemon/decompose_handlers.rs`
- Modify: `engine/src/daemon/mod.rs`（注册路由）
- Modify: `engine/src/daemon/handlers.rs`（re-export）
- Test: `engine/src/daemon/decompose_handlers.rs` 的 `#[cfg(test)] mod tests`

- [ ] **Step 5.1: 创建 handlers 文件 + 写失败测试**

创建 `engine/src/daemon/decompose_handlers.rs`：

```rust
//! Decompose HTTP handlers：触发拆解 + 读拆解产物。
//!
//! 端点：
//! - `POST /v1/characters/:id/decompose` — 拆解角色卡
//! - `POST /v1/presets/:id/decompose` — 拆解预设
//! - `GET  /v1/characters/:id/analysis` — 列出 analysis 文件
//! - `GET  /v1/characters/:id/analysis/*filename` — 读单个 analysis 文件
//! - `GET  /v1/presets/:id/analysis` — 列出预设 analysis 文件
//! - `GET  /v1/presets/:id/analysis/*filename` — 读单个预设 analysis 文件

use crate::daemon::DaemonState;
use crate::data_dir;
use crate::decompose::{CharacterDecomposer, PresetDecomposer};
use crate::error::AirpError;
use crate::orchestrator::card::{TavernCardV2, TavernPreset};
use crate::types::{CharacterId, PresetId};
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct DecomposeResponse {
    /// B2 修复：对齐 DecomposeResult，用 asset_id + asset_type
    pub asset_id: String,
    pub asset_type: String,
    pub files_written: Vec<String>,
    pub target_dir: String,
    pub lorebook_decomposed: bool,
}

/// `POST /v1/characters/:id/decompose`
pub async fn decompose_character(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> Result<Json<DecomposeResponse>, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    let card_text = data_dir::get_character(&state.data_root, &cid)?;
    let card: TavernCardV2 = serde_json::from_str(&card_text).map_err(|e| {
        AirpError::BadRequest(format!("card.json is not a valid TavernCardV2: {}", e))
    })?;

    let lb_path = data_dir::paths::char_world_lorebook_path(&state.data_root, cid.as_str());
    let lorebook = if lb_path.exists() {
        std::fs::read_to_string(&lb_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    } else {
        None
    };

    let analysis_dir = data_dir::paths::ensure_char_analysis_dir(&state.data_root, cid.as_str())?;
    // C3 修复：读 raw.json 提取 creator/character_version/tags 元信息
    let raw_meta = {
        let raw_path = state
            .data_root
            .join("characters")
            .join(cid.as_str())
            .join("card")
            .join("raw.json");
        if raw_path.exists() {
            std::fs::read_to_string(&raw_path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        } else {
            None
        }
    };
    let result = CharacterDecomposer::new()
        .decompose(cid.as_str(), &card, lorebook.as_ref(), &analysis_dir, raw_meta.as_ref())
        .await?;

    Ok(Json(DecomposeResponse {
        asset_id: result.asset_id,
        asset_type: result.asset_type,
        files_written: result.files_written,
        target_dir: result.target_dir,
        lorebook_decomposed: result.lorebook_decomposed,
    }))
}

/// `POST /v1/presets/:id/decompose`
pub async fn decompose_preset(
    State(state): State<Arc<DaemonState>>,
    Path(preset_id): Path<String>,
) -> Result<Json<DecomposeResponse>, AirpError> {
    let pid = PresetId::new(&preset_id)?;
    let preset_path = data_dir::paths::preset_json_path(&state.data_root, pid.as_str());
    if !preset_path.exists() {
        return Err(AirpError::NotFound(format!(
            "preset {} has no preset.json",
            pid
        )));
    }
    let preset_text = std::fs::read_to_string(&preset_path)?;
    let preset: TavernPreset =
        serde_json::from_str(&preset_text).map_err(|e| {
            AirpError::BadRequest(format!("preset.json is not a valid TavernPreset: {}", e))
        })?;

    let analysis_dir = data_dir::paths::ensure_preset_analysis_dir(&state.data_root, pid.as_str())?;
    let result = PresetDecomposer::new()
        .decompose(pid.as_str(), &preset, &analysis_dir)
        .await?;

    Ok(Json(DecomposeResponse {
        asset_id: result.asset_id,
        asset_type: result.asset_type,
        files_written: result.files_written,
        target_dir: result.target_dir,
        lorebook_decomposed: false,
    }))
}

/// `GET /v1/characters/:id/analysis` — 列出 analysis 目录下的 MD 文件。
pub async fn list_character_analysis(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    // E1：读端点用 char_analysis_dir_path，不创建目录
    let dir = data_dir::paths::char_analysis_dir_path(&state.data_root, cid.as_str())?;
    let files = list_md_files_recursive(&dir)?;
    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "files": files,
    })))
}

/// `GET /v1/characters/:id/analysis/*filename` — 读单个 analysis MD 文件。
pub async fn get_character_analysis_file(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, filename)): Path<(String, String)>,
) -> Result<impl IntoResponse, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    let path = data_dir::paths::char_analysis_file_path(&state.data_root, cid.as_str(), &filename)?;
    if !path.exists() {
        return Err(AirpError::NotFound(format!(
            "analysis file {} not found for character {}",
            filename, cid
        )));
    }
    let content = tokio::fs::read_to_string(&path).await?;
    Ok((
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        content,
    ))
}

/// `GET /v1/presets/:id/analysis` — 列出预设 analysis 文件。
pub async fn list_preset_analysis(
    State(state): State<Arc<DaemonState>>,
    Path(preset_id): Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let pid = PresetId::new(&preset_id)?;
    // E1：读端点用 preset_analysis_dir_path，不创建目录
    let dir = data_dir::paths::preset_analysis_dir_path(&state.data_root, pid.as_str())?;
    let files = list_md_files_recursive(&dir)?;
    Ok(Json(serde_json::json!({
        "preset_id": pid.as_str(),
        "files": files,
    })))
}

/// `GET /v1/presets/:id/analysis/*filename` — 读单个预设 analysis MD 文件。
pub async fn get_preset_analysis_file(
    State(state): State<Arc<DaemonState>>,
    Path((preset_id, filename)): Path<(String, String)>,
) -> Result<impl IntoResponse, AirpError> {
    let pid = PresetId::new(&preset_id)?;
    let path = data_dir::paths::preset_analysis_file_path(&state.data_root, pid.as_str(), &filename)?;
    if !path.exists() {
        return Err(AirpError::NotFound(format!(
            "analysis file {} not found for preset {}",
            filename, pid
        )));
    }
    let content = tokio::fs::read_to_string(&path).await?;
    Ok((
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        content,
    ))
}

/// 递归列出目录下所有 .md 文件，返回相对路径列表。
fn list_md_files_recursive(dir: &std::path::Path) -> Result<Vec<String>, AirpError> {
    let mut result = Vec::new();
    let base = dir;
    walk(dir, base, &mut result)?;
    result.sort();
    Ok(result)
}

fn walk(
    dir: &std::path::Path,
    base: &std::path::Path,
    out: &mut Vec<String>,
) -> Result<(), AirpError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let path = entry.path();
        if ft.is_dir() {
            walk(&path, base, out)?;
        } else if ft.is_file() {
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| AirpError::Internal(format!("strip_prefix failed: {}", e)))?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::card::{CharacterData, TavernCardV2};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::fs;
    use tower::ServiceExt;

    async fn setup_test_state() -> (tempfile::TempDir, Arc<DaemonState>) {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(DaemonState::for_test(tmp.path().to_path_buf()));
        (tmp, state)
    }

    fn write_sample_card(root: &std::path::Path) {
        let char_dir = root.join("characters").join("alice");
        let card_dir = char_dir.join("card");
        fs::create_dir_all(&card_dir).unwrap();
        let card = TavernCardV2 {
            spec: Some("chara_card_v2".into()),
            spec_version: Some("2.0".into()),
            data: CharacterData {
                name: Some("Alice".into()),
                description: Some("test desc".into()),
                personality: None,
                scenario: None,
                first_mes: Some("hi".into()),
                mes_template: None,
                system_prompt: None,
                mes_example: None,
                alternate_greetings: vec![],
                character_book: None,
            },
        };
        fs::write(
            card_dir.join("card.json"),
            serde_json::to_string(&card).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn http_decompose_character_returns_200_and_writes_files() {
        let (tmp, state) = setup_test_state().await;
        write_sample_card(tmp.path());

        let app = axum::Router::new()
            .route("/v1/characters/:id/decompose", axum::routing::post(decompose_character))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/characters/alice/decompose")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(tmp
            .path()
            .join("characters/alice/analysis/basic_info.md")
            .is_file());
    }

    #[tokio::test]
    async fn http_get_analysis_file_returns_markdown_content() {
        let (tmp, state) = setup_test_state().await;
        write_sample_card(tmp.path());

        // 先 decompose
        let app = axum::Router::new()
            .route("/v1/characters/:id/decompose", axum::routing::post(decompose_character))
            .route(
                "/v1/characters/:id/analysis/:filename",
                axum::routing::get(get_character_analysis_file),
            )
            .with_state(state.clone());

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/characters/alice/decompose")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/characters/alice/analysis/basic_info.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.starts_with("text/markdown"));
    }

    #[tokio::test]
    async fn http_get_analysis_file_rejects_traversal() {
        let (tmp, state) = setup_test_state().await;
        write_sample_card(tmp.path());

        let app = axum::Router::new()
            .route(
                "/v1/characters/:id/analysis/:filename",
                axum::routing::get(get_character_analysis_file),
            )
            .with_state(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/characters/alice/analysis/..%2fescape.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
```

- [ ] **Step 5.2: 在 daemon 模块注册路由**

在 `engine/src/daemon/mod.rs` 中：

a. 添加模块声明：
```rust
mod decompose_handlers;
pub use decompose_handlers::*;
```

b. 在 `v1_routes` 中添加路由（紧接 `.route("/v1/characters/:character_id/avatar", get(get_character_avatar_endpoint))` 之后）：

```rust
.route("/v1/characters/:character_id/decompose", post(decompose_character))
.route("/v1/characters/:character_id/analysis", get(list_character_analysis))
.route("/v1/characters/:character_id/analysis/*filename", get(get_character_analysis_file))
.route("/v1/presets/:preset_id/decompose", post(decompose_preset))
.route("/v1/presets/:preset_id/analysis", get(list_preset_analysis))
.route("/v1/presets/:preset_id/analysis/*filename", get(get_preset_analysis_file))
```

- [ ] **Step 5.3: 运行测试验证通过**

Run: `cargo test --lib -p airp-engine decompose_handlers::`
Expected: PASS（3 个测试全过）

- [ ] **Step 5.4: 跑全量回归**

Run: `cargo test --lib -p airp-engine`
Expected: 所有测试 PASS（包含神圣不变式 `subagent_context_has_no_orchestrator_noise`）

- [ ] **Step 5.5: 提交**

```bash
git add engine/src/daemon/decompose_handlers.rs engine/src/daemon/mod.rs
git commit -m "feat(daemon): 添加 decompose HTTP 端点（6个路由）+ analysis 文件读"
```

---

## Task 6: 实现 enhance_analysis agent 工具（diff 预览模式）

**Files:**
- Modify: `engine/src/agent/tools.rs`（添加 `EnhanceAnalysisTool`）
- Modify: `engine/src/daemon/mod.rs`（A3：暴露 adapter 给 enhance 工具）
- Test: `engine/src/agent/tools.rs` 的 `#[cfg(test)] mod tests`

**设计（A1 修订：diff 预览模式，不直接覆盖）**：
- `EnhanceAnalysisTool` 调用 LLM 生成填充后的 MD 内容，**不直接写盘**
- 返回 `{ original_md, enhanced_md, diff_hunks }` 供前端显示 diff 预览
- 用户在前端逐字段确认后，调用 Task 7 的 `apply` 端点写入
- 输入：`{ "character_id": string, "filename": string }`
- 输出：`{ "filename": string, "original_md": string, "enhanced_md": string, "has_changes": bool }`
- 守不变式6：只读 MD 骨架 + 已解析字段，不读 raw.json 原始大 blob

**A2 守则**：`EnhanceAnalysisTool` 拒绝 `filename` 以 `world_book/` 开头的请求，世界书条目不参与 enhance（对齐 issue #87）。

**A3 修订（adapter 调用方式）**：执行者必须先用 Grep 找到 `chat_completion` handler 中如何调 adapter 的实际代码路径。本计划不再写 `state.adapter`（DaemonState 可能没有该字段或字段名不同）。改用以下方式之一（执行者按仓内实际选其一）：
1. 若 `DaemonState` 有 `pub` 方法暴露 adapter（如 `state.adapter()` / `state.backend()`），直接用
2. 若 adapter 是 `DaemonState` 的 private 字段，在 `DaemonState` 上加 `pub(crate) fn adapter(&self) -> &Adapter` 访问器
3. 若有 `chat_completion` 的内部纯函数（如 `async fn do_chat_completion(state, messages) -> Result<String>`），直接复用

- [ ] **Step 6.1: 写失败测试**

在 `engine/src/agent/tools.rs` 末尾的 `#[cfg(test)] mod tests` 中添加：

```rust
#[tokio::test]
async fn enhance_analysis_tool_rejects_when_no_analysis_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let state = std::sync::Arc::new(
        crate::daemon::DaemonState::for_test(tmp.path().to_path_buf()),
    );
    let tool = EnhanceAnalysisTool { state: state.clone() };
    let result = tool
        .call(
            serde_json::json!({ "character_id": "ghost", "filename": "personality.md" }),
            false,
        )
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        AirpError::NotFound(_) => {}, // 期望
        other => panic!("expected NotFound, got {:?}", other),
    }
}
```

- [ ] **Step 6.2: 实现 EnhanceAnalysisTool（diff 预览模式）**

在 `engine/src/agent/tools.rs` 的 `DecomposeLorebookTool` 之后添加：

```rust
/// `enhance_analysis`：调 LLM 填充 MD 骨架中的 `<!-- Agent分析后填充 -->` 占位符。
///
/// **A1 修订（diff 预览模式）**：不直接覆盖原 MD。返回 original_md + enhanced_md，
/// 供前端显示 diff 预览。用户确认后通过 Task 7 的 `apply` 端点写入。
///
/// **A2 守则**：拒绝 `filename` 以 `world_book/` 开头的请求（世界书条目不参与 enhance）。
///
/// 输入：`{ "character_id": string, "filename": string }`
/// 输出：`{ "filename": string, "original_md": string, "enhanced_md": string, "has_changes": bool }`
///
/// 守不变式6：只读 MD 骨架 + 已解析的 card.json 结构化字段，不读 raw.json 原始 blob。
/// 调 LLM 通过控制平面 system prompt（"你是 RP 数据整理助手"），不进角色平面，守不变式①。
struct EnhanceAnalysisTool {
    state: Arc<DaemonState>,
}

impl Tool for EnhanceAnalysisTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "enhance_analysis",
            description: "Generate enhanced Markdown by filling placeholders (<!-- Agent分析后填充 -->) via LLM. Returns diff preview (original_md + enhanced_md); does NOT overwrite. User must call apply endpoint to commit. World book entries (world_book/*) are rejected.",
            side_effect: ToolSideEffect::Readonly,  // A1：改为只读，不写盘
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let cid_str = params
                .get("character_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing character_id".into()))?;
            let cid = CharacterId::new(cid_str)?;
            let filename = params
                .get("filename")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AirpError::BadRequest("missing filename".into()))?;

            // A2 守则：世界书条目不参与 enhance
            if filename.starts_with("world_book/") {
                return Err(AirpError::BadRequest(format!(
                    "world book entries ({}) are read-only and not eligible for enhance (issue #87)",
                    filename
                )));
            }

            // 1. 读 MD 骨架（必存在，否则提示先 decompose）
            let md_path = data_dir::paths::char_analysis_file_path(
                &state.data_root,
                cid.as_str(),
                filename,
            )?;
            if !md_path.exists() {
                return Err(AirpError::NotFound(format!(
                    "analysis file {} not found for character {}; run decompose_character first",
                    filename, cid
                )));
            }
            let md_skeleton = tokio::fs::read_to_string(&md_path).await?;

            // 若无占位符，直接返回（已 enhance 过）
            if !md_skeleton.contains("<!-- Agent分析后填充 -->") {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "character_id": cid.as_str(),
                        "filename": filename,
                        "original_md": md_skeleton,
                        "enhanced_md": md_skeleton,
                        "has_changes": false,
                        "reason": "no placeholders found (already enhanced?)",
                    }),
                    dry_run: true,  // 只读：标记为 dry_run
                });
            }

            // 2. 读已解析的 card.json 结构化字段（不读 raw.json 大 blob）
            let card_text = data_dir::get_character(&state.data_root, &cid)?;
            let card: crate::orchestrator::card::TavernCardV2 = serde_json::from_str(&card_text)
                .map_err(|e| {
                    AirpError::BadRequest(format!("card.json parse failed: {}", e))
                })?;

            // 3. 构造控制平面 system prompt（不进角色平面）
            let system_prompt = format!(
                r#"你是 RP 数据整理助手。任务：填充下方 Markdown 骨架中的 `<!-- Agent分析后填充 -->` 占位符。

规则：
1. 保留所有原始内容（标题、字段值），只替换占位符注释为实际分析内容
2. 不要新增或删除标题层级（# / ## / ###）
3. 输出完整的 Markdown 文档（包含未修改部分）
4. 分析应基于角色卡已有字段，不要编造设定
5. 用中文输出

角色卡已解析字段（JSON）：
```json
{}
```

待填充的 Markdown 骨架：
```markdown
{}
```

请输出完整的填充后 Markdown 文档（不要包裹 ```markdown 代码块，直接输出内容）。"#,
                serde_json::to_string_pretty(&card).unwrap_or_else(|_| "{}".into()),
                md_skeleton,
            );

            // 4. 调 LLM 生成增强内容（A3：adapter 调用方式按仓内实际）
            let enhanced_md = call_llm_for_enhance(&state, &system_prompt).await?;

            // A1：不写盘，返回 diff 预览
            let has_changes = enhanced_md != md_skeleton;
            Ok(ToolResult {
                output: serde_json::json!({
                    "character_id": cid.as_str(),
                    "filename": filename,
                    "original_md": md_skeleton,
                    "enhanced_md": enhanced_md,
                    "has_changes": has_changes,
                }),
                dry_run: true,  // 只读：等待用户 apply
            })
        })
    }
}

/// 调 LLM 增强 MD（控制平面调用，不进角色平面）。
///
/// **A3 修订**：执行者必须先 Grep 找到 `chat_completion` handler 中如何调 adapter 的实际代码路径。
/// 以下为参考实现，假设 `DaemonState` 暴露了 `adapter()` 访问器。若仓内实际不同，按以下三种方式之一适配：
/// 1. 若 DaemonState 有 pub 方法暴露 adapter（如 state.adapter() / state.backend()），直接用
/// 2. 若 adapter 是 private 字段，在 DaemonState 上加 pub(crate) fn adapter(&self) -> &Adapter
/// 3. 若有 chat_completion 的内部纯函数，直接复用
async fn call_llm_for_enhance(
    state: &DaemonState,
    system_prompt: &str,
) -> Result<String, AirpError> {
    use crate::adapter::{ChatMessage, MessageRole};

    let messages = vec![
        ChatMessage {
            role: MessageRole::System,
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: MessageRole::User,
            content: "请输出填充后的完整 Markdown 文档。".into(),
        },
    ];

    let settings = state.settings.clone();
    let model = settings
        .model
        .clone()
        .unwrap_or_else(|| "gpt-4o-mini".into());

    // A3：执行者按仓内实际 adapter 调用方式调整以下代码。
    // 参考：Grep "stream_chat" 或 "chat_completion" 在 engine/src/daemon/handlers.rs 中的用法。
    // 假设 DaemonState 有 pub(crate) fn adapter(&self) -> &Adapter：
    let mut stream = state
        .adapter()  // A3：用访问器，不用字段直访
        .stream_chat(&model, &messages, None, &settings)
        .await?;

    let mut full = String::new();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        if let Some(delta) = chunk? {
            full.push_str(&delta);
        }
    }
    Ok(full)
}
```

> **A3 执行者必读**：在实现本步骤前，必须先执行：
> ```bash
> # 找到 chat_completion handler 中调 adapter 的实际代码
> grep -n "stream_chat\|chat_completion\|adapter" engine/src/daemon/handlers.rs
> grep -n "adapter" engine/src/daemon/mod.rs
> ```
> 根据实际找到的方式调整 `call_llm_for_enhance`。若 `DaemonState` 没有 adapter 访问器，在 `mod.rs` 加 `pub(crate) fn adapter(&self) -> &Adapter { &self.adapter }`（或实际字段名）。**不要** 留 placeholder。

- [ ] **Step 6.3: 在工具注册表中注册**

在 `engine/src/agent/tools.rs` 的注册函数中添加：

```rust
registry.register(Box::new(EnhanceAnalysisTool { state: state.clone() }))?;
```

- [ ] **Step 6.4: 跑测试**

Run: `cargo test --lib -p airp-engine tools::tests::enhance`
Expected: PASS（注意：此测试只验证"无 analysis 时返回 NotFound"，不真调 LLM；真调 LLM 的集成测试需 mock adapter，超本计划范围）

- [ ] **Step 6.5: 提交**

```bash
git add engine/src/agent/tools.rs
git commit -m "feat(agent): 实现 enhance_analysis 工具（调 LLM 填充 MD 占位符）"
```

---

## Task 7: 添加 enhance HTTP 端点 + apply 确认端点（A1）

**Files:**
- Modify: `engine/src/daemon/decompose_handlers.rs`（添加 `enhance_character_analysis` + `apply_enhanced_analysis` handler）
- Modify: `engine/src/daemon/mod.rs`（注册路由 + B3 body limit）
- Test: `engine/src/daemon/decompose_handlers.rs` 的 `#[cfg(test)] mod tests`

**A1 设计**：enhance 端点只读返回 diff 预览；apply 端点接收用户确认的 `enhanced_md` 写入。

- [ ] **Step 7.1: 写失败测试**

在 `engine/src/daemon/decompose_handlers.rs` 的 `#[cfg(test)] mod tests` 末尾添加：

```rust
#[tokio::test]
async fn http_enhance_character_returns_404_when_no_analysis() {
    let (tmp, state) = setup_test_state().await;
    write_sample_card(tmp.path());

    let app = axum::Router::new()
        .route(
            "/v1/characters/:id/analysis/:filename/enhance",
            axum::routing::post(enhance_character_analysis),
        )
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/characters/alice/analysis/personality.md/enhance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // analysis 文件不存在 → 404
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_enhance_rejects_world_book_entries() {
    // A2：世界书条目拒绝 enhance
    let (tmp, state) = setup_test_state().await;
    write_sample_card(tmp.path());

    let app = axum::Router::new()
        .route(
            "/v1/characters/:id/analysis/:filename/enhance",
            axum::routing::post(enhance_character_analysis),
        )
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/characters/alice/analysis/world_book%2Fentry_001.md/enhance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_apply_writes_enhanced_md() {
    // A1：apply 端点接收 enhanced_md 并写入
    let (tmp, state) = setup_test_state().await;
    write_sample_card(tmp.path());

    // 先 decompose
    let analysis_dir = tmp.path().join("characters/alice/analysis");
    std::fs::create_dir_all(&analysis_dir).unwrap();
    std::fs::write(
        analysis_dir.join("personality.md"),
        "# 性格\n<!-- Agent分析后填充 -->\n",
    ).unwrap();

    let app = axum::Router::new()
        .route(
            "/v1/characters/:id/analysis/:filename/apply",
            axum::routing::post(apply_enhanced_analysis),
        )
        .with_state(state);

    let enhanced = "# 性格\n温柔内敛\n".to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/characters/alice/analysis/personality.md/apply")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "enhanced_md": enhanced }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let written = std::fs::read_to_string(analysis_dir.join("personality.md")).unwrap();
    assert_eq!(written, enhanced);
}
```

- [ ] **Step 7.2: 实现 enhance_character_analysis + apply_enhanced_analysis handler**

在 `engine/src/daemon/decompose_handlers.rs` 末尾、`#[cfg(test)] mod tests` 之前添加：

```rust
/// `POST /v1/characters/:id/analysis/:filename/enhance`
///
/// A1：只读端点，返回 diff 预览（original_md + enhanced_md）。不写盘。
/// 用户确认后调用 `apply` 端点写入。
pub async fn enhance_character_analysis(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, filename)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    // A2 守则：EnhanceAnalysisTool 内部已拒绝 world_book/* 路径
    let tool = crate::agent::tools::EnhanceAnalysisTool {
        state: state.clone(),
    };
    let result = tool
        .call(
            serde_json::json!({
                "character_id": cid.as_str(),
                "filename": filename,
            }),
            false,
        )
        .await?;

    Ok(Json(result.output))
}

/// `POST /v1/characters/:id/analysis/:filename/apply`
///
/// A1：用户确认 diff 后调此端点写入 enhanced_md。
/// body: `{ "enhanced_md": string }`
pub async fn apply_enhanced_analysis(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, filename)): Path<(String, String)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(&character_id)?;

    // A2 守则：世界书条目不允许 apply
    if filename.starts_with("world_book/") {
        return Err(AirpError::BadRequest(format!(
            "world book entries ({}) are read-only (issue #87)",
            filename
        )));
    }

    let enhanced_md = body
        .get("enhanced_md")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AirpError::BadRequest("missing enhanced_md field".into()))?;

    let path = data_dir::paths::char_analysis_file_path(
        &state.data_root,
        cid.as_str(),
        &filename,
    )?;
    if !path.exists() {
        return Err(AirpError::NotFound(format!(
            "analysis file {} not found for character {}",
            filename, cid
        )));
    }

    let size_before = tokio::fs::read_to_string(&path).await?.len();
    tokio::fs::write(&path, enhanced_md).await?;
    let size_after = enhanced_md.len();

    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "filename": filename,
        "applied": true,
        "size_before": size_before,
        "size_after": size_after,
    })))
}
```

- [ ] **Step 7.3: 把 EnhanceAnalysisTool 暴露为 pub**

在 `engine/src/agent/tools.rs` 中，把 `EnhanceAnalysisTool` 结构体声明从默认 private 改为 `pub`：

```rust
// 原：struct EnhanceAnalysisTool { ... }
// 改为：
pub struct EnhanceAnalysisTool {
    pub state: Arc<DaemonState>,
}
```

- [ ] **Step 7.4: 在 daemon mod.rs 注册路由 + B3 body limit**

在 `engine/src/daemon/mod.rs` 的 v1_routes 中，紧接 analysis 读路由之后添加：

```rust
// A1：enhance（只读返回 diff）+ apply（确认写入）
.route(
    "/v1/characters/:character_id/analysis/:filename/enhance",
    post(enhance_character_analysis),
)
.route(
    "/v1/characters/:character_id/analysis/:filename/apply",
    post(apply_enhanced_analysis),
)
```

**B3 修复**：所有新 POST 端点（decompose / enhance / apply）加 body limit。在路由链上添加 `DefaultBodyLimit`：

```rust
use axum::body::DefaultBodyLimit;

// 在 v1_routes 的 Router 链末尾添加 .layer(DefaultBodyLimit::max(1024 * 1024))
// 1MB 足够：decompose body 为空，enhance body 为空，apply body 含 enhanced_md（通常 < 100KB）
.route("/v1/characters/:character_id/decompose", post(decompose_character))
.route("/v1/characters/:character_id/analysis", get(list_character_analysis))
.route("/v1/characters/:character_id/analysis/*filename", get(get_character_analysis_file))
.route("/v1/characters/:character_id/analysis/:filename/enhance", post(enhance_character_analysis))
.route("/v1/characters/:character_id/analysis/:filename/apply", post(apply_enhanced_analysis))
.route("/v1/presets/:preset_id/decompose", post(decompose_preset))
.route("/v1/presets/:preset_id/analysis", get(list_preset_analysis))
.route("/v1/presets/:preset_id/analysis/*filename", get(get_preset_analysis_file))
.layer(DefaultBodyLimit::max(1024 * 1024))  // B3：1MB body limit
```

- [ ] **Step 7.5: 跑测试**

Run: `cargo test --lib -p airp-engine decompose_handlers::`
Expected: PASS（3 个新测试 + 原有测试）

- [ ] **Step 7.6: 提交**

```bash
git add engine/src/daemon/decompose_handlers.rs engine/src/daemon/mod.rs engine/src/agent/tools.rs
git commit -m "feat(daemon): 添加 enhance + apply HTTP 端点（A1 diff 预览模式）+ B3 body limit"
```

---

## Task 8: WebUI 工作台集成

**Files:**
- Modify: `airp-engine-console/pages/workbench.html`（添加 decompose / enhance UI）
- Modify: `airp-engine-console/pages/characters.html`（导入后引导链接）

- [ ] **Step 8.1: 在 workbench.html 添加 decompose 区块**

在 `airp-engine-console/pages/workbench.html` 找一个合适位置（如现有 workbench entries 之后），添加以下 HTML：

```html
<!-- ── Decompose / Enhance 区块 ── -->
<section class="border-t" style="border-color: var(--color-border-default);">
    <div class="px-4 py-3">
        <div class="flex items-center justify-between mb-3">
            <span class="text-xs font-semibold" style="color: var(--color-text-primary); font-size: 13px;">资产拆解</span>
            <button id="btn-refresh-analysis" class="text-xs font-medium transition-colors duration-150 hover:opacity-80" style="color: var(--color-primary); font-size: 12px;">
                刷新
            </button>
        </div>

        <div class="flex gap-2 mb-3">
            <button id="btn-decompose-character" class="flex-1 h-8 rounded-md text-xs font-semibold whitespace-nowrap transition-colors duration-150" style="background: var(--color-primary); color: var(--color-text-inverse); font-size: 12px;">
                拆解角色卡
            </button>
            <button id="btn-decompose-preset" class="flex-1 h-8 rounded-md text-xs font-semibold whitespace-nowrap border transition-colors duration-150" style="border-color: var(--color-border-default); color: var(--color-text-secondary); font-size: 12px;">
                拆解预设
            </button>
        </div>

        <div id="analysis-files-list" class="space-y-1 mb-3 max-h-48 overflow-y-auto">
            <div class="text-xs text-center py-4" style="color: var(--color-text-tertiary); font-size: 11px;">
                点击「拆解角色卡」生成 Markdown 分析文件
            </div>
        </div>

        <div id="analysis-md-viewer" class="hidden">
            <div class="flex items-center justify-between mb-2">
                <span id="analysis-md-filename" class="text-xs font-mono" style="color: var(--color-text-secondary); font-size: 11px;"></span>
                <button id="btn-enhance-md" class="h-6 px-2 rounded text-xs font-medium whitespace-nowrap transition-colors duration-150" style="background: var(--color-primary-muted); color: var(--color-primary); font-size: 11px;">
                    Agent 增强
                </button>
            </div>
            <pre id="analysis-md-content" class="text-xs overflow-auto rounded-md p-3 leading-normal whitespace-pre-wrap" style="font-family: var(--font-mono); font-size: 11px; background: var(--color-bg-base); color: var(--color-text-primary); border: 1px solid var(--color-border-subtle); max-height: 400px;"></pre>
        </div>
    </div>
</section>
```

- [ ] **Step 8.2: 添加 JS 逻辑（A4 安全 DOM 操作 + A1 diff 预览 + B4 重复 decompose 确认）**

在 `airp-engine-console/pages/workbench.html` 的 `<script>` 块中（或现有 JS 文件中）添加：

```javascript
// ── Decompose / Enhance 逻辑 ──
// A4：所有外部数据（文件名、MD 内容）用 textContent / createElement 渲染，禁止 innerHTML 拼接
const ANALYSIS_BASE = '/v1/characters';
const charId = new URLSearchParams(location.search).get('character_id') || 'linwanqing';

// A4 辅助：清空容器并填入纯文本提示
function setListMessage(list, text, isError = false) {
    list.replaceChildren();
    const div = document.createElement('div');
    div.className = 'text-xs text-center py-4';
    div.style.cssText = isError
        ? 'color: var(--state-error); font-size: 11px;'
        : 'color: var(--color-text-tertiary); font-size: 11px;';
    div.textContent = text;
    list.appendChild(div);
}

async function refreshAnalysisFiles() {
    const list = document.getElementById('analysis-files-list');
    try {
        const res = await fetch(`${ANALYSIS_BASE}/${encodeURIComponent(charId)}/analysis`);
        if (!res.ok) {
            setListMessage(list, `加载失败: ${res.status}`, true);
            return;
        }
        const data = await res.json();
        const files = data.files || [];
        if (files.length === 0) {
            setListMessage(list, '点击「拆解角色卡」生成 Markdown 分析文件');
            return;
        }
        list.replaceChildren();
        for (const f of files) {
            const item = document.createElement('div');
            item.className = 'rounded-md px-2 py-1.5 cursor-pointer hover:bg-[var(--color-primary-subtle)] transition-colors duration-150';
            item.dataset.filename = f;  // A4：data attr 安全，浏览器自动转义
            const inner = document.createElement('div');
            inner.className = 'text-xs truncate';
            inner.style.cssText = 'color: var(--color-text-primary); font-size: 11px; font-family: var(--font-mono);';
            inner.textContent = f;  // A4：textContent，非 innerHTML
            item.appendChild(inner);
            item.addEventListener('click', () => loadAnalysisMd(f));
            list.appendChild(item);
        }
    } catch (e) {
        setListMessage(list, e.message, true);
    }
}

async function loadAnalysisMd(filename) {
    const viewer = document.getElementById('analysis-md-viewer');
    const contentEl = document.getElementById('analysis-md-content');
    const filenameEl = document.getElementById('analysis-md-filename');
    filenameEl.textContent = filename;
    contentEl.textContent = '加载中…';
    viewer.classList.remove('hidden');
    try {
        const res = await fetch(`${ANALYSIS_BASE}/${encodeURIComponent(charId)}/analysis/${encodeURIComponent(filename)}`);
        if (!res.ok) {
            contentEl.textContent = `加载失败: ${res.status}`;
            return;
        }
        contentEl.textContent = await res.text();  // A4：textContent 安全
    } catch (e) {
        contentEl.textContent = e.message;
    }
}

async function decomposeCharacter() {
    const btn = document.getElementById('btn-decompose-character');
    btn.disabled = true;
    btn.textContent = '拆解中…';
    try {
        // B4：检测已有 analysis 文件，提示用户确认覆盖
        const listRes = await fetch(`${ANALYSIS_BASE}/${encodeURIComponent(charId)}/analysis`);
        let hasExisting = false;
        if (listRes.ok) {
            const listData = await listRes.json();
            hasExisting = (listData.files || []).length > 0;
        }
        if (hasExisting && !confirm('已存在 analysis 文件，重新拆解将覆盖（已 enhance 的内容会丢失），是否继续？')) {
            return;
        }

        const res = await fetch(`${ANALYSIS_BASE}/${encodeURIComponent(charId)}/decompose?force=true`, { method: 'POST' });
        if (!res.ok) {
            const err = await res.text();
            alert(`拆解失败: ${err}`);
            return;
        }
        const data = await res.json();
        alert(`拆解完成，生成 ${data.files_written.length} 份 MD 文件`);
        refreshAnalysisFiles();
    } catch (e) {
        alert(`拆解失败: ${e.message}`);
    } finally {
        btn.disabled = false;
        btn.textContent = '拆解角色卡';
    }
}

// A1：enhance 改为 diff 预览模式，用户确认后调 apply
async function enhanceCurrentMd() {
    const filenameEl = document.getElementById('analysis-md-filename');
    const filename = filenameEl.textContent;
    if (!filename) return;
    const btn = document.getElementById('btn-enhance-md');
    btn.disabled = true;
    btn.textContent = '增强中…';
    try {
        const res = await fetch(`${ANALYSIS_BASE}/${encodeURIComponent(charId)}/analysis/${encodeURIComponent(filename)}/enhance`, { method: 'POST' });
        if (!res.ok) {
            const err = await res.text();
            alert(`增强失败: ${err}`);
            return;
        }
        const data = await res.json();
        if (!data.has_changes) {
            alert(`未增强：${data.reason || '无变化'}`);
            return;
        }
        // A1：显示 diff 预览，用户逐字段确认
        const accept = confirm(`增强预览（${filename}）：\n\n原内容（前 200 字）：\n${(data.original_md || '').slice(0, 200)}\n\n增强后（前 200 字）：\n${(data.enhanced_md || '').slice(0, 200)}\n\n是否接受并写入？`);
        if (!accept) return;
        // A1：用户确认后调 apply 端点
        const applyRes = await fetch(`${ANALYSIS_BASE}/${encodeURIComponent(charId)}/analysis/${encodeURIComponent(filename)}/apply`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ enhanced_md: data.enhanced_md }),
        });
        if (!applyRes.ok) {
            const err = await applyRes.text();
            alert(`写入失败: ${err}`);
            return;
        }
        const applyData = await applyRes.json();
        alert(`已写入：${applyData.size_before} → ${applyData.size_after} 字节`);
        loadAnalysisMd(filename);
    } catch (e) {
        alert(`增强失败: ${e.message}`);
    } finally {
        btn.disabled = false;
        btn.textContent = 'Agent 增强';
    }
}

document.getElementById('btn-refresh-analysis')?.addEventListener('click', refreshAnalysisFiles);
document.getElementById('btn-decompose-character')?.addEventListener('click', decomposeCharacter);
document.getElementById('btn-enhance-md')?.addEventListener('click', enhanceCurrentMd);

// 初次加载时刷新 analysis 列表
refreshAnalysisFiles();
```

- [ ] **Step 8.3: 在 characters.html 导入后引导**

在 `airp-engine-console/pages/characters.html` 中，找到角色卡导入成功的反馈处（如导入按钮的 success handler），添加引导链接：

```javascript
// 导入成功后显示引导
function showDecomposeGuide(characterId) {
    const guide = document.createElement('div');
    guide.className = 'fixed bottom-4 right-4 rounded-md shadow-lg p-3 max-w-sm';
    guide.style.cssText = 'background: var(--color-bg-elevated); border: 1px solid var(--color-primary); color: var(--color-text-primary);';
    guide.innerHTML = `
        <div class="text-xs font-semibold mb-1" style="font-size: 12px;">导入成功</div>
        <div class="text-xs mb-2" style="font-size: 11px; color: var(--color-text-secondary);">
            角色 ${characterId} 已导入。前往工作台拆解为 Markdown 分析文档？
        </div>
        <div class="flex gap-2">
            <a href="./workbench.html?character_id=${encodeURIComponent(characterId)}" class="text-xs font-semibold px-2 py-1 rounded" style="background: var(--color-primary); color: var(--color-text-inverse); font-size: 11px;">
                前往工作台
            </a>
            <button class="text-xs px-2 py-1 rounded" style="color: var(--color-text-tertiary); font-size: 11px;" onclick="this.parentElement.parentElement.remove()">
                稍后
            </button>
        </div>
    `;
    document.body.appendChild(guide);
    setTimeout(() => guide.remove(), 30000);  // 30秒后自动消失
}
```

> **注意**：执行者需找到现有导入成功 handler 的位置，调用 `showDecomposeGuide(characterId)`。如果现有代码没有 success callback，在 fetch 导入端点后加。

- [ ] **Step 8.4: 手动验证**

启动 engine daemon，访问 `airp-engine-console/pages/workbench.html?character_id=linwanqing`：
1. 点击"拆解角色卡" → 应看到 alert 提示生成 7 份 MD
2. 左侧文件列表显示 7 个 .md 文件
3. 点击任一文件 → 右侧显示 MD 内容（含 `<!-- Agent分析后填充 -->` 占位符）
4. 点击"Agent 增强" → 调 LLM 填充（需 engine 已配置 LLM endpoint）
5. 增强后 MD 内容更新，占位符被替换

- [ ] **Step 8.5: 提交**

```bash
git add airp-engine-console/pages/workbench.html airp-engine-console/pages/characters.html
git commit -m "feat(webui): 工作台添加拆解/增强 UI + 导入后引导"
```

---

## Task 9: 全量回归 + 端到端集成测试

**Files:**
- Create: `engine/tests/decompose_integration.rs`
- Run: 全量 cargo test

- [ ] **Step 9.1: 创建集成测试**

创建 `engine/tests/decompose_integration.rs`：

```rust
//! 端到端集成测试：完整 daemon HTTP + 文件落盘验证。

use airp_engine::daemon::DaemonState;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::fs;
use std::sync::Arc;
use tower::ServiceExt;

async fn setup() -> (tempfile::TempDir, axum::Router) {
    let tmp = tempfile::tempdir().unwrap();
    let state = Arc::new(DaemonState::for_test(tmp.path().to_path_buf()));

    // 写一个 sample 角色
    let char_dir = tmp.path().join("characters").join("alice");
    let card_dir = char_dir.join("card");
    fs::create_dir_all(&card_dir).unwrap();
    fs::write(
        card_dir.join("card.json"),
        r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"Alice","description":"test","first_mes":"hi"}}"#,
    )
    .unwrap();

    let app = axum::Router::new()
        .route("/v1/characters/:id/decompose", axum::routing::post(airp_engine::daemon::decompose_character))
        .route("/v1/characters/:id/analysis", axum::routing::get(airp_engine::daemon::list_character_analysis))
        .route(
            "/v1/characters/:id/analysis/*filename",
            axum::routing::get(airp_engine::daemon::get_character_analysis_file),
        )
        .with_state(state);

    (tmp, app)
}

#[tokio::test]
async fn e2e_decompose_then_list_then_read() {
    let (tmp, app) = setup().await;

    // 1. POST decompose
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/characters/alice/decompose")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. GET analysis list
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/characters/alice/analysis")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 7);
    assert!(files.iter().any(|f| f.as_str() == Some("basic_info.md")));

    // 3. GET single file
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/characters/alice/analysis/basic_info.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let content = String::from_utf8(body.to_vec()).unwrap();
    assert!(content.contains("Alice"));
    assert!(content.contains("基础信息"));

    // 4. 文件确实落盘
    assert!(tmp
        .path()
        .join("characters/alice/analysis/basic_info.md")
        .is_file());
}

#[tokio::test]
async fn e2e_traversal_blocked() {
    let (_tmp, app) = setup().await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/characters/alice/analysis/..%2Fcard.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 9.2: 把 DaemonState 和 handlers 暴露给集成测试**

在 `engine/src/lib.rs` 中确保：
```rust
pub mod daemon;  // 已是 pub
```

在 `engine/src/daemon/mod.rs` 中确保 `decompose_handlers` 模块和 `DaemonState` 是 `pub`：
```rust
pub mod decompose_handlers;
pub use decompose_handlers::*;
```

`DaemonState` 已是 `pub`（外部测试需要 `for_test`）。如果 `for_test` 是 `#[cfg(test)]`，需改为 `pub fn for_test`（不带 cfg），或加 `#[cfg(any(test, feature = "test-utils"))]` + 在 Cargo.toml 加 `[features] test-utils = []`。

> **简化方案**：直接把 `for_test` 改为不带 `#[cfg(test)]` 的 `pub fn`，仅供测试用，但允许集成测试调用。或者用 `#[doc(hidden)] pub fn for_test(...)`。

- [ ] **Step 9.3: 跑全量回归**

```powershell
$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
cargo test --lib -p airp-engine
cargo test --test decompose_integration -p airp-engine
```

Expected: 全部 PASS，包含：
- 神圣不变式 `subagent_context_has_no_orchestrator_noise`
- 现有 365+ 测试不回归
- 新增 decompose / enhance / HTTP 测试全过

- [ ] **Step 9.4: 跑 security / markdown 测试**

```powershell
node target/test-serve-security.js
node target/test-md-v2.js
```

Expected: 全部 PASS（与 main 一致）

- [ ] **Step 9.5: 提交**

```bash
git add engine/tests/decompose_integration.rs engine/src/daemon/mod.rs engine/src/lib.rs
git commit -m "test(engine): 添加 decompose 端到端集成测试 + 暴露 DaemonState 测试构造器"
```

---

## Self-Review Checklist

执行完所有任务后，自检：

- [ ] **Spec coverage**：MCP-SERVER-ABSORPTION.md §1 列出的 `decompose_character` / `decompose_preset` / `analyze_card` 是否都有对应实现？
  - `decompose_character` ✅ Task 2 + Task 4
  - `decompose_preset` ✅ Task 3 + Task 4
  - `decompose_lorebook` ✅ Task 2（内嵌）+ Task 4（独立工具）
  - `analyze_card` = `enhance_analysis` 的前段（读已解析字段）✅ Task 6
  - `enhance_analysis` ✅ Task 6 + Task 7
- [ ] **ASSET-SPEC 守则**：导入主干是否仍是代码归一化？agent 是否旁路？✅ decompose 在用户手动触发，不在导入路径；enhance 是 agent 旁路 sidecar
- [ ] **不变式6**：decompose 阶段零 LLM 调用？enhance 只读 MD 骨架 + 已解析字段，不读 raw.json？✅
- [ ] **不变式①**：MD 产物只含 RP 数据 + 占位符，零 agent 脚手架？enhance 的 LLM 调用走控制平面 system prompt？✅
- [ ] **路径沙箱**：HTTP 端点读 analysis 文件有白名单校验？✅ Task 1 + Task 5 测试覆盖路径穿越
- [ ] **未来 Tauri UI 复用**：HTTP 端点是否独立于 WebUI？✅ 所有功能都通过 HTTP 暴露，Tauri 可直接调

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-07-07-decompose-agent-flow.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
