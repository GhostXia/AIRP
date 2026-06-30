//! M6.1：wiremock 集成测试 — 上游 OpenAI 兼容 SSE 流端到端验证。
//!
//! 测试目标：
//!   1. `call_streaming_api` 能正确按 `data:` 行切分 SSE 字节流
//!   2. 多 chunk 跨越 SSE 行边界时不丢 token
//!   3. 上游 HTTP 错误状态码正确映射为 stream error
//!   4. 上游 `[DONE]` 标记后 stream 正常关闭

use std::sync::Arc;

use airp_core::adapter::{
    call_streaming_api, ChatMessage, GenerationParams, MessageRole, Provider, ProviderConfig,
};
use futures_util::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 构造 OpenAI 兼容 SSE 响应体：每个 token 包装为 `data: {...delta.content...}` 行 + `[DONE]` 结尾。
fn build_sse_body(tokens: &[&str]) -> String {
    let mut out = String::new();
    for tk in tokens {
        let escaped = tk.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            escaped
        ));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn build_provider(endpoint: String) -> Arc<ProviderConfig> {
    Arc::new(ProviderConfig {
        provider: Provider::OpenAI,
        endpoint,
        api_key: Some("test-key".to_string()),
    })
}

fn build_params() -> GenerationParams {
    GenerationParams {
        model: "test-model".to_string(),
        temperature: Some(0.7),
        max_tokens: None,
    }
}

fn build_user_msg(content: &str) -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: MessageRole::User,
        content: content.to_string(),
    }]
}

#[tokio::test]
async fn streaming_api_collects_all_delta_tokens() {
    let server = MockServer::start().await;
    let body = build_sse_body(&["你好", "，", "世界", "！"]);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let client = reqwest::Client::new();
    let stream = call_streaming_api(
        client,
        build_provider(endpoint),
        build_params(),
        "你是一个助手".to_string(),
        build_user_msg("打个招呼"),
    );

    tokio::pin!(stream);
    let mut collected = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(tok) => collected.push_str(&tok),
            Err(e) => panic!("stream error: {}", e),
        }
    }
    assert_eq!(collected, "你好，世界！");
}

#[tokio::test]
async fn streaming_api_handles_done_marker_only() {
    // 上游立即返回 [DONE] 无 delta → stream 空但不报错
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: [DONE]\n\n"),
        )
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let stream = call_streaming_api(
        reqwest::Client::new(),
        build_provider(endpoint),
        build_params(),
        "sys".to_string(),
        build_user_msg("hi"),
    );

    tokio::pin!(stream);
    let mut count = 0;
    while let Some(item) = stream.next().await {
        item.expect("应无错误");
        count += 1;
    }
    assert_eq!(count, 0);
}

#[tokio::test]
async fn streaming_api_propagates_upstream_error_status() {
    // 上游 401 → stream 第一帧返回 Err 含错误信息
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"unauthorized"}"#))
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let stream = call_streaming_api(
        reqwest::Client::new(),
        build_provider(endpoint),
        build_params(),
        "sys".to_string(),
        build_user_msg("hi"),
    );

    tokio::pin!(stream);
    let first = stream.next().await.expect("应有错误帧而非直接 None");
    let err = first.expect_err("401 应返回 Err");
    assert!(err.contains("401"), "错误信息应含状态码: {}", err);
    assert!(err.contains("unauthorized"), "错误应含上游 body: {}", err);
}

#[tokio::test]
async fn streaming_api_handles_unparsable_sse_line_gracefully() {
    // 中间夹一行无法解析的 SSE → 应被忽略，正常 token 继续 yield（M1.6 容错）
    let server = MockServer::start().await;
    let body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n\
data: this-is-not-json\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}]}\n\n\
data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let stream = call_streaming_api(
        reqwest::Client::new(),
        build_provider(endpoint),
        build_params(),
        "sys".to_string(),
        build_user_msg("hi"),
    );

    tokio::pin!(stream);
    let mut collected = String::new();
    while let Some(item) = stream.next().await {
        // 不可解析行应 tracing::debug 而非传播为 stream Err
        let tok = item.expect("可解析行错误不应中断 stream");
        collected.push_str(&tok);
    }
    assert_eq!(collected, "OK!");
}

#[tokio::test]
async fn streaming_api_handles_crlf_line_endings() {
    // M1.6：SSE 行用 \r\n 结尾（部分上游网关如此）应被兼容
    let server = MockServer::start().await;
    let body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\r\n\r\n\
data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\r\n\r\n\
data: [DONE]\r\n\r\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let stream = call_streaming_api(
        reqwest::Client::new(),
        build_provider(endpoint),
        build_params(),
        "sys".to_string(),
        build_user_msg("hi"),
    );

    tokio::pin!(stream);
    let mut collected = String::new();
    while let Some(item) = stream.next().await {
        collected.push_str(&item.expect("CRLF 应被容忍"));
    }
    assert_eq!(collected, "AB");
}
