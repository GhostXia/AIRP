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
