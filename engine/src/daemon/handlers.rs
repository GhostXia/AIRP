//! HTTP handler functions for the daemon API.
//!
//! #155 PR 5 之后：本文件是 handler facade。sessions / personas / chat / agent
//! / settings / presets / scenes / models 八个 family 已拆入 `handlers/` 子模块，
//! facade 经 `pub(super) use` re-export 保持 `daemon/mod.rs` 的
//! `use handlers::{...}` 调用路径不变。其余 handler（characters / state /
//! lorebook）仍留在本文件。
use super::DaemonState;
use crate::data_dir;
use crate::domain::{ChatService, LorebookService};
use crate::error::AirpError;
use crate::types::CharacterId;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

mod agent;
mod chat;
mod models;
mod personas;
mod presets;
mod scenes;
mod sessions;
mod settings;

// #155 PR 4/5：re-export moved handlers 保持 `daemon/mod.rs` 的 `use handlers::{...}` 不变。
pub(super) use agent::{agent_run, list_agent_tools};
pub(super) use chat::{chat_completion, get_chat_history, regen_chat, rollback_chat};
pub(super) use models::list_models;
pub(super) use personas::{
    bind_persona_endpoint, create_persona_endpoint, delete_persona_multi_endpoint,
    get_persona_endpoint, get_persona_multi_endpoint, list_personas_endpoint,
    unbind_persona_endpoint, update_persona_endpoint, update_persona_multi_endpoint,
};
pub(super) use presets::{get_preset_endpoint, import_preset_endpoint, list_presets_endpoint};
pub(super) use scenes::{
    add_scene_character_endpoint, create_scene_endpoint, get_scene_endpoint, list_scenes_endpoint,
};
pub(super) use sessions::{
    create_session_endpoint, delete_session_endpoint, list_sessions_endpoint,
};
pub(super) use settings::{get_settings, update_settings};

const MAX_DERIVED_CHARACTER_ID_BYTES: usize = 120;

// ── Private request/response types (handler-local) ────────────────────────────

/// R-01: Import a character card. Path-first (守不变式6)：优先 `card_path`
/// 让引擎读盘；`card_json`/`card_png_base64` 为 fallback（无真实路径时）。
/// `character_id` 可选——不传时引擎从 `card.name` slugify 派生并在响应返回。
#[derive(Debug, Deserialize)]
pub(super) struct ImportCharacterRequest {
    /// 落盘目录名。None → 引擎从卡内 name 派生；重名自动加后缀。
    pub character_id: Option<CharacterId>,
    /// 绝对路径——引擎 fs::read 后按内容嗅探（PNG 魔数 → png_parser，否则 JSON）。
    pub card_path: Option<std::path::PathBuf>,
    /// TavernCardV2 JSON string (fallback)。
    pub card_json: Option<String>,
    /// Raw PNG bytes, base64-encoded (fallback)。
    pub card_png_base64: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ImportCharacterResponse {
    /// 最终落盘用的 id（传入则原样；未传则为派生 id）。
    pub character_id: String,
    pub card_format: String,
}

// ── Character handlers ────────────────────────────────────────────────────────

/// GET /v1/characters — list all available character folder names
pub(super) async fn list_characters(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<Vec<String>>, AirpError> {
    let chars = data_dir::list_characters(&state.data_root)?;
    Ok(Json(chars))
}

// ── Character card import ─────────────────────────────────────────────────────

/// M_MCP MCP-2: 角色卡导入的内部实现（pub(crate) 供 daemon HTTP handler 与 MCP tool 共享）。
///
/// 三选一参数（按优先级）：`card_path`（path-first，引擎读盘）/ `card_json` / `card_png_base64`。
/// 全为 None → 400。
///
/// `character_id` 为 `None` 时，引擎解析卡后 slugify `data.name` 派生默认 id 并返回；
/// 重名（目标目录已存在）时加 `-2`/`-3` 后缀，不覆盖既有角色。
///
/// 副作用（CF-7 解包）：
/// - `card.json` 或 `card.png` 写入角色根目录（向后兼容）
/// - `card/raw.json` — 完整 TavernV2 JSON（最小 sidecar，守 ASSET-SPEC 规则2 存储永不丢）
/// - `card/greetings/00.md` — first_mes（其他 `0x.md` 为 alternate_greetings）
/// - `world/lorebook.json` — 角色卡内嵌 character_book 转 Lorebook 格式
///
/// 返回 `(character_id, card_format, json_str)`。`character_id` 是最终落盘用的 id
/// （可能与传入不同——传入 None 时为派生 id）。
pub(crate) fn import_card_to_disk(
    data_root: &std::path::Path,
    character_id: Option<&str>,
    card_path: Option<&std::path::Path>,
    card_json: Option<String>,
    card_png_base64: Option<String>,
) -> Result<(String, String, String), AirpError> {
    // 阶段 1：提取 + 校验 JSON（path/PNG 先读入内存，暂不落盘）。
    // 写盘推迟到形状校验之后，避免被拒的预设残留脏文件。
    let (card_format, json_str, png_bytes): (String, String, Option<Vec<u8>>) = if let Some(path) =
        card_path
    {
        // path-first 主路径：引擎读盘（守不变式6——大 blob 不经线协议）。
        // ⚠️ RR-001 / 审计 2026-07-04：card_path = 服务端任意绝对路径读。
        // 门控：仅当 engine 启动时设了 AIRP_ALLOW_LOCAL_PATH=1 才开放此分支。
        // Tauri sidecar 启动脚本带此变量；对外暴露的 engine 不带 → Web/远端
        // 调用方即使伪造 JSON body 带 card_path 也被拒。不可伪造（env 在进程
        // 启动时定，非请求头）。审计裁定：Web 永不走 card_path，即使持 bearer。
        let allow_local_path = crate::config::local_path_import_enabled();
        if !allow_local_path {
            return Err(AirpError::BadRequest(
                    "card_path 任意路径读已禁用（AIRP_ALLOW_LOCAL_PATH 未设）。Web/远端调用方请用 card_png_base64 或 card_json 字段（JSON body，非 multipart）。".to_string(),
                ));
        }
        let bytes = fs::read(path)
            .map_err(|e| AirpError::BadRequest(format!("读取 card_path 失败: {}", e)))?;
        // 按内容嗅探：PNG 魔数 → png_parser；否则当 JSON 文本。
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            let json = crate::png_parser::parse_png_character_card_bytes(&bytes)?;
            ("png".to_string(), json, Some(bytes))
        } else {
            let clean = data_dir::strip_utf8_bom(
                std::str::from_utf8(&bytes)
                    .map_err(|e| AirpError::BadRequest(format!("card_path 非 UTF-8: {}", e)))?,
            )
            .to_owned();
            let _ = serde_json::from_str::<serde_json::Value>(&clean)
                .map_err(|e| AirpError::BadRequest(format!("card_path 不是有效 JSON: {}", e)))?;
            ("json".to_string(), clean, None)
        }
    } else if let Some(json) = card_json {
        let clean = data_dir::strip_utf8_bom(&json).to_owned();
        let _ = serde_json::from_str::<serde_json::Value>(&clean)
            .map_err(|e| AirpError::BadRequest(format!("card_json 不是有效 JSON: {}", e)))?;
        ("json".to_string(), clean, None)
    } else if let Some(png_b64) = card_png_base64 {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&png_b64)
            .map_err(|e| AirpError::BadRequest(format!("base64 解码失败: {}", e)))?;
        // 从内存字节解析 tEXt/ccv3；解析失败即报错，不落盘。
        let json = crate::png_parser::parse_png_character_card_bytes(&bytes)?;
        ("png".to_string(), json, Some(bytes))
    } else {
        return Err(AirpError::BadRequest(
            "必须提供 card_path / card_json / card_png_base64 之一".to_string(),
        ));
    };

    // 阶段 2：形状校验。若内容明显是 SillyTavern 预设（顶层 prompts[] + 模型参数），
    // 拒绝导入为角色卡，提示改用 import_preset。此处尚未写盘，拒绝不留脏文件。
    if matches!(
        crate::orchestrator::card::detect_json_shape(&json_str),
        crate::orchestrator::card::JsonShape::Preset
    ) {
        return Err(AirpError::BadRequest(
            "内容像 SillyTavern 预设（顶层 prompts[] + 模型参数），不是角色卡。请改用 import_preset 导入。".to_string(),
        ));
    }

    // v1 平铺卡归一化为 v2 schema（data 嵌套）。v2/v3 卡原样返回。
    // 不归一化则下游 TavernCardV2 解析失败，greetings/lorebook 全丢。
    let json_str = crate::orchestrator::card::normalize_v1_to_v2(&json_str);

    // 阶段 2.5：确定 character_id。传入则校验；未传则 slugify card.name 派生。
    let final_id: String = match character_id {
        Some(id) => {
            // 传入的 id 必须本身合法（UI/调用方负责）；CharacterId 校验。
            CharacterId::new(id)?;
            id.to_string()
        }
        None => {
            let name = extract_card_name(&json_str);
            let base = slugify_id(&name);
            resolve_unique_id(data_root, &base)?
        }
    };
    // 最终 id 再过一次 newtype 校验（slugify 可能产生需复核的串）+ 防 None 路径漏网。
    CharacterId::new(&final_id)?;
    let char_dir = data_dir::character_dir(data_root, &final_id)?;

    // 阶段 3：落盘（仅在校验通过后）。
    if let Some(bytes) = png_bytes {
        fs::write(char_dir.join("card.png"), &bytes)?;
        let card_dir = data_dir::char_card_dir(data_root, &final_id)?;
        fs::write(card_dir.join("card.png"), &bytes)?;
    } else {
        fs::write(char_dir.join("card.json"), &json_str)?;
    }

    // CF-7: 解包资产（非阻塞；失败仅 warn）
    extract_card_assets(data_root, &final_id, &json_str);

    Ok((final_id, card_format, json_str))
}

/// 从归一化后的 TavernV2 JSON 提取 `data.name`。失败/缺字段返回空串。
fn extract_card_name(json_str: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json_str)
        .ok()
        .and_then(|v| {
            v.get("data")
                .and_then(|d| d.get("name"))?
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 把卡名 sanitize 成合法 id_segment：NFC 归一化，替换文件名非法字符/空白，
/// 移除不可见控制字符，去行首点，去 `..`，并限制长度。空串返回 `character`。
fn slugify_id(name: &str) -> String {
    let mut s = String::with_capacity(name.len().min(MAX_DERIVED_CHARACTER_ID_BYTES));
    for c in name.nfc() {
        if is_invisible_id_control(c) {
            continue;
        }
        match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => s.push('_'),
            c if c.is_whitespace() => s.push('_'),
            _ => s.push(c),
        }
    }
    // 去行首点（validate 拒 `.`/`..` 及以点开头）。
    while s.starts_with('.') {
        s.remove(0);
    }
    // 去 `..` 子串（validate 拒含 `..`）——逐字符折叠连续点为单点。
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_dot = false;
    for c in s.chars() {
        if c == '.' {
            if !prev_dot {
                collapsed.push(c);
            }
            prev_dot = true;
        } else {
            collapsed.push(c);
            prev_dot = false;
        }
    }
    let mut s = collapsed;
    truncate_utf8_bytes(&mut s, MAX_DERIVED_CHARACTER_ID_BYTES);
    if s.is_empty() {
        "character".to_string()
    } else {
        s
    }
}

fn is_invisible_id_control(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' // zero-width + LRM/RLM
                | '\u{202A}'..='\u{202E}' // bidi embedding/override
                | '\u{2060}'..='\u{206F}' // word joiner + bidi isolates
                | '\u{FEFF}' // BOM / zero-width no-break space
        )
}

fn truncate_utf8_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// 目标角色目录已存在时加 `-2`/`-3` 后缀直到空闲，不覆盖既有角色。
fn resolve_unique_id(data_root: &std::path::Path, base: &str) -> Result<String, AirpError> {
    let candidate = |id: &str| data_root.join("characters").join(id).exists();
    if !candidate(base) {
        return Ok(base.to_string());
    }
    for n in 2..u32::MAX {
        let id = format!("{}-{}", base, n);
        if !candidate(&id) {
            return Ok(id);
        }
    }
    Err(AirpError::BadRequest("角色 id 重名后缀耗尽".to_string()))
}

pub(super) async fn import_character(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<ImportCharacterRequest>,
) -> Result<Json<ImportCharacterResponse>, AirpError> {
    let cid_str = req.character_id.as_ref().map(|c| c.as_str().to_string());
    let (final_id, card_format, _json_str) = import_card_to_disk(
        &state.data_root,
        cid_str.as_deref(),
        req.card_path.as_deref(),
        req.card_json,
        req.card_png_base64,
    )?;
    Ok(Json(ImportCharacterResponse {
        character_id: final_id,
        card_format,
    }))
}

/// POST /v1/characters/:character_id/reextract
/// 对已导入的角色卡重新运行 CF-7 资产解包（world/ + card/greetings/）。
pub(super) async fn reextract_character_assets(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id_str): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id_str)
        .map_err(|e| AirpError::BadRequest(format!("非法 character_id: {}", e)))?;
    let char_dir = data_dir::character_dir(&state.data_root, cid.as_str())?;

    let json_str = if char_dir.join("card").join("raw.json").exists() {
        let raw = fs::read_to_string(char_dir.join("card").join("raw.json"))?;
        data_dir::strip_utf8_bom(&raw).to_owned()
    } else if char_dir.join("card.json").exists() {
        let raw = fs::read_to_string(char_dir.join("card.json"))?;
        data_dir::strip_utf8_bom(&raw).to_owned()
    } else if char_dir.join("card.png").exists() {
        crate::png_parser::parse_png_character_card(char_dir.join("card.png"))?
    } else {
        return Err(AirpError::NotFound(format!(
            "角色 {} 无可用卡片文件（card.json / card.png）",
            cid.as_str()
        )));
    };

    extract_card_assets(&state.data_root, cid.as_str(), &json_str);

    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "status": "ok",
        "message": "资产解包已触发（world/lorebook.json + card/greetings/）"
    })))
}

/// CF-7: 从 TavernV2 JSON 解包子资产，写入功能子目录。
///
/// 失败路径 `tracing::warn` 而非返回错误——导入主路径已完成，资产解包是增值操作。
/// `pub(crate)` 用于 M_MCP MCP-2 复用（避免与 daemon::import_character 代码漂移）。
pub(crate) fn extract_card_assets(data_root: &std::path::Path, character_id: &str, json_str: &str) {
    use crate::orchestrator::card::TavernCardV2;

    let card: TavernCardV2 = match serde_json::from_str(json_str) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(err = %e, character_id, "CF-7: 解析 TavernCardV2 失败，跳过资产解包");
            return;
        }
    };

    // ── card/raw.json ──
    match data_dir::char_card_dir(data_root, character_id) {
        Ok(card_dir) => {
            if let Err(e) = fs::write(card_dir.join("raw.json"), json_str) {
                tracing::warn!(err = %e, character_id, "CF-7: 写 card/raw.json 失败");
            }
        }
        Err(e) => tracing::warn!(err = %e, character_id, "CF-7: 创建 card/ 目录失败"),
    }

    // ── card/greetings/ ──
    match data_dir::char_greetings_dir(data_root, character_id) {
        Ok(greet_dir) => {
            if let Some(ref fm) = card.data.first_mes {
                let path = greet_dir.join("00.md");
                if let Err(e) = fs::write(&path, fm) {
                    tracing::warn!(err = %e, "CF-7: 写 greetings/00.md 失败");
                }
            }
            for (i, greeting) in card.data.alternate_greetings.iter().enumerate() {
                let path = greet_dir.join(format!("{:02}.md", i + 1));
                if let Err(e) = fs::write(&path, greeting) {
                    tracing::warn!(err = %e, idx = i + 1, "CF-7: 写 greetings/{:02}.md 失败", i + 1);
                }
            }
        }
        Err(e) => tracing::warn!(err = %e, character_id, "CF-7: 创建 greetings/ 目录失败"),
    }

    // ── world/lorebook.json ──
    if let Some(ref cb) = card.data.character_book {
        let (lorebook, report) = crate::orchestrator::normalize_worldbook(cb);
        if let Some(reason) = report.replacement_error() {
            tracing::warn!(
                character_id,
                reason,
                "CF-7: character_book 归一化失败，跳过 lorebook 写入"
            );
        } else if lorebook.entries.is_empty() && report.total_input == 0 {
            tracing::warn!(
                character_id,
                "CF-7: character_book 无 entries，跳过 lorebook 写入"
            );
        } else {
            match serde_json::to_string_pretty(&lorebook) {
                Ok(lb_json) => {
                    let lb_path = data_dir::char_world_lorebook_path(data_root, character_id);
                    if let Err(e) = data_dir::char_world_dir(data_root, character_id) {
                        tracing::warn!(err = %e, "CF-7: 创建 world/ 目录失败");
                        return;
                    }
                    if let Err(e) = fs::write(&lb_path, lb_json) {
                        tracing::warn!(err = %e, "CF-7: 写 world/lorebook.json 失败");
                    } else {
                        tracing::info!(
                            character_id,
                            entries = lorebook.entries.len(),
                            total_input = report.total_input,
                            converted = report.converted,
                            aliases_normalized = report.aliases_normalized,
                            advisory_preserved = report.advisory_preserved,
                            invalid = report.invalid.len(),
                            needs_review = report.needs_review.len(),
                            "CF-7: world/lorebook.json 已写入（normalizer v3）"
                        );
                    }
                }
                Err(e) => tracing::warn!(err = %e, "CF-7: 序列化 Lorebook 失败"),
            }
        }
    }
}

// ── Character state / avatar handlers ────────────────────────────────────────

/// GET /v1/characters/:character_id/avatar — serve card.png as image/png.
pub(super) async fn get_character_avatar(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let char_dir = match data_dir::character_dir(&state.data_root, char_id.as_str()) {
        Ok(d) => d,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let png_path = char_dir.join("card.png");
    match fs::read(&png_path) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state
pub(super) async fn get_character_state(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let live_path = data_dir::char_state_dir(&state.data_root, char_id.as_str()).join("live.json");
    match fs::read_to_string(&live_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state/schema
pub(super) async fn get_character_state_schema(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let schema_path =
        data_dir::char_state_dir(&state.data_root, char_id.as_str()).join("schema.json");
    match fs::read_to_string(&schema_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state/history?limit=N
pub(super) async fn get_character_state_history(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .clamp(1, 1000);

    let history_path = data_dir::char_state_history_path(&state.data_root, char_id.as_str());
    let Ok(text) = fs::read_to_string(&history_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let entries: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(limit)
        .collect();

    match serde_json::to_string(&entries) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// ── Character card / lorebook CRUD（PR E：工作台编辑所需） ──────────────────

/// GET /v1/characters/:character_id — 返回角色卡 JSON 原文。
/// 优先读 `card/card.json`（迁移后路径），兼容旧 `card.json`。
pub(super) async fn get_character_card(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    Ok(Json(data_dir::get_character_card(&state.data_root, &cid)?))
}

/// DELETE /v1/characters/:character_id — 删除整个角色目录（card + state + sessions + ...）。
/// destructive：调用方负责确认。返回 {deleted: id}。
pub(super) async fn delete_character_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    ChatService::new(&state.data_root).delete_character(&cid)?;
    Ok(Json(serde_json::json!({
        "deleted": cid.as_str(),
        "status": "ok"
    })))
}

/// PUT /v1/characters/:character_id — 更新角色卡 JSON（整体替换）。
/// body 是 TavernCardV2 JSON；写回 `card/card.json` + `card/raw.json`。
/// 不重新解包资产（greetings/lorebook），如需重解请调 reextract。
///
/// 设计说明：raw.json 在导入时是"原始 imported 卡"的 sidecar（守 ASSET-SPEC
/// 规则2 存储永不丢）。本端点将其一并覆盖——把工作台编辑视为"新的规范化
/// 版本"，后续 reextract 会以编辑后的卡为源。如需保留原始 imported 卡，
/// 调用方应在 PUT 前自行备份（例如调 reextract 前再 PUT 一次原始内容）。
pub(super) async fn update_character_card(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Json(card): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    // 校验角色已存在（character_dir 会创建子目录，所以用 list_characters 校验）
    let exists = data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }
    let json_str = serde_json::to_string_pretty(&card)
        .map_err(|e| AirpError::BadRequest(format!("card JSON 序列化失败: {}", e)))?;
    let char_dir = data_dir::character_dir(&state.data_root, cid.as_str())?;
    let card_dir = char_dir.join("card");
    fs::create_dir_all(&card_dir)?;
    fs::write(card_dir.join("card.json"), &json_str)?;
    fs::write(card_dir.join("raw.json"), &json_str)?;
    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "status": "ok"
    })))
}

/// GET /v1/characters/:character_id/lorebook — 返回角色级世界书 JSON。
/// 不存在 → 404（与空对象 {} 区分）。
pub(super) async fn get_character_lorebook(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    // #67 #5 fix: 改用 Result<Json<Value>, AirpError> 统一错误格式。
    // 之前返回 Response + 裸 StatusCode::BAD_REQUEST，客户端 formatError 拿不到结构化 error body。
    let char_id = CharacterId::new(character_id)?;
    let lorebook = LorebookService::new(&state.data_root).read(&char_id)?;
    Ok(Json(serde_json::to_value(lorebook)?))
}

/// PUT /v1/characters/:character_id/lorebook — 更新角色级世界书（整体替换）。
///
/// body 接受三种形式（由 [`normalize_worldbook`] 统一归一化）：
/// - AIRP canonical Lorebook JSON（幂等）
/// - SillyTavern lorebook / character_book entries（含 `disable`/`order`/
///   `keysecondary`/`caseSensitive` 等别名）
/// - 裸 entry 数组
///
/// 返回写入的 canonical Lorebook 条目数 + 归一化诊断报告。
/// 角色不存在 → 404。
pub(super) async fn update_character_lorebook(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    // 校验角色已存在
    let exists = data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }
    let (lorebook, report) = crate::orchestrator::normalize_worldbook(&body);
    if let Some(reason) = report.replacement_error() {
        return Err(AirpError::BadRequest(format!("invalid lorebook: {reason}")));
    }
    LorebookService::new(&state.data_root).write(&cid, &lorebook)?;
    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "entries_count": lorebook.entries.len(),
        "import_report": report,
        "status": "ok"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CharacterId;
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvVarGuard {
        key: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let original = std::env::var_os(key);
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn with_local_path_env<F: FnOnce()>(enabled: bool, f: F) {
        let _lock = ENV_LOCK.blocking_lock();
        let _env = EnvVarGuard::set("AIRP_ALLOW_LOCAL_PATH", enabled.then_some("1"));
        f();
    }

    #[test]
    fn slugify_strips_invisible_controls() {
        let id = slugify_id("艾\u{200B}米\u{200F}丽\u{202E}");
        assert_eq!(id, "艾米丽");
        CharacterId::new(&id).unwrap();
    }

    #[test]
    fn slugify_normalizes_to_nfc() {
        let decomposed = slugify_id("Cafe\u{301}");
        let composed = slugify_id("Caf\u{00E9}");
        assert_eq!(decomposed, composed);
        assert_eq!(composed, "Caf\u{00E9}");
        CharacterId::new(&composed).unwrap();
    }

    #[test]
    fn slugify_truncates_long_names_on_utf8_boundary() {
        let id = slugify_id(&"角色".repeat(200));
        assert!(id.len() <= MAX_DERIVED_CHARACTER_ID_BYTES);
        assert!(std::str::from_utf8(id.as_bytes()).is_ok());
        CharacterId::new(&id).unwrap();
    }

    #[test]
    fn card_path_import_rejected_without_local_path_env() {
        with_local_path_env(false, || {
            let data_root = tempfile::tempdir().unwrap();
            let card_file = tempfile::NamedTempFile::new().unwrap();

            let result = import_card_to_disk(
                data_root.path(),
                Some("gate-test"),
                Some(card_file.path()),
                None,
                None,
            );

            assert!(
                matches!(result, Err(AirpError::BadRequest(msg)) if msg.contains("AIRP_ALLOW_LOCAL_PATH"))
            );
        });
    }

    #[test]
    fn card_path_import_allowed_with_local_path_env() {
        with_local_path_env(true, || {
            let data_root = tempfile::tempdir().unwrap();
            let card_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(
                card_file.path(),
                r#"{"spec":"chara_card_v2","data":{"name":"Gate Test","first_mes":"hi"}}"#,
            )
            .unwrap();

            let (character_id, card_format, json) = import_card_to_disk(
                data_root.path(),
                Some("gate-test"),
                Some(card_file.path()),
                None,
                None,
            )
            .unwrap();

            assert_eq!(character_id, "gate-test");
            assert_eq!(card_format, "json");
            assert!(json.contains("Gate Test"));
            assert!(data_root
                .path()
                .join("characters/gate-test/card/raw.json")
                .exists());
        });
    }

    // M3 RR-001 护栏 HTTP-level 回归测试
    //
    // 单测 `card_path_import_rejected_without_local_path_env` 只覆盖
    // `import_card_to_disk` 内部分支；此处验证完整 axum 路由链路：
    //   POST /v1/characters/import {card_path: "..."}  →  400 + 明确错误文案
    //
    // 守 RR-001：Web/browser 永远不能用 card_path 让 engine 读任意本地路径，
    // 即使持 bearer token、即使请求 body 形式合法。env 门控不可伪造
    // （进程启动时定，非请求头）。复用 ENV_LOCK 与 unit test 串行，避免 env race。
    //
    // Gemini Code Assist 建议抽 DaemonState 初始化样板：两个 m3_* 测试共用
    // `make_state_for_http_test`，避免 ~13 行重复。helper 返回 (state, _tmpguard)，
    // `_tmpguard` 持有 tempdir 防止目录被早回收。
    fn make_state_for_http_test() -> (Arc<super::super::DaemonState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(super::super::DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(super::super::MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://localhost".to_string(),
                api_key: None,
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });
        (state, tmp)
    }

    #[tokio::test]
    async fn m3_import_card_path_rejected_at_http_level() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let _lock = ENV_LOCK.lock().await;
        let _env = EnvVarGuard::set("AIRP_ALLOW_LOCAL_PATH", None);

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state.clone());
        let body = serde_json::json!({ "card_path": "/etc/passwd" });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/characters/import")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let s = String::from_utf8_lossy(&body_bytes);
        assert!(
            s.contains("AIRP_ALLOW_LOCAL_PATH"),
            "错误响应应明确提示 card_path 已被 env 门控禁用，got: {}",
            s
        );
    }

    // M3 happy-path HTTP 烟测：card_json 路径不被 env 门控影响，未设
    // AIRP_ALLOW_LOCAL_PATH 时仍可正常导入（确认护栏不影响合法路径）。
    #[tokio::test]
    async fn m3_import_card_json_works_without_local_path_env() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let _lock = ENV_LOCK.lock().await;
        let _env = EnvVarGuard::set("AIRP_ALLOW_LOCAL_PATH", None);

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state);
        let body = serde_json::json!({
            "card_json": r#"{"spec":"chara_card_v2","data":{"name":"Http M3 Test","first_mes":"hi"}}"#
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/characters/import")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        // slugify_id 不小写化，仅把空格 → '_'：`Http M3 Test` → `Http_M3_Test`
        assert_eq!(v["character_id"], "Http_M3_Test");
        assert_eq!(v["card_format"], "json");
    }

    // ── PR #74 W-01: get_character_lorebook HTTP-level 回归测试 ─────────────
    //
    // 守 #67 #5 修复：handler 改为 `Result<Json<Value>, AirpError>` 后，错误响应
    // 必须是 JSON envelope（`{"error":{"code","message"}}`），不能是裸 StatusCode。
    // 复用 make_state_for_http_test，3 个 case 覆盖主要分支。

    #[tokio::test]
    async fn pr74_lorebook_not_found_returns_json_envelope() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/does_not_exist/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["error"]["code"], "not_found");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("does_not_exist"),
            "错误 message 应含 character_id，got: {}",
            v["error"]["message"]
        );
    }

    #[tokio::test]
    async fn pr74_lorebook_invalid_character_id_returns_400_envelope() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = super::super::create_router(state);
        // 含路径遍历字符 → CharacterId::new 校验失败 → BadRequest
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/..%2Fetc/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["error"]["code"], "bad_request");
    }

    #[tokio::test]
    async fn pr74_lorebook_happy_path_returns_json_value() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        // 在 data_root 下放一个合法 lorebook 文件
        let char_dir = tmp.path().join("characters").join("test_char");
        std::fs::create_dir_all(char_dir.join("world")).unwrap();
        std::fs::write(
            char_dir.join("world").join("lorebook.json"),
            r#"{"entries":[{"keys":["hi"],"content":"hello"}]}"#,
        )
        .unwrap();

        let app = super::super::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/test_char/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["entries"][0]["keys"][0], "hi");
        assert_eq!(v["entries"][0]["content"], "hello");
    }

    #[tokio::test]
    async fn lorebook_put_rejects_invalid_replacement_without_overwrite() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        let world_dir = tmp
            .path()
            .join("characters")
            .join("test_char")
            .join("world");
        std::fs::create_dir_all(&world_dir).unwrap();
        let lorebook_path = world_dir.join("lorebook.json");
        let original = r#"{"entries":[{"keys":["safe"],"content":"keep me"}]}"#;
        std::fs::write(&lorebook_path, original).unwrap();

        let response = super::super::create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/characters/test_char/lorebook")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "entries": [{"keys": ["bad"], "content": 42}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(lorebook_path).unwrap(), original);
    }

    // ── L1 修复（issue #89 B3 / #92 L1）：非法 card.json 校验测试覆盖 ───────
    //
    // PR #62 Gemini 建议写入前校验 TavernCardV2。本组测试覆盖非法 card.json 场景，
    // 断言现有行为：语法错误 / 预设误导入 / 缺 data.name 均被正确处理。

    /// 非法 JSON 语法 → BadRequest "不是有效 JSON"
    #[test]
    fn l1_invalid_card_json_syntax_rejected() {
        let data_root = tempfile::tempdir().unwrap();
        let result = import_card_to_disk(
            data_root.path(),
            Some("bad-syntax"),
            None,
            Some("{not valid json".to_string()),
            None,
        );
        assert!(
            matches!(&result, Err(AirpError::BadRequest(msg)) if msg.contains("不是有效 JSON")),
            "expected BadRequest with JSON syntax error, got: {:?}",
            result
        );
        // 不留脏文件
        assert!(!data_root.path().join("characters/bad-syntax").exists());
    }

    /// JSON 是预设（顶层 prompts[] + 模型参数）→ BadRequest "像 SillyTavern 预设"
    #[test]
    fn l1_preset_misimport_as_card_rejected() {
        let data_root = tempfile::tempdir().unwrap();
        let preset_json = serde_json::json!({
            "prompts": [{"name": "sys", "content": "be helpful"}],
            "model": "gpt-4o",
            "temperature": 0.7
        })
        .to_string();
        let result = import_card_to_disk(
            data_root.path(),
            Some("preset-as-card"),
            None,
            Some(preset_json),
            None,
        );
        assert!(
            matches!(&result, Err(AirpError::BadRequest(msg)) if msg.contains("预设")),
            "expected BadRequest rejecting preset-as-card, got: {:?}",
            result
        );
        assert!(!data_root.path().join("characters/preset-as-card").exists());
    }

    /// 合法 JSON 但缺 data.name → slugify 回退为 "character"，仍可导入（覆盖现有行为）
    #[test]
    fn l1_card_missing_data_name_falls_back_to_character_id() {
        let data_root = tempfile::tempdir().unwrap();
        // 无 data.name 的 v2 卡：spec 正确但缺 name 字段
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "",
                "description": "test",
                "first_mes": "hi"
            }
        })
        .to_string();
        // 传 None 触发 slugify 派生路径，由于 name 为空，应回退为 "character"
        let (id, _fmt, _json) =
            import_card_to_disk(data_root.path(), None, None, Some(json), None).unwrap();
        assert_eq!(id, "character");
        // 落盘成功
        assert!(data_root
            .path()
            .join("characters/character/card.json")
            .exists());
    }

    /// 合法 v2 卡 + 显式 character_id → 正常导入（happy path 覆盖）
    #[test]
    fn l1_valid_v2_card_imports() {
        let data_root = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "a knight",
                "first_mes": "Hello!"
            }
        })
        .to_string();
        let (id, fmt, _json) =
            import_card_to_disk(data_root.path(), Some("alice-test"), None, Some(json), None)
                .unwrap();
        assert_eq!(id, "alice-test");
        assert_eq!(fmt, "json");
        assert!(data_root
            .path()
            .join("characters/alice-test/card.json")
            .exists());
        assert!(data_root
            .path()
            .join("characters/alice-test/card/raw.json")
            .exists());
    }

    /// 三参数全 None → BadRequest "必须提供 ... 之一"
    #[test]
    fn l1_no_card_source_rejected() {
        let data_root = tempfile::tempdir().unwrap();
        let result = import_card_to_disk(data_root.path(), Some("none"), None, None, None);
        assert!(
            matches!(&result, Err(AirpError::BadRequest(msg)) if msg.contains("card_path") && msg.contains("card_json")),
            "expected BadRequest requiring one of card_path/card_json/card_png_base64, got: {:?}",
            result
        );
        // 不留脏文件（审计 CR4：与其他拒绝测试一致防御）
        assert!(!data_root.path().join("characters/none").exists());
    }
}
