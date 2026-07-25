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

    // 读取角色卡 JSON
    let card_text = crate::data_dir::read_character_card_text(&state.data_root, &cid)?;
    let mut card: serde_json::Value = serde_json::from_str(&card_text)
        .map_err(|e| AirpError::BadRequest(format!("card.json 解析失败: {}", e)))?;

    // 构造 provider config + gen params
    let snapshot = state
        .config
        .read()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?
        .clone();
    let provider_config = Arc::new(crate::adapter::ProviderConfig {
        provider: snapshot.provider.clone(),
        endpoint: snapshot.endpoint.clone(),
        api_key: snapshot.api_key.clone(),
    });
    let gen_params = crate::adapter::GenerationParams {
        model: snapshot.model.clone(),
        temperature: Some(0.7),
        max_tokens: Some((300 * payload.turns + 200).min(4000)),
    };

    // 调用 LLM 生成
    let generated =
        run_dialogue_gen(&state.http_client, provider_config, gen_params, &card, &payload).await?;

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

    // 写入角色卡：data.mes_example（v2 嵌套）或顶层 mes_example（v1 flat）
    // 取出旧值
    let data_obj = if card.get("data").is_some() {
        card.get("data").cloned().unwrap_or_default()
    } else {
        card.clone()
    };
    let previous = data_obj
        .get("mes_example")
        .and_then(|v| v.as_str())
        .map(String::from);

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

    // 写回 card JSON
    let is_v2 = card.get("data").is_some();
    if is_v2 {
        card["data"]["mes_example"] = serde_json::Value::String(new_mes_example.clone());
    } else {
        card["mes_example"] = serde_json::Value::String(new_mes_example.clone());
    }

    // 原子写回 card/card.json + card/raw.json
    let char_dir = crate::data_dir::character_dir(&state.data_root, cid.as_str())?;
    let card_dir = char_dir.join("card");
    std::fs::create_dir_all(&card_dir)?;
    let json_str = serde_json::to_string_pretty(&card)
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
