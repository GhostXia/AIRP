//! Phase 4.3: 对话示例生成器 HTTP handlers。
//!
//! 端点：
//! - `POST /v1/characters/:character_id/dialogue-examples` — 调用 LLM 生成
//!   SillyTavern 兼容的 `<START>` 对话示例，可选写入角色卡 `mes_example` 字段
//!
//! 复用 `crate::dialogue_gen` 业务逻辑；handler 负责：
//! 1. 校验 character_id 与请求体
//! 2. 读取角色卡 JSON
//! 3. 调用 `run_dialogue_gen` 生成内容
//! 4. dry_run=false 时写回角色卡（保留 previous_mes_example 备份）

use crate::daemon::DaemonState;
use crate::dialogue_gen::{run_dialogue_gen, DialogueExampleRequest, DialogueExampleResponse};
use crate::error::AirpError;
use crate::types::CharacterId;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// Phase 4.3: `POST /v1/characters/:character_id/dialogue-examples`
///
/// 请求体：`{ turns?, user_stance?, scenario_override?, dry_run?, append?, user_name? }`
///
/// 行为：
/// 1. 校验 character_id 存在
/// 2. 读取角色卡 JSON（card/card.json → card/raw.json → card.json fallback）
/// 3. 调用 LLM 生成对话示例
/// 4. dry_run=false 时写入角色卡 `data.mes_example`，保留原值到响应 `previous_mes_example`
/// 5. append=true 时追加到现有 mes_example 末尾，而非覆盖
pub(in crate::daemon) async fn generate_dialogue_examples_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Json(payload): Json<DialogueExampleRequest>,
) -> impl IntoResponse {
    match run_dialogue_gen_handler(&state, &character_id, payload).await {
        Ok(resp) => match serde_json::to_value(resp) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => AirpError::from(e).into_response(),
        },
        Err(e) => e.into_response(),
    }
}

async fn run_dialogue_gen_handler(
    state: &DaemonState,
    character_id: &str,
    payload: DialogueExampleRequest,
) -> Result<DialogueExampleResponse, AirpError> {
    let cid = CharacterId::new(character_id)?;

    // 校验角色存在
    let exists = crate::data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }

    // R1（修订）：锁纪律说明。
    // 旧版尝试在"读卡 → LLM 生成 → 写卡"整段临界区上持有 character_lock 写锁，
    // 但 `std::sync::RwLockWriteGuard` 是 `!Send`，跨 `.await` 持有会让 future 变成
    // `!Send`，违反 axum `Handler` trait，编译报错 E0277。改为两阶段：
    //   Phase A（无锁）：读卡 → LLM 生成（异步、可能耗时数秒）
    //   Phase B（character_lock 写锁）：重读卡 → 校验 mes_example 仍是旧值
    //                                  → 写入新值 + mes_example.bak
    // 若 Phase B 重读发现 mes_example 被并发改动，返回 Conflict 让用户重试。
    // 这放弃了"LLM 期间阻塞所有卡写入"的强保证，但保留了关键的"原子
    // read-modify-write"——并发写入会被检测到，绝不会丢失（last-writer
    // 检测到 stale snapshot 后拒绝写入，符合 CodeRabbit #1 的修复意图）。
    // 与 update_character_card 的锁纪律对齐（其临界区是纯同步 fs 写）。

    // ── Phase A: 无锁读卡 + LLM 生成 ──
    let card_text = crate::data_dir::read_character_card_text(&state.data_root, &cid)?;
    let card: serde_json::Value = serde_json::from_str(&card_text)
        .map_err(|e| AirpError::BadRequest(format!("card.json 解析失败: {}", e)))?;
    let baseline_mes_example = card
        .get("data")
        .and_then(|d| d.get("mes_example"))
        .or_else(|| card.get("mes_example"))
        .and_then(|v| v.as_str())
        .map(String::from);

    // 构造 provider config + gen params。
    // 注意：handler 只提供 model；temperature 与 max_tokens 由 run_dialogue_gen
    // 在校验 turns (1~10) 之后安全计算（避免 300 * payload.turns 在 turns=u32::MAX
    // 时溢出 panic，构成 DoS）。
    let snapshot = state.read_config().clone();

    // CodeRabbit #9（critical）：mes_example_override 路径——前端 writeGenerated
    // 持有 dry_run=true 阶段拿到的预览内容（lastGenerated），通过本字段把该内容
    // 原样写盘。绝不在此路径再次调用 LLM，否则 temperature 0.7 非确定性会让用户
    // 预览 A 但写入 B，破坏"预览→确认→写入"契约。
    let generated = if let Some(override_text) = payload.mes_example_override.as_deref() {
        let trimmed = override_text.trim();
        if trimmed.is_empty() {
            return Err(AirpError::BadRequest(
                "mes_example_override 不能为空字符串".to_string(),
            ));
        }
        // 校验格式契约：必须包含至少一个 <START> 分隔符，与 run_dialogue_gen 一致。
        if crate::dialogue_gen::count_starts(trimmed) == 0 {
            return Err(AirpError::BadRequest(
                "mes_example_override 未包含 <START> 标记，不符合 SillyTavern 格式契约".to_string(),
            ));
        }
        trimmed.to_string()
    } else {
        // 走 LLM 生成路径
        let provider_config = Arc::new(crate::adapter::ProviderConfig {
            provider: snapshot.provider.clone(),
            endpoint: snapshot.endpoint.clone(),
            api_key: snapshot.api_key.clone(),
        });
        let gen_params = crate::adapter::GenerationParams {
            model: snapshot.model.clone(),
            temperature: None,
            max_tokens: None,
        };
        run_dialogue_gen(
            &state.http_client,
            provider_config,
            gen_params,
            &card,
            &payload,
        )
        .await?
    };

    let turns_generated = crate::dialogue_gen::count_starts(&generated) as u32;

    if payload.dry_run {
        return Ok(DialogueExampleResponse {
            written: false,
            character_id: cid.as_str().to_string(),
            mes_example: generated,
            turns_generated,
            previous_mes_example: None,
        });
    }

    // ── Phase B: 取 character_lock 写锁，原子 read-modify-write 卡片 ──
    let character = crate::domain::character_lock(cid.as_str());
    let _guard = character.write().unwrap_or_else(|p| p.into_inner());

    // 重读 card：检测 Phase A 期间是否发生并发写入。
    let card_text_now = crate::data_dir::read_character_card_text(&state.data_root, &cid)?;
    let mut card_now: serde_json::Value = serde_json::from_str(&card_text_now)
        .map_err(|e| AirpError::BadRequest(format!("card.json 解析失败: {}", e)))?;
    let current_mes_example = card_now
        .get("data")
        .and_then(|d| d.get("mes_example"))
        .or_else(|| card_now.get("mes_example"))
        .and_then(|v| v.as_str())
        .map(String::from);
    if current_mes_example != baseline_mes_example {
        // Phase A 拿到的 snapshot 已 stale，拒绝覆盖（防止丢失并发写入）。
        return Err(AirpError::Conflict(format!(
            "character {} card mes_example was modified during dialogue generation; please retry",
            cid
        )));
    }

    // 写入角色卡：data.mes_example（v2 嵌套）或顶层 mes_example（v1 flat）
    let previous = current_mes_example;

    let new_mes_example = if payload.append {
        let mut combined = previous.clone().unwrap_or_default();
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&generated);
        combined
    } else {
        generated.clone()
    };

    // 持久化备份：在覆盖 mes_example 之前，先把旧值写入 `mes_example.bak` 字段。
    // 这与模块级文档（engine/src/dialogue_gen.rs:11-16 "不破坏用户资产"）对齐：
    // 仅靠响应字段 previous_mes_example 无法跨请求/跨会话恢复旧值，必须落盘到卡内。
    // v2 卡写入 data.mes_example.bak；v1 卡写入顶层 mes_example.bak。
    let is_v2 = card_now.get("data").is_some();
    if let Some(prev) = previous.as_deref() {
        if is_v2 {
            card_now["data"]["mes_example.bak"] = serde_json::Value::String(prev.to_string());
        } else {
            card_now["mes_example.bak"] = serde_json::Value::String(prev.to_string());
        }
    }
    if is_v2 {
        card_now["data"]["mes_example"] = serde_json::Value::String(new_mes_example.clone());
    } else {
        card_now["mes_example"] = serde_json::Value::String(new_mes_example.clone());
    }

    // 原子写回 card/card.json + card/raw.json
    let char_dir = crate::data_dir::character_dir(&state.data_root, cid.as_str())?;
    let card_dir = char_dir.join("card");
    std::fs::create_dir_all(&card_dir)?;
    let json_str = serde_json::to_string_pretty(&card_now)
        .map_err(|e| AirpError::BadRequest(format!("card JSON 序列化失败: {}", e)))?;
    let json_bytes = json_str.as_bytes();
    crate::data_dir::replace_file(&card_dir.join("card.json"), json_bytes)?;
    crate::data_dir::replace_file(&card_dir.join("raw.json"), json_bytes)?;

    Ok(DialogueExampleResponse {
        written: true,
        character_id: cid.as_str().to_string(),
        mes_example: generated,
        turns_generated,
        previous_mes_example: previous,
    })
}
