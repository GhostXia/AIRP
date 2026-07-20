//! M_AGENT-1: 单步生成（供 AgentLoop 协调器复用）。
//!
//! 与 `build_sse_stream` 的区别：后者把 prepare→stream→finalize 三相封装成 SSE
//! 流，结果吞进 finalizer；而 AgentLoop 需要每步拿到累积结果（raw / cleaned /
//! 拆包 chunks）来决策下一步（调工具 or 续写 or 收敛），且 finalize 由协调器
//! 在收敛时统一触发，不在每步触发。故抽出此函数：跑一次生成，返回累积，
//! **不 finalize**。
//!
//! 复用纪律（计划书 §4.1 铁律）：不重写 SSE / provider / 拆包。本函数内部仍走
//! `call_streaming_api_auto` + `StreamingFsm` + `StreamingXmlUnpacker`，只是把
//! 累积结果交还调用方而非塞进 SSE channel。

use futures_util::StreamExt;

use crate::adapter::call_streaming_api_auto;

use super::types::{GenerationStepResult, PreparedPipeline};

/// 跑一次生成步骤：复用 `PreparedPipeline` 的全部装配，跑流式生成，返回累积。
///
/// **不触发 finalize**（不持久化 ChatLog / 不落 state / 不封卷）——调用方
/// （`AgentLoop`）在收敛时自行决定是否落库。这避免 loop 多步中间态污染 ChatLog。
pub async fn run_generation_step(pipeline: PreparedPipeline) -> GenerationStepResult {
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
    let mut chunks: Vec<crate::xml_unpacker::UnpackedChunk> = Vec::new();
    let mut error: Option<String> = None;

    while let Some(item) = raw_stream.next().await {
        match item {
            Ok(token) => {
                raw_acc.push_str(&token);
                let cleaned = fsm.process_chunk(&token);
                cleaned_acc.push_str(&cleaned);
                chunks.extend(unpacker.process_chunk(&cleaned));
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }

    if error.is_none() {
        let tail = fsm.finish();
        cleaned_acc.push_str(&tail);
        chunks.extend(unpacker.process_chunk(&tail));
        chunks.extend(unpacker.finish());
    }

    GenerationStepResult {
        raw_acc,
        cleaned_acc,
        chunks,
        error,
        finalizer,
    }
}
