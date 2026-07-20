//! stdout runner (M4.5)：把 PreparedPipeline 跑到完成，输出到 stdout / stderr。
//!
//! 与 `build_sse_stream` 共享同一 prepare / stream / finalize 路径——CLI `run`
//! 子命令复用全部 daemon 改进（FSM + Unpacker + 持久化 + 卷注入）而不需 TCP
//! 自 POST。

use crate::adapter::call_streaming_api_auto;
use crate::error::AirpError;
use crate::xml_unpacker::UnpackedChunk;
use futures_util::StreamExt;

use super::finalize::run_finalize;
use super::types::PreparedPipeline;

/// Drives a `PreparedPipeline` to completion, printing `Body` chunks to stdout
/// and `Think` chunks to stderr.
///
/// 与 `build_sse_stream` 共享同一 prepare/stream/finalize 路径——CLI `run` 子命令
/// 复用全部 daemon 改进（FSM + Unpacker + 持久化 + 卷注入）而不需 TCP 自 POST。
pub async fn run_pipeline_to_stdout(pipeline: PreparedPipeline) -> Result<(), AirpError> {
    use std::io::Write;

    let PreparedPipeline {
        provider_config,
        gen_params,
        system_prompt,
        prompt_trace: _,
        messages,
        mut fsm,
        mut unpacker,
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
    tokio::pin!(raw_stream);

    let mut raw_acc = String::new();
    let mut cleaned_acc = String::new();
    let mut had_error: Option<String> = None;

    while let Some(item) = raw_stream.next().await {
        match item {
            Ok(token) => {
                raw_acc.push_str(&token);
                let cleaned = fsm.process_chunk(&token);
                cleaned_acc.push_str(&cleaned);
                for chunk in unpacker.process_chunk(&cleaned) {
                    print_chunk_to_stdout(&chunk);
                }
            }
            Err(e) => {
                eprintln!("\n[Error]: {}", e);
                had_error = Some(e);
                break;
            }
        }
    }

    if had_error.is_none() {
        let tail = fsm.finish();
        cleaned_acc.push_str(&tail);
        let tail_chunks: Vec<_> = unpacker
            .process_chunk(&tail)
            .into_iter()
            .chain(unpacker.finish())
            .collect();
        for chunk in tail_chunks {
            print_chunk_to_stdout(&chunk);
        }
    }

    println!();
    let _ = std::io::stdout().flush();

    // 即使流出错也调用 finalize，让累积的 user/assistant 文本仍能持久化。
    run_finalize(finalizer, raw_acc, cleaned_acc).await?;

    match had_error {
        Some(e) => Err(AirpError::Upstream { status: 0, body: e }),
        None => Ok(()),
    }
}

pub(super) fn print_chunk_to_stdout(chunk: &UnpackedChunk) {
    use std::io::Write;
    match chunk {
        UnpackedChunk::Body(text) if !text.is_empty() => {
            print!("{}", text);
            let _ = std::io::stdout().flush();
        }
        UnpackedChunk::Think(text) if !text.is_empty() => {
            // stderr 避免污染 stdout 管道；ANSI dim 标记思考块
            eprintln!("\x1b[2m[思考] {}\x1b[0m", text.trim_end());
        }
        UnpackedChunk::ActionOptions { options } if !options.is_empty() => {
            for (i, opt) in options.iter().enumerate() {
                println!("\x1b[33m[选项 {}] {}\x1b[0m", i + 1, opt);
            }
        }
        _ => {}
    }
}
