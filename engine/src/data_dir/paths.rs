use crate::error::AirpError;
use crate::types::CharacterId;
use std::fs;
use std::path::{Path, PathBuf};

/// 解析引擎数据根目录。三层 fallback，优先级清晰：
///
/// 1. `AIRP_DATA_DIR` 环境变量 —— 用户显式指定（最高优先，所有场景适用）。
///    空串或仅空白视为未设置，防止下游路径拼接出错。
/// 2. `cwd/data` —— **开发者场景**：debug 编译，或运行于 Cargo 环境下，且 cwd 含
///    `Cargo.toml`（即从 repo 根 `cargo run`）。数据落 repo 内，删 repo = 卸载，
///    复制 repo = 迁移。符合 clone 后产物收口诉求。
///    release 二进制在任意含 `Cargo.toml` 的目录下双击时，不会误判为开发模式。
/// 3. `dirs::data_dir().join("airp")` —— **打包 .exe 双击场景**：cwd 不在 repo 根
///    （如 `Program Files` 的 UAC 拒写、或用户从任意目录双击）时，落 OS 标准 per-user
///    位（Win `%APPDATA%\airp\`，macOS `~/Library/Application Support/airp/`，
///    Linux `~/.local/share/airp/`），per-user 隔离、重装不丢、不污染 Program Files。
/// 4. 兜底 `cwd/data` —— `dirs` 取不到（极罕见，某些容器化环境）。
///
/// 旧实现仅「cwd/data」相对 cwd，双击 .exe 时 cwd 漂到安装目录致写失败或数据共享。
pub fn resolve_data_root() -> PathBuf {
    resolve_data_root_inner(
        std::env::var("AIRP_DATA_DIR").ok().as_deref(),
        cfg!(debug_assertions),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        &PathBuf::from("Cargo.toml"),
    )
}

/// `resolve_data_root` 的纯函数内核 —— 把 env / 编译态 / cwd 这些全局依赖
/// 参数化后，单元测试可直接覆盖每条 fallback 与边界条件（空 env var、whitespace、
/// release 误入 dev 模式、dirs 不可用等），无需求助 `serial_test` 锁 env 或
/// 改 cwd。Kimi-K2.7-Code 的 in-place 修法（review #1/#2）虽然正确但不可测，
/// 这一层抽出来正好补回测试覆盖。
fn resolve_data_root_inner(
    env_value: Option<&str>,
    is_debug: bool,
    under_cargo: bool,
    cargo_toml_path: &Path,
) -> PathBuf {
    if let Some(custom) = env_value {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    let is_dev = is_debug || under_cargo;
    if is_dev && cargo_toml_path.exists() {
        return PathBuf::from("data");
    }
    if let Some(per_user) = dirs::data_dir() {
        return per_user.join("airp");
    }
    PathBuf::from("data")
}

pub fn ensure_data_dirs(root: &Path) -> Result<(), AirpError> {
    let dirs = [
        root.to_path_buf(),
        root.join("characters"),
        root.join("styles"),
        root.join("styles").join("profiles"),
        root.join("presets"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            fs::create_dir_all(dir)?;
        }
    }

    let settings_path = root.join("settings.json");
    if !settings_path.exists() {
        let default_settings = serde_json::json!({
            "provider": "OpenAI",
            "endpoint": "",
            "api_key": "",
            "model": "gpt-4o",
            "daemon_port": 8000,
            "default_user_name": "User",
            "default_filters": []
        });
        let content = serde_json::to_string_pretty(&default_settings)?;
        fs::write(&settings_path, content)?;
    }

    let default_style = root.join("styles").join("profiles").join("default.md");
    if !default_style.exists() {
        let style_content = r#"# Default Narrative Style

## Tone
Warm, immersive, literary fiction tone with balanced pacing.

## Sentence Patterns
- Mix of short and medium-length sentences
- Moderate use of sensory detail
- Natural dialogue with character voice variation

## Vocabulary
- Prefer concrete, vivid language over abstract generalizations
- Avoid clinical or overly formal vocabulary in narrative passages

## Paragraph Structure
- 2-4 sentences per paragraph in action scenes
- Longer descriptive paragraphs for atmosphere building

## Pacing
- Vary rhythm between tension and release
- Allow quiet moments between action beats
"#;
        fs::write(&default_style, style_content)?;
    }

    let world_path = root.join("world.md");
    if !world_path.exists() {
        let content = r#"# 世界观与场景状态 world

## 区域与场景
| 区域名称 | 当前状态 | 描述 | 在场NPC |
| :--- | :--- | :--- | :--- |
| 起始基地 | 安全 | 弥漫着微雾的钢铁甲板 | Emily, Companion |
| 码头 | 锁闭 | 停靠着老旧巡逻艇的栈桥 | 无 |

## 势力关系
- 玩家 - 基地防卫队: 友善 (50/100)
- 玩家 - 神秘组织: 敌对 (0/100)
"#;
        fs::write(&world_path, content)?;
    }

    let items_path = root.join("items.md");
    if !items_path.exists() {
        let content = r#"# 物品追踪清单 items

| 物品名称 | 持有者 | 状态/位置 | 详细描述 |
| :--- | :--- | :--- | :--- |
| 神秘钥匙 | 基地保险箱 | 起始基地办公室 | 一把沾满锈迹、刻有古老花纹的黄铜钥匙 |
| 战术手电 | 玩家 | 随身携带 | 强光军用手电，电量充足 |
"#;
        fs::write(&items_path, content)?;
    }

    if let Err(e) = super::migrations::migrate_legacy_presets(root) {
        tracing::warn!(err = %e, "M_PR: 预设迁移部分失败");
    }

    let _ = crate::auto_converter::auto_convert_legacy_files(root);

    if let Err(e) = super::migrations::migrate_legacy_char_dirs(root) {
        tracing::warn!(err = %e, "CF-6: 角色目录迁移部分失败");
    }

    Ok(())
}

pub fn character_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("characters").join(character_id);
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }

    let sub_dirs = ["worldbooks", "memory"];
    for sub in &sub_dirs {
        let sub_path = dir.join(sub);
        if !sub_path.exists() {
            fs::create_dir_all(&sub_path)?;
        }
    }

    let _ = char_gating_dir(root, character_id)?;

    Ok(dir)
}

const CHECKPOINTS_TEMPLATE: &str = r#"# 剧情关卡 checkpoints (CP)

## 当前进度
- 当前关卡: CP-1
- 进度百分比: 0%

## 关卡清单
- [ ] CP-1: 探索期。
- [ ] CP-2: 对峙期。
- [ ] CP-3: 决战期。
"#;

const TIMELINE_TEMPLATE: &str = r#"# 时间线与时槽追踪 timeline

## 统计数据
- 累计消耗时槽: 0

## 历史事件日志
"#;

pub(crate) fn char_card_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("characters").join(character_id).join("card");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn char_greetings_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root
        .join("characters")
        .join(character_id)
        .join("card")
        .join("greetings");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn char_world_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("characters").join(character_id).join("world");
    fs::create_dir_all(&dir)?;
    let extra = dir.join("extra");
    if !extra.exists() {
        fs::create_dir_all(&extra)?;
    }
    Ok(dir)
}

pub(crate) fn char_world_lorebook_path(root: &Path, character_id: &str) -> PathBuf {
    root.join("characters")
        .join(character_id)
        .join("world")
        .join("lorebook.json")
}

// ── analysis sidecar 路径函数（decompose 产物） ──────────────────────────────
//
// E1 修复（G1+CR1）：拆分纯路径 vs 写路径。
// - `*_analysis_dir_path`：纯解析，不创建目录——供 list/get/enhance 读端点用
// - `ensure_*_analysis_dir`：会 create_dir_all——仅供 decompose/apply 写端点用
// - `*_analysis_file_path`：带白名单校验，内部调纯路径版，不创建目录

/// `characters/{id}/analysis/` 目录路径（**纯解析，不创建目录**）。
///
/// 读端点（list/get/enhance）用此函数，避免访问不存在的 character 时
/// 留下空 sidecar 目录。写端点用 `ensure_char_analysis_dir`。
pub fn char_analysis_dir_path(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    super::security::validate_id_segment(character_id)?;
    Ok(root.join("characters").join(character_id).join("analysis"))
}

/// `characters/{id}/analysis/` 目录，**会创建**。仅供写端点（decompose/apply）使用。
pub fn ensure_char_analysis_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = char_analysis_dir_path(root, character_id)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `characters/{id}/analysis/{filename}` 路径，带白名单校验（**不创建目录**）。
///
/// 仅允许 `[a-z0-9_/.-]+\.md`，拒路径穿越、拒非 .md 扩展。
pub fn char_analysis_file_path(
    root: &Path,
    character_id: &str,
    filename: &str,
) -> Result<PathBuf, AirpError> {
    use std::path::Component;

    let valid = filename.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '/' | '.' | '-')
    }) && filename.ends_with(".md")
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
    super::security::validate_id_segment(preset_id)?;
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

    let valid = filename.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '/' | '.' | '-')
    }) && filename.ends_with(".md")
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

#[allow(dead_code)]
pub(crate) fn preset_dir(root: &Path, preset_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("presets").join(preset_id);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn preset_json_path(root: &Path, preset_id: &str) -> PathBuf {
    root.join("presets").join(preset_id).join("preset.json")
}

pub(crate) fn legacy_preset_json_path(root: &Path, preset_id: &str) -> PathBuf {
    root.join("presets").join(format!("{preset_id}.json"))
}

#[allow(dead_code)]
pub(crate) fn preset_regex_dir(root: &Path, preset_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("presets").join(preset_id).join("regex");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[allow(dead_code)]
pub(crate) fn preset_meta_path(root: &Path, preset_id: &str) -> PathBuf {
    root.join("presets").join(preset_id).join("meta.json")
}

#[allow(dead_code)]
pub(crate) fn char_history_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("characters").join(character_id).join("history");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[allow(dead_code)]
pub(crate) fn char_analysis_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let dir = root.join("characters").join(character_id).join("analysis");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn char_gating_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let char_dir = root.join("characters").join(character_id);
    fs::create_dir_all(&char_dir)?;
    let gating = char_dir.join("gating");
    fs::create_dir_all(&gating)?;

    for fname in ["checkpoints.md", "timeline.md"] {
        let old = char_dir.join(fname);
        let new = gating.join(fname);
        if old.exists() && !new.exists() {
            super::utils::move_path(&old, &new)?;
            tracing::info!(old = ?old, new = ?new, "CF-4: 迁移到 gating/");
        }
    }

    let cp = gating.join("checkpoints.md");
    if !cp.exists() {
        fs::write(&cp, CHECKPOINTS_TEMPLATE)?;
    }
    let tl = gating.join("timeline.md");
    if !tl.exists() {
        fs::write(&tl, TIMELINE_TEMPLATE)?;
    }
    Ok(gating)
}

/// M_LS-3: `characters/{id}/state/` 目录路径（不自动创建）。
pub fn char_state_dir(root: &Path, character_id: &str) -> PathBuf {
    root.join("characters").join(character_id).join("state")
}

/// M_LS-3: `characters/{id}/state/history.jsonl` 路径（不自动创建）。
pub fn char_state_history_path(root: &Path, character_id: &str) -> PathBuf {
    char_state_dir(root, character_id).join("history.jsonl")
}

pub fn list_characters(root: &Path) -> Result<Vec<String>, AirpError> {
    let chars_dir = root.join("characters");
    if !chars_dir.exists() {
        return Ok(vec![]);
    }

    let mut result = Vec::new();
    let entries = fs::read_dir(&chars_dir)?;

    for entry in entries {
        let entry = entry?;
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            if let Some(name) = entry.file_name().to_str() {
                if super::security::validate_id_segment(name).is_ok() {
                    result.push(name.to_string());
                }
            }
        }
    }

    result.sort();
    Ok(result)
}

/// Read a character's card.json as raw text for format-specific consumers.
pub fn read_character_card_text(
    root: &Path,
    character_id: &CharacterId,
) -> Result<String, AirpError> {
    let dir = root.join("characters").join(character_id.as_str());
    if !dir.is_dir() {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            character_id
        )));
    }
    let migrated = dir.join("card").join("card.json");
    let legacy = dir.join("card.json");
    let path = if migrated.exists() { migrated } else { legacy };
    if !path.exists() {
        return Err(AirpError::NotFound(format!(
            "character {} has no card.json (neither card/card.json nor card.json exists)",
            character_id
        )));
    }
    fs::read_to_string(&path).map_err(AirpError::from)
}

pub(crate) fn ensure_context_bundle_dir(
    root: &Path,
    character_id: &str,
) -> Result<PathBuf, AirpError> {
    super::security::validate_id_segment(character_id)?;
    let dir = root
        .join("exports")
        .join("context-bundles")
        .join(character_id);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Read and validate the shared card API contract: a JSON object.
pub fn get_character_card(
    root: &Path,
    character_id: &CharacterId,
) -> Result<serde_json::Value, AirpError> {
    let raw = read_character_card_text(root, character_id)?;
    let card: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        AirpError::BadRequest(format!(
            "character {} card.json is invalid JSON: {}",
            character_id, error
        ))
    })?;
    if !card.is_object() {
        return Err(AirpError::BadRequest(format!(
            "character {} card.json must be a JSON object",
            character_id
        )));
    }
    Ok(card)
}

/// Delete a character directory entirely (card + state + memory + sessions + ...)。
/// 角色不存在 → `NotFound`。destructive：调用方负责确认。
pub fn delete_character(root: &Path, character_id: &CharacterId) -> Result<(), AirpError> {
    let dir = root.join("characters").join(character_id.as_str());
    if !dir.is_dir() {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            character_id
        )));
    }
    fs::remove_dir_all(&dir).map_err(AirpError::from)
}

pub fn list_presets(root: &Path) -> Result<Vec<String>, AirpError> {
    use std::collections::BTreeSet;

    let presets_dir = root.join("presets");
    if !presets_dir.exists() {
        return Ok(vec![]);
    }

    let mut seen: BTreeSet<String> = BTreeSet::new();

    for entry in fs::read_dir(&presets_dir)? {
        let entry = entry?;
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let name = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if ft.is_dir() {
            let p = entry.path();
            if p.join("preset.json").exists() || p.join("preset.md").exists() {
                seen.insert(name);
            }
        } else if ft.is_file() {
            if let Some(stem) = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(".md"))
            {
                seen.insert(stem.to_string());
            }
        }
    }

    Ok(seen.into_iter().collect())
}

// ── M_UP: User Persona path functions ─────────────────────────────────────────
//
// User personas mirror character cards: `persona.json` is the immutable
// 元设定 (base setup), `state/live.json` is the mutable 变量设定 (drift
// overlay), and `state/history.jsonl` records the timeline. A persona.lock
// sentinel file marks a sealed (read-only) persona — further `import_user`
// calls on a locked persona are rejected so the base contract stays stable
// across an entire RP campaign.

/// `users/{user_id}/` directory (not auto-created).
///
/// P1: signature takes `&UserId` so callers cannot bypass id validation.
pub fn user_dir(root: &Path, user_id: &crate::types::UserId) -> PathBuf {
    root.join("users").join(user_id.as_str())
}

/// `users/{user_id}/persona.json` — immutable base persona (元设定).
pub fn user_persona_path(root: &Path, user_id: &crate::types::UserId) -> PathBuf {
    user_dir(root, user_id).join("persona.json")
}

/// `users/{user_id}/persona.lock` — sentinel; existence = persona is sealed.
pub fn user_persona_lock_path(root: &Path, user_id: &crate::types::UserId) -> PathBuf {
    user_dir(root, user_id).join("persona.lock")
}

/// `users/{user_id}/state/` directory, created on demand.
pub fn user_state_dir(root: &Path, user_id: &crate::types::UserId) -> Result<PathBuf, AirpError> {
    let dir = user_dir(root, user_id).join("state");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `users/{user_id}/state/live.json` — current 变量设定 (drift overlay).
pub fn user_state_live_path(root: &Path, user_id: &crate::types::UserId) -> PathBuf {
    user_dir(root, user_id).join("state").join("live.json")
}

/// `users/{user_id}/state/history.jsonl` — append-only snapshot timeline.
pub fn user_state_history_path(root: &Path, user_id: &crate::types::UserId) -> PathBuf {
    user_dir(root, user_id).join("state").join("history.jsonl")
}

/// List all user IDs present under `data/users/`.
pub fn list_users(root: &Path) -> Result<Vec<String>, AirpError> {
    let dir = root.join("users");
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

// ── M_MS: Scene path functions ────────────────────────────────────────────────
//
// AUDIT-2: signatures take `&SceneId` so the caller is forced to construct
// (and thus validate) the ID before touching the filesystem. The compile-time
// guarantee replaces the previous pattern of manual `validate_id_segment`
// calls scattered through callers.

/// `scenes/{scene_id}/` directory (not auto-created).
pub fn scene_dir(root: &Path, scene_id: &crate::types::SceneId) -> PathBuf {
    root.join("scenes").join(scene_id.as_str())
}

/// `scenes/{scene_id}/scene.json` path.
pub fn scene_json_path(root: &Path, scene_id: &crate::types::SceneId) -> PathBuf {
    scene_dir(root, scene_id).join("scene.json")
}

/// `scenes/{scene_id}/world/` directory, created on demand.
pub fn scene_world_dir(
    root: &Path,
    scene_id: &crate::types::SceneId,
) -> Result<PathBuf, AirpError> {
    let dir = scene_dir(root, scene_id).join("world");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `scenes/{scene_id}/world/lorebook.json` path (not auto-created).
pub fn scene_world_lorebook_path(root: &Path, scene_id: &crate::types::SceneId) -> PathBuf {
    scene_dir(root, scene_id)
        .join("world")
        .join("lorebook.json")
}

/// `scenes/{scene_id}/history/` directory, created on demand.
pub fn scene_history_dir(
    root: &Path,
    scene_id: &crate::types::SceneId,
) -> Result<PathBuf, AirpError> {
    let dir = scene_dir(root, scene_id).join("history");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `scenes/{scene_id}/memory/` directory, created on demand.
pub fn scene_memory_dir(
    root: &Path,
    scene_id: &crate::types::SceneId,
) -> Result<PathBuf, AirpError> {
    let dir = scene_dir(root, scene_id).join("memory");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// DX-1: Compute per-request effective data root.
///
/// - `user_id` = Some(uid): `{root}/users/{uid}/` (created on demand, minimal subdirs).
///   `uid` is validated by `validate_id_segment` — dots, slashes, empty strings rejected.
/// - `user_id` = None: returns `root` unchanged (backward-compatible single-user mode).
pub fn resolve_effective_root(root: &Path, user_id: Option<&str>) -> Result<PathBuf, AirpError> {
    match user_id {
        None | Some("") => Ok(root.to_path_buf()),
        Some(uid) => {
            super::security::validate_id_segment(uid)?;
            let user_root = root.join("users").join(uid);
            // Create minimal subdirs; skip migrations/settings for user roots.
            for sub in &["characters", "presets", "scenes"] {
                let p = user_root.join(sub);
                if !p.exists() {
                    fs::create_dir_all(&p)?;
                }
            }
            Ok(user_root)
        }
    }
}

/// List all scene IDs (directory names under `scenes/`).
pub fn list_scenes(root: &Path) -> Result<Vec<String>, AirpError> {
    let scenes_dir = root.join("scenes");
    if !scenes_dir.exists() {
        return Ok(vec![]);
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(&scenes_dir)? {
        let entry = entry?;
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let p = entry.path();
            if p.join("scene.json").exists() {
                if let Some(name) = entry.file_name().to_str() {
                    result.push(name.to_string());
                }
            }
        }
    }
    result.sort();
    Ok(result)
}

// ── Tests for resolve_data_root_inner ──────────────────────────────────────────
//
// 拆出 `resolve_data_root_inner` 的主要动机：把 env / cfg! / cwd 这些进程级依赖
// 参数化后，每条 fallback 层与边界条件都能直接 unit test，无需 `serial_test`
// 锁 env、无需改 cwd、无需 flakiness 风险。这部分覆盖 Gemini-code-assist 在
// PR #55 标的两条 review 之外的关键回归路径（空/空白 env、release 误入 dev
// 模式、dirs 不可用兜底、dev 模式但 cwd 不在 repo 根）。

#[cfg(test)]
mod analysis_path_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn char_analysis_dir_path_does_not_create_directory() {
        let tmp = tempdir().unwrap();
        let dir = char_analysis_dir_path(tmp.path(), "alice").unwrap();
        assert!(!dir.exists(), "char_analysis_dir_path 不得创建目录");
        assert_eq!(
            dir,
            tmp.path().join("characters").join("alice").join("analysis")
        );
    }

    #[test]
    fn ensure_char_analysis_dir_creates_directory() {
        let tmp = tempdir().unwrap();
        let dir = ensure_char_analysis_dir(tmp.path(), "alice").unwrap();
        assert!(dir.is_dir());
    }

    #[test]
    fn char_analysis_file_path_rejects_traversal() {
        let tmp = tempdir().unwrap();
        let result = char_analysis_file_path(tmp.path(), "alice", "../escape.md");
        assert!(result.is_err());
        assert!(
            !tmp.path()
                .join("characters")
                .join("alice")
                .join("analysis")
                .exists(),
            "校验失败不得创建目录"
        );
    }

    #[test]
    fn char_analysis_file_path_rejects_non_md_extension() {
        let tmp = tempdir().unwrap();
        let result = char_analysis_file_path(tmp.path(), "alice", "basic_info.txt");
        assert!(result.is_err());
    }

    #[test]
    fn char_analysis_file_path_accepts_world_book_subdir() {
        let tmp = tempdir().unwrap();
        let result = char_analysis_file_path(tmp.path(), "alice", "world_book/index.md");
        assert!(result.is_ok());
    }

    #[test]
    fn preset_analysis_dir_path_does_not_create_directory() {
        let tmp = tempdir().unwrap();
        let dir = preset_analysis_dir_path(tmp.path(), "mypreset").unwrap();
        assert!(!dir.exists());
        assert_eq!(
            dir,
            tmp.path().join("presets").join("mypreset").join("analysis")
        );
    }

    #[test]
    fn ensure_preset_analysis_dir_creates_directory() {
        let tmp = tempdir().unwrap();
        let dir = ensure_preset_analysis_dir(tmp.path(), "mypreset").unwrap();
        assert!(dir.is_dir());
    }

    #[test]
    fn char_analysis_dir_path_rejects_bad_id() {
        let tmp = tempdir().unwrap();
        let result = char_analysis_dir_path(tmp.path(), "../escape");
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod data_root_tests {
    use super::resolve_data_root_inner;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// 不存在路径作为 cargo_toml_path → 强制跳过 dev 模式
    fn missing_toml() -> PathBuf {
        // 固定的不存在路径（不依赖 tempdir 生命周期），保证 .exists() == false
        PathBuf::from("/nonexistent_for_test_only_Cargo.toml")
    }

    fn present_toml() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("Cargo.toml");
        fs::write(&p, "[package]\nname=\"x\"\n").unwrap();
        (tmp, p)
    }

    #[test]
    fn env_var_takes_priority_over_dev_mode() {
        // dev 模式条件全开，但 env 仍胜出
        let r = resolve_data_root_inner(Some("/custom/path"), true, true, &missing_toml());
        assert_eq!(r, PathBuf::from("/custom/path"));
    }

    #[test]
    fn env_var_empty_string_falls_through_to_dev_mode() {
        // Gemini review #1：空串必须不被当作有效路径
        // 配 debug + cargo + 真 Cargo.toml → 应落 cwd/data
        let (_tmp, fake_toml) = present_toml();
        let r = resolve_data_root_inner(Some(""), true, true, &fake_toml);
        assert_eq!(
            r,
            PathBuf::from("data"),
            "empty AIRP_DATA_DIR should fall through to dev-mode 'data', not return empty PathBuf"
        );
    }

    #[test]
    fn env_var_whitespace_only_falls_through() {
        // 纯空白视为未设置 —— shell 误传 "  " 也不该走通
        let (_tmp, fake_toml) = present_toml();
        let r = resolve_data_root_inner(Some("   "), true, true, &fake_toml);
        assert_eq!(r, PathBuf::from("data"));
    }

    #[test]
    fn release_build_with_cargo_toml_in_cwd_does_not_trigger_dev_mode() {
        // Gemini review #2 (核心)：release 打包 .exe 误入含 Cargo.toml 的目录
        // 必须不进入 dev 模式，应落 dirs::data_dir() 的 per-user 位
        let (_tmp, fake_toml) = present_toml();
        let r = resolve_data_root_inner(
            None, /* is_debug= */ false, /* under_cargo= */ false, &fake_toml,
        );
        if let Some(per_user) = dirs::data_dir() {
            assert_eq!(
                r,
                per_user.join("airp"),
                "release + coincidental Cargo.toml must NOT write to cwd/data"
            );
        } else {
            assert_eq!(r, PathBuf::from("data"));
        }
    }

    #[test]
    fn debug_build_with_cargo_toml_uses_dev_mode() {
        // debug 编译下自动走 dev 模式（即使不通过 cargo 启动）——
        // 让 `cargo build && ./target/debug/airp-core` 也能落 cwd/data
        let (_tmp, fake_toml) = present_toml();
        let r = resolve_data_root_inner(None, true, false, &fake_toml);
        assert_eq!(r, PathBuf::from("data"));
    }

    #[test]
    fn release_under_cargo_uses_dev_mode() {
        // `cargo run --release` 必须走 dev 模式（CARGO_MANIFEST_DIR 在）
        let (_tmp, fake_toml) = present_toml();
        let r = resolve_data_root_inner(None, false, true, &fake_toml);
        assert_eq!(r, PathBuf::from("data"));
    }

    #[test]
    fn no_dev_marker_no_env_uses_per_user_data_dir() {
        // release + 无 cargo + cwd 无 Cargo.toml → per-user
        let r = resolve_data_root_inner(None, false, false, &missing_toml());
        if let Some(per_user) = dirs::data_dir() {
            assert_eq!(r, per_user.join("airp"));
        } else {
            assert_eq!(r, PathBuf::from("data"));
        }
    }

    #[test]
    fn dev_mode_with_no_cargo_toml_uses_per_user() {
        // debug 但 cwd 实际不在 repo 根（无 Cargo.toml）→ 仍走 per-user，
        // 不该退化到 cwd/data；这是 2 与 3 的边界（防 Gemini #2 误伤）
        let r = resolve_data_root_inner(None, true, true, &missing_toml());
        if let Some(per_user) = dirs::data_dir() {
            assert_eq!(r, per_user.join("airp"));
        } else {
            assert_eq!(r, PathBuf::from("data"));
        }
    }
}
