use async_stream::try_stream;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// #67 #7：chat 路径 per-request timeout（建连到首字节）。
///
/// upstream 接受连接但不响应时，`request.send().await` 会一直 hang（直到 OS/TCP keepalive
/// 超时，可能数分钟）。本 timeout 套在 `RequestBuilder::timeout` 上，reqwest 文档明确：
/// `RequestBuilder::timeout` 覆盖到 `send().await` 阶段（建连到首响应头），**body streaming
/// 消费阶段不受其约束**——故对流式 SSE 的慢 token 不误杀，恰挡"建连后挂死"。
///
/// 默认 30s（reqwest 默认无上限，30s 给 provider 冷启足够余量）。env
/// `AIRP_CHAT_REQUEST_TIMEOUT_MS` 可调（与 `models_proxy_timeout` 同模式）。
fn chat_request_timeout() -> Duration {
    const DEFAULT: Duration = Duration::from_secs(30);
    std::env::var("AIRP_CHAT_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT)
}

/// DX-6：后端引擎选择。控制 AIRP 如何调用上游 LLM 服务。
///
/// - `Direct`（默认）：OpenAI 兼容 `/v1/chat/completions` SSE，适用于 OpenAI /
///   DeepSeek / Together / Ollama 等所有 OpenAI compat 端点。
/// - `AnthropicMessages`：Anthropic 原生 `/v1/messages` API（SSE），需配合
///   `anthropic.com` 端点与 `x-api-key` 鉴权。自动从消息列表提取 `system` 字段。
/// - `ClaudeCodeSdk`：保留为未来集成 Claude Code SDK 的入口；当前返回 "not implemented"。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendEngine {
    #[default]
    Direct,
    AnthropicMessages,
    ClaudeCodeSdk,
}

/// 调用方供应商标识。
///
/// **设计决策（2026-05-21）**：放弃 4.1 多 provider 适配，统一走 OpenAI 兼容协议
/// （DeepSeek / Together / vLLM / LM Studio / Ollama OpenAI-compat 端点等都满足该格式）。
/// 此枚举保留为单变量是为了：
///   - 维持现有 `ChatCompletionRequest.provider` / `AppConfig.provider` 的字段类型；
///   - 未来如需重新引入异类 provider 时不必再改 API 形状。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[non_exhaustive]
pub enum Provider {
    #[default]
    OpenAI,
}

/// M4.2：连接层配置。`ProviderConfig` 与 LLM 服务的物理连接相关，
/// 在单次请求内不变；用 `Arc` 共享给 stream 与 finalizer 任务消除双重 clone。
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// 供应商标识（当前统一为 OpenAI 兼容）。
    pub provider: Provider,
    /// 完整端点 URL（含 `/chat/completions` 路径）。
    pub endpoint: String,
    /// 可选 API key；为 `None` 时不发 `Authorization` 头（本地 Ollama 等）。
    pub api_key: Option<String>,
}

/// M4.2：生成参数。封卷流程会基于一份生成参数派生新参数
/// （改写 model / temperature），因此与 `ProviderConfig` 解耦：clone 仅复制
/// 一个小 struct + 一次 String clone，相比原 `AdapterConfig` 已大幅缩减。
#[derive(Debug, Clone)]
pub struct GenerationParams {
    /// 上游模型 ID（如 `gpt-4o` / `glm-4-flash`）。
    pub model: String,
    /// 采样温度。`None` 走 daemon 默认值（当前 0.7）。
    pub temperature: Option<f32>,
    /// 最大生成 token 数。`None` 不设上限，交由上游默认值。
    pub max_tokens: Option<u32>,
}

/// M0 F-03 / 6.0b：消息角色 enum。serde 序列化为 OpenAI 兼容的小写字符串
/// (`"user"` / `"assistant"` / `"system"`)，反序列化同样接受这些字符串。
/// 杜绝非法 role 字符串在编译期（之前只在 API 调用时被上游拒绝）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// 终端用户消息。
    User,
    /// LLM 生成的助手消息。
    Assistant,
    /// 系统提示（system prompt）。
    System,
}

/// OpenAI 兼容协议的单条对话消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 消息发出方的角色。
    pub role: MessageRole,
    /// 消息文本内容。
    pub content: String,
}

/// 发起流式模型请求，返回统一的文本 Token Stream。
///
/// 仅支持 OpenAI 兼容 `/v1/chat/completions` SSE 流（`data: {...}\n\ndata: [DONE]\n`）。
///
/// **M0 F-01 修复**：`reqwest::Client` 由调用方注入并复用，避免每请求重建连接池。
/// `reqwest::Client` 内部基于 `Arc` 实现，`clone()` 仅 +1 引用计数，廉价。
///
/// **M4.2**：`provider` 与 `params` 分离——`provider` 用 `Arc` 共享（连接层一次构造、
/// 多任务共享），`params` 按值传入（封卷会派生新生成参数，独立 clone）。
pub fn call_streaming_api(
    client: reqwest::Client,
    provider: Arc<ProviderConfig>,
    params: GenerationParams,
    system_prompt: String,
    messages: Vec<ChatMessage>,
) -> impl Stream<Item = Result<String, String>> {
    try_stream! {
        let mut request = client.post(&provider.endpoint);

        if let Some(ref key) = provider.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let mut full_messages = vec![ChatMessage {
            role: MessageRole::System,
            content: system_prompt,
        }];
        full_messages.extend(messages);

        let mut payload = serde_json::json!({
            "model": params.model,
            "messages": full_messages,
            "stream": true,
            "temperature": params.temperature.unwrap_or(0.7),
        });
        if let Some(max) = params.max_tokens {
            payload["max_tokens"] = serde_json::Value::from(max);
        }

        // #67 #7 / 审计 G1：用 tokio::time::timeout 包 request.send().await，仅保护
        // "建连到响应头"阶段。send() 收到响应头即返回，body streaming 在 send 返回后
        // 才由 bytes_stream() 消费——故慢 token 不误杀，恰挡"建连后挂死"。
        // ⚠️ 不用 RequestBuilder::timeout（套到整个 response body 接收完成，会误杀长文本流式）。
        let response = tokio::time::timeout(chat_request_timeout(), request.json(&payload).send())
            .await
            .map_err(|_| "请求超时: 等待响应头超时".to_string())?
            .map_err(|e| format!("发送请求失败: {}", e))?;

        let mut byte_stream = if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            Err(format!("API 返回错误状态码 {}: {}", status, err_text))?
        } else {
            response.bytes_stream()
        };
        let mut line_buffer = Vec::new();

        while let Some(chunk_result) = byte_stream.next().await {
            let chunk: Bytes = chunk_result.map_err(|e| format!("读取流数据包失败: {}", e))?;
            for &b in chunk.iter() {
                if b == b'\n' {
                    let line = String::from_utf8_lossy(&line_buffer).trim().to_string();
                    line_buffer.clear();

                    if line.is_empty() {
                        continue;
                    }
                    match parse_openai_sse_line(&line) {
                        Ok(Some(token)) => yield token,
                        Ok(None) => {}
                        Err(e) => {
                            tracing::debug!(line = %line, err = %e, "ignore unparsable SSE line");
                        }
                    }
                } else {
                    line_buffer.push(b);
                }
            }
        }

        // 处理流末尾残留的单行
        if !line_buffer.is_empty() {
            let line = String::from_utf8_lossy(&line_buffer).trim().to_string();
            if !line.is_empty() {
                match parse_openai_sse_line(&line) {
                    Ok(Some(token)) => yield token,
                    Ok(None) => {}
                    Err(e) => {
                        tracing::debug!(line = %line, err = %e, "ignore unparsable trailing SSE line");
                    }
                }
            }
        }
    }
}

/// DX-6: Anthropic Messages API streaming (`/v1/messages` SSE).
///
/// Key differences from OpenAI compat:
/// - Auth header is `x-api-key` (not `Bearer`)
/// - System prompt goes in top-level `system` field (not as a message)
/// - Streaming events use `event: content_block_delta` + `delta.text`
pub fn call_streaming_api_anthropic(
    client: reqwest::Client,
    provider: Arc<ProviderConfig>,
    params: GenerationParams,
    system_prompt: String,
    messages: Vec<ChatMessage>,
) -> impl Stream<Item = Result<String, String>> {
    try_stream! {
        let mut request = client.post(&provider.endpoint);

        if let Some(ref key) = provider.api_key {
            request = request.header("x-api-key", key.as_str());
        }
        request = request.header("anthropic-version", "2023-06-01");

        // Anthropic does not accept role=system in messages array
        let filtered: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .collect();

        let mut payload = serde_json::json!({
            "model": params.model,
            "system": system_prompt,
            "messages": filtered,
            "stream": true,
            "max_tokens": params.max_tokens.unwrap_or(4096),
        });
        if let Some(t) = params.temperature {
            payload["temperature"] = serde_json::Value::from(t);
        }

        // #67 #7 / 审计 G2：同 openai 路径，tokio::time::timeout 包 send().await，
        // 仅保护建连到响应头，不误杀流式 body。
        let response = tokio::time::timeout(chat_request_timeout(), request.json(&payload).send())
            .await
            .map_err(|_| "请求超时: 等待响应头超时".to_string())?
            .map_err(|e| format!("发送请求失败: {}", e))?;

        let mut byte_stream = if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            Err(format!("Anthropic API 返回错误状态码 {}: {}", status, err_text))?
        } else {
            response.bytes_stream()
        };

        // Track current event type across lines
        let mut current_event: Option<String> = None;
        let mut line_buffer = Vec::new();

        while let Some(chunk_result) = byte_stream.next().await {
            let chunk: Bytes = chunk_result.map_err(|e| format!("读取流数据包失败: {}", e))?;
            for &b in chunk.iter() {
                if b == b'\n' {
                    let line = String::from_utf8_lossy(&line_buffer).trim().to_string();
                    line_buffer.clear();

                    if line.is_empty() {
                        current_event = None;
                        continue;
                    }
                    if let Some(ev) = line.strip_prefix("event:") {
                        current_event = Some(ev.trim().to_string());
                        continue;
                    }
                    if line.starts_with("data:")
                        && current_event.as_deref() == Some("content_block_delta")
                    {
                        if let Some(token) = parse_anthropic_delta_line(&line) {
                            yield token;
                        }
                    }
                } else {
                    line_buffer.push(b);
                }
            }
        }

        // handle trailing line
        if !line_buffer.is_empty() {
            let line = String::from_utf8_lossy(&line_buffer).trim().to_string();
            if line.starts_with("data:") && current_event.as_deref() == Some("content_block_delta") {
                if let Some(token) = parse_anthropic_delta_line(&line) {
                    yield token;
                }
            }
        }
    }
}

/// Extract text from an Anthropic `content_block_delta` data line.
fn parse_anthropic_delta_line(line: &str) -> Option<String> {
    let data = line["data:".len()..].trim();
    let json_val: Value = serde_json::from_str(data).ok()?;
    json_val
        .pointer("/delta/text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// DX-6: Dispatcher — pick streaming backend based on engine setting.
///
/// Returns a type-erased boxed stream so callers don't need to know which
/// concrete stream type was chosen.
pub type BoxTokenStream = Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>;

pub fn call_streaming_api_auto(
    engine: &BackendEngine,
    client: reqwest::Client,
    provider: Arc<ProviderConfig>,
    params: GenerationParams,
    system_prompt: String,
    messages: Vec<ChatMessage>,
) -> BoxTokenStream {
    match engine {
        BackendEngine::Direct => Box::pin(call_streaming_api(
            client,
            provider,
            params,
            system_prompt,
            messages,
        )),
        BackendEngine::AnthropicMessages => Box::pin(call_streaming_api_anthropic(
            client,
            provider,
            params,
            system_prompt,
            messages,
        )),
        BackendEngine::ClaudeCodeSdk => Box::pin(futures_util::stream::once(async {
            Err("ClaudeCodeSdk engine not yet implemented".to_string())
        })),
    }
}

/// 解析 OpenAI 兼容协议的一行 SSE，提取 `choices[0].delta.content`。
fn parse_openai_sse_line(line: &str) -> Result<Option<String>, String> {
    if !line.starts_with("data:") {
        return Ok(None);
    }
    let data_content = line["data:".len()..].trim();
    if data_content == "[DONE]" {
        return Ok(None);
    }

    let json_val: Value = serde_json::from_str(data_content)
        .map_err(|e| format!("解析 OpenAI data JSON 失败: {}，原始行: {}", e, line))?;

    if let Some(content) = json_val
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
    {
        return Ok(Some(content.to_string()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_role_serde_lowercase() {
        // 序列化为 OpenAI 兼容小写字符串
        assert_eq!(
            serde_json::to_string(&MessageRole::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
        // 反序列化接受相同字符串
        assert_eq!(
            serde_json::from_str::<MessageRole>("\"user\"").unwrap(),
            MessageRole::User
        );
        assert_eq!(
            serde_json::from_str::<MessageRole>("\"assistant\"").unwrap(),
            MessageRole::Assistant
        );
        // 非法 role 反序列化失败
        assert!(serde_json::from_str::<MessageRole>("\"bot\"").is_err());
    }

    #[test]
    fn test_chat_message_roundtrip() {
        let m = ChatMessage {
            role: MessageRole::User,
            content: "你好".to_string(),
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"role\":\"user\""));
        assert!(j.contains("\"content\":\"你好\""));
        let back: ChatMessage = serde_json::from_str(&j).unwrap();
        assert_eq!(back.role, MessageRole::User);
        assert_eq!(back.content, "你好");
    }

    #[test]
    fn test_parse_openai_delta_content() {
        let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        assert_eq!(
            parse_openai_sse_line(line).unwrap(),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_parse_openai_done_marker() {
        assert_eq!(parse_openai_sse_line("data: [DONE]").unwrap(), None);
    }

    #[test]
    fn test_parse_openai_ignores_non_data_lines() {
        // 心跳 / 事件行不爆错
        assert_eq!(parse_openai_sse_line(": heartbeat").unwrap(), None);
        assert_eq!(parse_openai_sse_line("event: ping").unwrap(), None);
    }

    #[test]
    fn test_parse_openai_no_content_delta() {
        // delta 中无 content（如 role 元数据）应返回 None 而非 Err
        let line = r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#;
        assert_eq!(parse_openai_sse_line(line).unwrap(), None);
    }

    #[test]
    fn test_parse_openai_invalid_json_is_error() {
        let res = parse_openai_sse_line("data: not-json");
        assert!(res.is_err());
    }

    // DX-6 tests
    #[test]
    fn test_backend_engine_default_is_direct() {
        assert_eq!(BackendEngine::default(), BackendEngine::Direct);
    }

    #[test]
    fn test_backend_engine_serde_roundtrip() {
        for (variant, expected_str) in &[
            (BackendEngine::Direct, "\"direct\""),
            (BackendEngine::AnthropicMessages, "\"anthropic_messages\""),
            (BackendEngine::ClaudeCodeSdk, "\"claude_code_sdk\""),
        ] {
            let serialized = serde_json::to_string(variant).unwrap();
            assert_eq!(&serialized, expected_str);
            let deserialized: BackendEngine = serde_json::from_str(&serialized).unwrap();
            assert_eq!(&deserialized, variant);
        }
    }

    #[test]
    fn test_parse_anthropic_delta_line_extracts_text() {
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        assert_eq!(parse_anthropic_delta_line(line), Some("Hello".to_string()));
    }

    #[test]
    fn test_parse_anthropic_delta_line_no_text_returns_none() {
        let line = r#"data: {"type":"message_start","message":{}}"#;
        assert_eq!(parse_anthropic_delta_line(line), None);
    }

    #[test]
    fn test_parse_anthropic_delta_line_invalid_json_returns_none() {
        assert_eq!(parse_anthropic_delta_line("data: notjson"), None);
    }
}
