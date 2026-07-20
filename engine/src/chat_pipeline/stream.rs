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

    // ── Processing task ───────────────────────────────────────────────────────
    tokio::spawn(async move {
        let mut fsm = fsm;
        let mut unpacker = unpacker;
        let mut raw_acc = String::new();
        let mut cleaned_acc = String::new();
        let mut cancelled = false;
        let mut failed = false;

        tokio::pin!(raw_stream);
        while let Some(item) = raw_stream.next().await {
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
                Err(_) => {
                    // The user message is already durable once streaming starts.
                    // Never expose the raw upstream body or invite a blind resend.
                    failed = true;
                    tracing::error!("chat upstream stream failed");
                    let _ = chunk_tx
                        .send(SseMessage::Error {
                            code: "upstream".to_string(),
                            message: "upstream request failed".to_string(),
                            retryable: false,
                            commit_state: "partially_committed",
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

        match run_finalize(finalizer, raw_acc, cleaned_acc).await {
            Ok(()) if !failed => {
                let _ = chunk_tx.send(SseMessage::Done).await;
            }
            Ok(()) => {}
            Err(error) => {
                tracing::error!(%error, "chat finalization failed");
                let _ = chunk_tx
                    .send(SseMessage::Error {
                        code: error.code_str().to_string(),
                        message: error.public_message(),
                        retryable: false,
                        commit_state: "partially_committed",
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
