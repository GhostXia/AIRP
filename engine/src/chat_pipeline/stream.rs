//! Stream phase: drive upstream LLM stream through FSM + Unpacker, emit SSE events.
//!
//! 架构（M3.2 — 热路径无 Arc/Mutex）：
//!   - 单一 **processing task** 拥有 FSM + Unpacker，通过有界 mpsc channel 把
//!     `UnpackedChunk` 批次投递给 SSE 响应流；
//!   - 正常结束或客户端取消时，由 finalizer 持久化 ChatLog + 触发卷副作用；
//!   - 只有 critical persistence 成功后才向客户端发 `done`；
//!   - SSE 响应在 mpsc receiver 上 poll，不需要任何锁。

use std::convert::Infallible;

use axum::response::sse::Event;
use futures_util::{stream, Stream, StreamExt};

use crate::adapter::call_streaming_api_auto;
use crate::xml_unpacker::UnpackedChunk;

use super::finalize::run_finalize;
use super::types::{PreparedPipeline, SseMessage};

/// Converts a `PreparedPipeline` into an SSE event stream.
///
/// Architecture (M3.2 – no Arc/Mutex on hot path):
///   - Spawns a single **processing task** that owns FSM + Unpacker.
///   - Processing task drives the raw API stream, sends `UnpackedChunk` batches
///     via a bounded mpsc channel.
///   - On normal end OR cancellation, persists ChatLog + volume side-effects.
///   - Emits `done` only after critical persistence succeeds.
///   - The SSE response polls the mpsc receiver (no mutex needed).
pub fn build_sse_stream(
    pipeline: PreparedPipeline,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let PreparedPipeline {
        provider_config,
        gen_params,
        system_prompt,
        prompt_trace: _,
        messages,
        fsm,
        unpacker,
        finalizer,
        http_client,
        engine,
    } = pipeline;

    let raw_stream = call_streaming_api_auto(
        &engine,
        http_client,
        provider_config,
        gen_params,
        system_prompt,
        messages,
    );

    let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel::<SseMessage>(32);
    // Failed regen skips finalization, so its durable assistant reply is
    // unchanged and the client can safely distinguish it from a partial write.
    let failure_commit_state = if finalizer.regen_snapshot.is_some() {
        "not_committed"
    } else {
        "partially_committed"
    };
    let cancellation = finalizer
        .session_operation_lease
        .as_ref()
        .and_then(|lease| lease.cancellation());
    let activity_session_dir = finalizer
        .session_operation_lease
        .as_ref()
        .and(finalizer.session_dir.clone());
    let activity_generation_id = finalizer
        .session_operation_lease
        .as_ref()
        .map(|lease| lease.generation_id().to_string());

    // ── Processing task ───────────────────────────────────────────────────────
    tokio::spawn(async move {
        let mut fsm = fsm;
        let mut unpacker = unpacker;
        let mut raw_acc = String::new();
        let mut cleaned_acc = String::new();
        let mut cancelled = false;
        let mut failed = false;

        tokio::pin!(raw_stream);
        loop {
            let item = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    _ = cancellation.cancelled() => {
                        cancelled = true;
                        None
                    }
                    item = raw_stream.next() => item,
                },
                None => raw_stream.next().await,
            };
            if cancelled {
                break;
            }
            let Some(item) = item else {
                break;
            };
            match item {
                Ok(token) => {
                    raw_acc.push_str(&token);
                    let cleaned = fsm.process_chunk(&token);
                    cleaned_acc.push_str(&cleaned);
                    let chunks = unpacker.process_chunk(&cleaned);
                    if chunk_tx.send(SseMessage::Chunks(chunks)).await.is_err() {
                        // Receiver dropped → client disconnected
                        cancelled = true;
                        break;
                    }
                }
                Err(error) => {
                    // The user message is already durable once streaming starts.
                    // Never expose the raw upstream body or invite a blind resend.
                    failed = true;
                    let code = crate::adapter::streaming_failure_code(&error);
                    tracing::error!(failure_code = code, "chat upstream stream failed");
                    record_activity_failure(
                        activity_session_dir.clone(),
                        activity_generation_id.clone(),
                        if code == "timeout" {
                            crate::ui_activity::ActivityFailureCode::Timeout
                        } else {
                            crate::ui_activity::ActivityFailureCode::UpstreamError
                        },
                    )
                    .await;
                    let _ = chunk_tx
                        .send(SseMessage::Error {
                            code: code.to_string(),
                            message: if code == "timeout" {
                                "upstream request timed out".to_string()
                            } else {
                                "upstream request failed".to_string()
                            },
                            retryable: false,
                            commit_state: failure_commit_state,
                        })
                        .await;
                    break;
                }
            }
        }

        if !cancelled {
            // Normal end: flush FSM tail + unpacker
            let tail = fsm.finish();
            cleaned_acc.push_str(&tail);
            let mut final_chunks = unpacker.process_chunk(&tail);
            final_chunks.extend(unpacker.finish());
            if !final_chunks.is_empty() {
                let _ = chunk_tx.send(SseMessage::Chunks(final_chunks)).await;
            }
        }

        // A failed or cancelled regen must leave its previously durable
        // assistant reply untouched. Dropping the finalizer also drops its
        // logical session lease, so later mutations can proceed.
        if (cancelled || failed) && finalizer.regen_snapshot.is_some() {
            if cancelled {
                let _ = chunk_tx
                    .send(SseMessage::Error {
                        code: "cancelled".to_string(),
                        message: "generation cancelled".to_string(),
                        retryable: false,
                        commit_state: "not_committed",
                    })
                    .await;
            }
            return;
        }

        match run_finalize(finalizer, raw_acc, cleaned_acc).await {
            Ok(()) if cancelled => {
                let _ = chunk_tx
                    .send(SseMessage::Error {
                        code: "cancelled".to_string(),
                        message: "generation cancelled".to_string(),
                        retryable: false,
                        commit_state: failure_commit_state,
                    })
                    .await;
            }
            Ok(()) if !failed => {
                let _ = chunk_tx.send(SseMessage::Done).await;
            }
            Ok(()) => {}
            Err(error) => {
                tracing::error!(%error, "chat finalization failed");
                let cancelled = matches!(
                    &error,
                    crate::error::AirpError::Conflict(message)
                        if message == "generation_cancelled"
                );
                if !cancelled {
                    record_activity_failure(
                        activity_session_dir,
                        activity_generation_id,
                        crate::ui_activity::ActivityFailureCode::FinalizationFailed,
                    )
                    .await;
                }
                let _ = chunk_tx
                    .send(SseMessage::Error {
                        code: if cancelled {
                            "cancelled".to_string()
                        } else {
                            error.code_str().to_string()
                        },
                        message: if cancelled {
                            "generation cancelled".to_string()
                        } else {
                            error.public_message()
                        },
                        retryable: false,
                        commit_state: failure_commit_state,
                    })
                    .await;
            }
        }
    });

    // ── SSE stream: mpsc receiver → Event items ───────────────────────────────
    stream::unfold(chunk_rx, |mut rx| async move {
        rx.recv().await.map(|result| {
            let events = chunks_result_to_events(result);
            (events, rx)
        })
    })
    .flat_map(stream::iter)
}

async fn record_activity_failure(
    session_dir: Option<std::path::PathBuf>,
    generation_id: Option<String>,
    code: crate::ui_activity::ActivityFailureCode,
) {
    let Some(session_dir) = session_dir else {
        return;
    };
    let result = tokio::task::spawn_blocking(move || {
        crate::ui_activity::record_failure(
            &session_dir,
            crate::ui_activity::ActivitySource::Chat,
            code,
            generation_id.as_deref(),
        )
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "failed to persist chat activity receipt"),
        Err(error) => tracing::warn!(%error, "chat activity persistence task failed"),
    }
}

pub(super) fn chunks_result_to_events(result: SseMessage) -> Vec<Result<Event, Infallible>> {
    match result {
        SseMessage::Chunks(chunks) => chunks
            .into_iter()
            .filter_map(|chunk| match &chunk {
                UnpackedChunk::Think(t) if t.is_empty() => None,
                UnpackedChunk::Body(t) if t.is_empty() => None,
                _ => {
                    let data = serde_json::to_string(&chunk).unwrap_or_default();
                    Some(Ok(Event::default().event("message").data(data)))
                }
            })
            .collect(),
        SseMessage::Error {
            code,
            message,
            retryable,
            commit_state,
        } => {
            let data = serde_json::to_string(&serde_json::json!({
                "type": "error",
                "text": message,
                "error": {
                    "code": code,
                    "message": message,
                    "retryable": retryable,
                    "commit_state": commit_state,
                }
            }))
            .unwrap_or_default();
            vec![Ok(Event::default().event("error").data(data))]
        }
        SseMessage::Done => vec![Ok(Event::default()
            .event("message")
            .data(r#"{"type":"done"}"#))],
    }
}

/// SSE 合同锁定测试：实际发射的帧形状必须与 `protocol/sse-events.json`
/// 机器可读规格一致（参照 `protocol/wire-discriminants.json` 的锁定模式）。
#[cfg(test)]
mod sse_contract_tests {
    use super::*;
    use axum::response::sse::Sse;
    use axum::response::IntoResponse;
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn contract() -> Value {
        serde_json::from_str(include_str!("../../../protocol/sse-events.json"))
            .expect("protocol/sse-events.json 必须是合法 JSON")
    }

    /// 把 `chunks_result_to_events` 的输出经 axum `Sse` 渲染为真实 wire 文本。
    async fn wire_text(msg: SseMessage) -> String {
        let events = chunks_result_to_events(msg);
        let response = Sse::new(stream::iter(events)).into_response();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("SSE 响应体可收集");
        String::from_utf8(bytes.to_vec()).expect("SSE wire 为 UTF-8")
    }

    /// 解析 wire 文本为 (event 名, data JSON) 序列。
    fn parse_frames(wire: &str) -> Vec<(String, Value)> {
        let mut frames = Vec::new();
        for block in wire.split("\n\n") {
            let block = block.trim();
            if block.is_empty() {
                continue;
            }
            let mut event = "message".to_string();
            let mut data = Vec::new();
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    event = value.trim().to_string();
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.trim());
                }
            }
            let payload: Value = serde_json::from_str(&data.join("\n")).expect("data 为合法 JSON");
            frames.push((event, payload));
        }
        frames
    }

    /// 校验 payload 字段与规格声明的字段类型一致（string / boolean / string[] / 嵌套对象）。
    fn assert_fields_match(payload: &Value, fields: &Value, type_name: &str) {
        let fields = fields
            .as_object()
            .unwrap_or_else(|| panic!("规格中 {type_name} 的 fields 必须是对象"));
        for (name, kind) in fields {
            let value = payload
                .get(name)
                .unwrap_or_else(|| panic!("{type_name} 帧缺少字段 {name}: {payload}"));
            if let Some(nested) = kind.get("fields") {
                assert!(value.is_object(), "{type_name}.{name} 必须是对象");
                assert_fields_match(value, nested, &format!("{type_name}.{name}"));
                continue;
            }
            match kind
                .as_str()
                .expect("字段类型必须是字符串标记或嵌套 fields 对象")
            {
                "string" => assert!(value.is_string(), "{type_name}.{name} 必须是 string"),
                "boolean" => assert!(value.is_boolean(), "{type_name}.{name} 必须是 boolean"),
                "string[]" => {
                    assert!(
                        value.is_array() && value.as_array().unwrap().iter().all(Value::is_string),
                        "{type_name}.{name} 必须是 string 数组"
                    );
                }
                other => panic!("规格含未知字段类型标记: {other}"),
            }
        }
    }

    #[tokio::test]
    async fn emitted_message_frames_match_sse_contract() {
        let contract = contract();
        let data_types = contract["events"]["message"]["dataTypes"]
            .as_object()
            .expect("规格必须声明 message.dataTypes");

        // 空文本 chunk 被过滤（既有发射行为，也是合同的一部分）。
        let wire = wire_text(SseMessage::Chunks(vec![
            UnpackedChunk::Body(String::new()),
            UnpackedChunk::Think(String::new()),
        ]))
        .await;
        assert!(parse_frames(&wire).is_empty(), "空 chunk 不得产生帧");

        // 三类非终态 chunk 的帧形状逐一与规格比对。
        let wire = wire_text(SseMessage::Chunks(vec![
            UnpackedChunk::Body("你好".to_string()),
            UnpackedChunk::Think("心想".to_string()),
            UnpackedChunk::ActionOptions {
                options: vec!["向左".to_string(), "向右".to_string()],
            },
        ]))
        .await;
        let frames = parse_frames(&wire);
        assert_eq!(frames.len(), 3, "每个非空 chunk 恰好一帧: {wire}");
        for (event, payload) in &frames {
            assert_eq!(
                event, "message",
                "非错误帧的 event 名必须是 message: {payload}"
            );
            let chunk_type = payload["type"]
                .as_str()
                .unwrap_or_else(|| panic!("message data 必须含 type 判别符: {payload}"));
            let spec = &data_types[chunk_type];
            assert!(
                !spec.is_null(),
                "发射了规格未锁定的判别符 {chunk_type}，违反 additive-only"
            );
            assert_fields_match(payload, &spec["fields"], chunk_type);
        }
        assert_eq!(
            payload_types(&frames),
            ["body_chunk", "think_chunk", "action_options"],
            "发射顺序与 chunk 顺序一致"
        );

        // UnpackedChunk 的 serde 判别符集合与规格集合逐字一致（防 enum 漂移）。
        let all_chunk_types: BTreeSet<String> = [
            serde_json::to_value(UnpackedChunk::Body(String::new())).unwrap(),
            serde_json::to_value(UnpackedChunk::Think(String::new())).unwrap(),
            serde_json::to_value(UnpackedChunk::ActionOptions { options: vec![] }).unwrap(),
        ]
        .iter()
        .map(|value| value["type"].as_str().unwrap().to_string())
        .collect();
        let spec_chunk_types: BTreeSet<String> = data_types
            .keys()
            .filter(|kind| kind.as_str() != "done")
            .cloned()
            .collect();
        assert_eq!(
            all_chunk_types, spec_chunk_types,
            "UnpackedChunk 判别符集合必须与规格一致"
        );

        // done 终态帧：event=message，data 形状与规格一致。
        let wire = wire_text(SseMessage::Done).await;
        let frames = parse_frames(&wire);
        assert_eq!(frames.len(), 1);
        let (event, payload) = &frames[0];
        assert_eq!(event, "message");
        assert_eq!(payload["type"], "done", "终态帧判别符必须是 done");
        assert_fields_match(payload, &data_types["done"]["fields"], "done");
    }

    fn payload_types(frames: &[(String, Value)]) -> Vec<&str> {
        frames
            .iter()
            .map(|(_, payload)| payload["type"].as_str().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn emitted_error_frame_matches_sse_contract() {
        let contract = contract();
        let spec_fields = &contract["events"]["error"]["fields"];
        assert!(spec_fields.is_object(), "规格必须声明 error 帧字段形状");

        let wire = wire_text(SseMessage::Error {
            code: "upstream_timeout".to_string(),
            message: "上游超时".to_string(),
            retryable: true,
            commit_state: "not_committed",
        })
        .await;
        let frames = parse_frames(&wire);
        assert_eq!(frames.len(), 1, "错误消息恰好一帧: {wire}");
        let (event, payload) = &frames[0];
        assert_eq!(event, "error", "错误帧的 event 名必须是 error");
        assert_eq!(payload["type"], "error", "event 与 data.type 必须一致");
        assert_eq!(payload["text"], "上游超时", "顶层 text 为人类可读摘要");
        assert_fields_match(payload, spec_fields, "error");
        let detail = &payload["error"];
        assert_eq!(detail["code"], "upstream_timeout");
        assert_eq!(detail["message"], "上游超时");
        assert_eq!(detail["retryable"], true);
        assert_eq!(detail["commit_state"], "not_committed");
    }

    #[tokio::test]
    async fn contract_event_names_are_the_locked_closed_set() {
        let contract = contract();
        assert_eq!(contract["compatibility"], "additive-only");
        assert_eq!(
            contract["eventNames"],
            serde_json::json!(["message", "error"])
        );
        assert_eq!(contract["events"]["message"]["dataDiscriminator"], "type");
        // 发射端实际产生的 event 名全部落在封闭集合内。
        for msg in [
            SseMessage::Chunks(vec![UnpackedChunk::Body("x".to_string())]),
            SseMessage::Done,
            SseMessage::Error {
                code: "c".to_string(),
                message: "m".to_string(),
                retryable: false,
                commit_state: "not_committed",
            },
        ] {
            let frames = parse_frames(&wire_text(msg).await);
            for (event, _) in frames {
                assert!(
                    contract["eventNames"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|name| name == &Value::String(event.clone())),
                    "发射了封闭集合之外的事件名: {event}"
                );
            }
        }
    }
}
