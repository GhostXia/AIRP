//! Bounded Agent loop with a provider-native structured tool-call control plane.
//!
//! 计划书 §4.0/§4.1：loop = 纯净 subagent 的编排器。协调器在每一步选择
//! 「派生纯净 subagent 生成 / 调一个工具 / 收敛结束」，把现有 `chat_pipeline`
//! **当库复用**，一行 SSE/provider/拆包都不重写。
//!
//! ## 两平面隔离（戒律#6，计划书 §4.2）
//! - **角色平面**：派生 subagent 时由 `prepare_pipeline` 装配全新纯净上下文
//!   （card / lorebook / preset / 卷 / state），**零 agent 脚手架**。
//! - **控制平面**：协调器自己的多步状态（已调工具 / 轮次 / observe 结果）
//!   活在协调器局部变量，**不注入** subagent 的 system prompt 或 messages。
//!
//! 这条不变式由 `subagent_context_has_no_orchestrator_noise` 测试守护。
//!
//! ## 有界（戒律#1，§2.1）
//! - step 上限 + token 预算 + 墙钟超时，任一触顶即停。
//! - 客户端取消（CancellationToken）→ 已派生子任务收敛。
//!
//! ## 触发判定（§4.3）
//! - `max_steps` 缺省或 =1 → 单回合退化（= 现有 `/v1/chat/completions`）。
//! - `max_steps>1` → 进 loop。

pub mod tools;

use crate::chat_pipeline::{finalize_generation, prepare_pipeline, run_generation_step};
use crate::daemon::{ChatCompletionRequest, DaemonState};
use crate::error::AirpError;
use airp_state_protocol::Capability;
use axum::response::sse::Event;
use futures_util::{stream, Stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tools::ToolRegistry;

// ── 请求 / 事件协议 ─────────────────────────────────────────────────────────

/// `POST /v1/agent/run` 入参。是 `ChatCompletionRequest` 的超集：加 `max_steps`。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentRunRequest {
    /// 基础 RP 请求（与 `/v1/chat/completions` 同形态）。
    #[serde(flatten)]
    pub base: ChatCompletionRequest,
    /// loop 步数上限。缺省或 =1 → 单回合退化（不进 loop）。
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    /// token 预算（输出侧累计）。缺省 = 不限（仅 step cap 兜底）。
    #[serde(default)]
    pub token_budget: Option<u64>,
    /// 墙钟超时秒数。缺省 = 300s。
    #[serde(default = "default_wall_clock_secs")]
    pub wall_clock_secs: u64,
    /// Capabilities granted by the trusted host for this run. Tool execution is
    /// denied unless `call:tool` is present.
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// Optional per-run tool allowlist, intersected with the engine registry.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Destructive tool names explicitly confirmed by the user/host.
    #[serde(default)]
    pub confirm_tools: Vec<String>,
}

fn default_max_steps() -> u32 {
    1
}
fn default_wall_clock_secs() -> u64 {
    300
}

/// Stable SSE event protocol: plan/tool_call/tool_result/delta/done.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// 协调器规划了一步。
    Plan { step: u32, action: PlanAction },
    /// 工具被调用。
    ToolCall {
        step: u32,
        tool: String,
        params: Value,
    },
    /// 工具返回。
    ToolResult {
        step: u32,
        tool: String,
        output: Value,
        dry_run: bool,
    },
    /// 生成增量（subagent 的拆包 chunk）。
    Delta { step: u32, chunk: String },
    /// loop 结束。
    Done {
        stop_reason: StopReason,
        steps_taken: u32,
        tokens_estimated: u64,
    },
}

/// 协调器每步的动作选择。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    /// 派生纯净 subagent 跑一次生成。
    Generate,
    /// 调一个工具。
    CallTool { tool: String, params: Value },
    /// 收敛结束。
    Finish,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 模型一步直接出叙事、无工具调用 → 视为收敛。
    Converged,
    /// 达到 step 上限。
    StepCap,
    /// 达到 token 预算。
    TokenBudget,
    /// 墙钟超时。
    WallClock,
    /// 客户端取消。
    Cancelled,
    /// 上游错误。
    UpstreamError,
}

// ── AgentLoop ────────────────────────────────────────────────────────────────

/// loop 协调器。薄层：持注册表 + 共享 daemon state，`run` 产 SSE 事件流。
pub struct AgentLoop {
    state: Arc<DaemonState>,
    registry: ToolRegistry,
}

impl AgentLoop {
    pub fn new(state: Arc<DaemonState>) -> Self {
        let registry = tools::default_registry(state.clone());
        Self { state, registry }
    }

    /// 跑一次 agent run，返回 SSE 事件流。
    ///
    /// 复用纪律：subagent 生成走 `prepare_pipeline` + `run_generation_step`，
    /// 不重写流式层。finalize 由协调器在收敛时对**最后一步**触发（落库/封卷）。
    pub fn run(
        self,
        req: AgentRunRequest,
        cancel: CancellationToken,
    ) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
        let state = self.state;
        let registry = Arc::new(self.registry);

        // 双向 channel：协调器任务 → SSE 层。
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

        tokio::spawn(async move {
            let outcome = run_loop(&state, &registry, &req, &cancel, tx.clone()).await;
            // 确保 done 事件发出（run_loop 内部收敛路径已发，这里兜底防漏）。
            if outcome.is_none() {
                let _ = tx
                    .send(AgentEvent::Done {
                        stop_reason: StopReason::UpstreamError,
                        steps_taken: 0,
                        tokens_estimated: 0,
                    })
                    .await;
            }
        });

        stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|ev| {
                let event = Event::default().data(serde_json::to_string(&ev).unwrap_or_default());
                (Ok(event), rx)
            })
        })
    }
}

async fn run_loop(
    state: &Arc<DaemonState>,
    registry: &Arc<ToolRegistry>,
    req: &AgentRunRequest,
    cancel: &CancellationToken,
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
) -> Option<StopReason> {
    let max_steps = req.max_steps.max(1);
    let deadline = Instant::now() + Duration::from_secs(req.wall_clock_secs.max(1));
    let token_budget = req.token_budget.unwrap_or(u64::MAX);
    let mut steps_taken: u32 = 0;
    let mut tokens_estimated: u64 = 0;
    let mut observations = Vec::new();
    let tool_authority_enabled = state
        .config
        .read()
        .map(|config| {
            config
                .access_api_key
                .as_deref()
                .is_some_and(|key| !key.is_empty())
        })
        .unwrap_or(false);
    if req.capabilities.contains(&Capability::CallTool) && !tool_authority_enabled {
        tracing::warn!(
            "ignoring requested Agent tool capabilities because AIRP_ACCESS_KEY is not configured"
        );
    }

    loop {
        // ── 闸：取消 / 墙钟 / step cap / token 预算 ──
        if cancel.is_cancelled() {
            return emit_done(tx, StopReason::Cancelled, steps_taken, tokens_estimated).await;
        }
        if Instant::now() >= deadline {
            return emit_done(tx, StopReason::WallClock, steps_taken, tokens_estimated).await;
        }
        if steps_taken >= max_steps {
            return emit_done(tx, StopReason::StepCap, steps_taken, tokens_estimated).await;
        }
        if tokens_estimated >= token_budget {
            return emit_done(tx, StopReason::TokenBudget, steps_taken, tokens_estimated).await;
        }

        let action = if max_steps == 1
            || !tool_authority_enabled
            || !req.capabilities.contains(&Capability::CallTool)
        {
            PlanAction::Generate
        } else {
            match decide_action(state, registry, req, &observations).await {
                Ok(action) => action,
                Err(error) => {
                    tracing::warn!(%error, "structured tool planner failed");
                    return emit_done(tx, StopReason::UpstreamError, steps_taken, tokens_estimated)
                        .await;
                }
            }
        };
        steps_taken += 1;
        let _ = tx
            .send(AgentEvent::Plan {
                step: steps_taken,
                action: action.clone(),
            })
            .await;

        match action {
            PlanAction::CallTool { tool, params } => {
                let _ = tx
                    .send(AgentEvent::ToolCall {
                        step: steps_taken,
                        tool: tool.clone(),
                        params: params.clone(),
                    })
                    .await;
                let result = match registry.get(&tool) {
                    Some(t)
                        if registry.allowed(
                            &tool,
                            &req.capabilities,
                            req.allowed_tools.as_deref(),
                        ) =>
                    {
                        let confirmed = req.confirm_tools.iter().any(|name| name == &tool);
                        t.call(params.clone(), confirmed).await
                    }
                    _ => Err(AirpError::BadRequest(format!(
                        "tool not granted for this run: {tool}"
                    ))),
                };
                match result {
                    Ok(r) => {
                        let _ = tx
                            .send(AgentEvent::ToolResult {
                                step: steps_taken,
                                tool: tool.clone(),
                                output: r.output.clone(),
                                dry_run: r.dry_run,
                            })
                            .await;
                        observations.push(ControlObservation {
                            tool,
                            params,
                            output: r.output,
                            dry_run: r.dry_run,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(err = %e, tool = %tool, "tool call failed");
                        let error_output = serde_json::json!({"error": e.to_string()});
                        let _ = tx
                            .send(AgentEvent::ToolResult {
                                step: steps_taken,
                                tool: tool.clone(),
                                output: error_output.clone(),
                                dry_run: true,
                            })
                            .await;
                        observations.push(ControlObservation {
                            tool,
                            params,
                            output: error_output,
                            dry_run: true,
                        });
                    }
                }
            }
            PlanAction::Generate => {
                // 派生纯净 subagent：复用 prepare_pipeline 装配全新上下文。
                // 戒律#6：base 请求里无任何协调器噪声（协调器状态不进 system prompt / messages）。
                let pipeline = match prepare_pipeline(&req.base, state) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(err = %e, "prepare_pipeline failed in loop");
                        return emit_done(
                            tx,
                            StopReason::UpstreamError,
                            steps_taken,
                            tokens_estimated,
                        )
                        .await;
                    }
                };
                // Generation stays pure while the planner is still deciding;
                // only this converged generation is finalized below.
                let result = run_generation_step(pipeline).await;
                if let Some(e) = result.error {
                    tracing::warn!(err = %e, "generation step upstream error");
                    return emit_done(tx, StopReason::UpstreamError, steps_taken, tokens_estimated)
                        .await;
                }
                // 累计 token + 流式下发 chunks。
                let step_tokens = crate::volume_store::estimate_tokens(&result.raw_acc) as u64;
                tokens_estimated += step_tokens;
                for chunk in &result.chunks {
                    let s = format!("{:?}", chunk);
                    let _ = tx
                        .send(AgentEvent::Delta {
                            step: steps_taken,
                            chunk: s,
                        })
                        .await;
                }
                finalize_generation(result.finalizer, result.raw_acc, result.cleaned_acc).await;
                return emit_done(tx, StopReason::Converged, steps_taken, tokens_estimated).await;
            }
            PlanAction::Finish => {
                return emit_done(tx, StopReason::Converged, steps_taken, tokens_estimated).await;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ControlObservation {
    tool: String,
    params: Value,
    output: Value,
    dry_run: bool,
}

/// Provider-neutral decision boundary. Provider-specific wire decoding stays in
/// this function; the loop only sees typed `PlanAction` and observations.
async fn decide_action(
    state: &Arc<DaemonState>,
    registry: &ToolRegistry,
    req: &AgentRunRequest,
    observations: &[ControlObservation],
) -> Result<PlanAction, AirpError> {
    let tools: Vec<Value> = registry
        .list()
        .into_iter()
        .filter(|tool| registry.allowed(tool.name, &req.capabilities, req.allowed_tools.as_deref()))
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": {"type": "object", "additionalProperties": true}
                }
            })
        })
        .collect();
    if tools.is_empty() {
        return Ok(PlanAction::Generate);
    }

    let (endpoint, api_key, model, engine) = {
        let config = state
            .config
            .read()
            .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
        (
            req.base
                .endpoint
                .clone()
                .unwrap_or_else(|| config.endpoint.clone()),
            req.base.api_key.clone().or_else(|| config.api_key.clone()),
            req.base
                .model
                .clone()
                .unwrap_or_else(|| config.model.clone()),
            config.engine.clone(),
        )
    };
    let system = "You are AIRP's control-plane planner. Select one function only when it is necessary to satisfy the user request. If no tool is needed, return a normal assistant message. Never write roleplay prose.";
    let user = serde_json::to_string(&serde_json::json!({
        "request": req.base.message,
        "observations": observations,
    }))?;
    let mut request = match &engine {
        crate::adapter::BackendEngine::Direct => {
            state.http_client.post(endpoint).json(&serde_json::json!({
                "model": model,
                "stream": false,
                "temperature": 0,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": user}
                ],
                "tools": tools,
                "tool_choice": "auto"
            }))
        }
        crate::adapter::BackendEngine::AnthropicMessages => {
            let anthropic_tools: Vec<Value> = tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool["function"]["name"],
                        "description": tool["function"]["description"],
                        "input_schema": tool["function"]["parameters"]
                    })
                })
                .collect();
            state
                .http_client
                .post(endpoint)
                .header("anthropic-version", "2023-06-01")
                .json(&serde_json::json!({
                    "model": model,
                    "max_tokens": 512,
                    "temperature": 0,
                    "system": system,
                    "messages": [{"role": "user", "content": user}],
                    "tools": anthropic_tools,
                    "tool_choice": {"type": "auto"}
                }))
        }
        crate::adapter::BackendEngine::ClaudeCodeSdk => {
            return Err(AirpError::Config(
                "ClaudeCodeSdk structured planner is not implemented".to_string(),
            ));
        }
    };
    if let Some(api_key) = api_key.filter(|key| !key.is_empty()) {
        request = match &engine {
            crate::adapter::BackendEngine::AnthropicMessages => {
                request.header("x-api-key", api_key)
            }
            _ => request.bearer_auth(api_key),
        };
    }
    let response = request
        .timeout(Duration::from_secs(req.wall_clock_secs.max(1)))
        .send()
        .await?;
    // #117 A：redirect 拒截先于 success/4xx/5xx 分流，typed 升级避免凭据泄露旁路。
    let response = if let Some(classified) = crate::outbound::classify_redirect_response(&response)
    {
        return Err(classified);
    } else {
        response
    };
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(AirpError::Upstream {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    let payload: Value = serde_json::from_slice(&bytes)?;
    let Some((tool, params)) = decode_tool_call(&engine, &payload)? else {
        return Ok(PlanAction::Generate);
    };
    Ok(PlanAction::CallTool { tool, params })
}

fn decode_tool_call(
    engine: &crate::adapter::BackendEngine,
    payload: &Value,
) -> Result<Option<(String, Value)>, AirpError> {
    let call = match engine {
        crate::adapter::BackendEngine::Direct => payload
            .pointer("/choices/0/message/tool_calls/0/function")
            .and_then(Value::as_object)
            .cloned(),
        crate::adapter::BackendEngine::AnthropicMessages => payload
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| blocks.iter().find(|block| block["type"] == "tool_use"))
            .and_then(Value::as_object)
            .map(|block| {
                serde_json::Map::from_iter([
                    ("name".to_string(), block["name"].clone()),
                    ("arguments".to_string(), block["input"].clone()),
                ])
            }),
        crate::adapter::BackendEngine::ClaudeCodeSdk => None,
    };
    let Some(call) = call else {
        return Ok(None);
    };
    let tool = call
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AirpError::BadRequest("tool call missing function.name".to_string()))?
        .to_string();
    let params = match call.get("arguments") {
        Some(Value::String(arguments)) => serde_json::from_str(arguments)?,
        Some(value) if value.is_object() => value.clone(),
        _ => serde_json::json!({}),
    };
    Ok(Some((tool, params)))
}

async fn emit_done(
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
    reason: StopReason,
    steps_taken: u32,
    tokens_estimated: u64,
) -> Option<StopReason> {
    let _ = tx
        .send(AgentEvent::Done {
            stop_reason: reason.clone(),
            steps_taken,
            tokens_estimated,
        })
        .await;
    Some(reason)
}

// ── 不变式测试（戒律#6 可验证）──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 戒律#6（§4.2）：派生 subagent 用的请求 = 用户原始 base 请求，
    /// 协调器多步状态（已调工具 / observe）**不注入** base 的 system prompt 或 messages。
    ///
    /// M_AGENT-1 骨架里协调器不修改 `req.base`（只读引用传给 prepare_pipeline），
    /// 故这条不变式在骨架阶段由"不写修改代码"保证。本测试断言：AgentRunRequest
    /// 的 base 字段经 serde round-trip 后，system_prompt 注入点（character_card_id /
    /// lorebook_path / message）不含协调器控制平面标记（"tool" / "plan" / "observe"）。
    #[test]
    fn subagent_context_has_no_orchestrator_noise() {
        let req = AgentRunRequest {
            base: ChatCompletionRequest {
                character_id: None,
                character_card_id: Some(
                    serde_json::json!({
                        "name": "Alice",
                        "description": "a knight"
                    })
                    .to_string(),
                ),
                lorebook_path: None,
                user_profile: crate::daemon::UserProfile {
                    name: "User".to_string(),
                    variables: std::collections::HashMap::new(),
                },
                message: "你好".to_string(),
                messages_history: None,
                regex_filters: None,
                preset_id: None,
                enabled_presets: None,
                session_id: None,
                provider: None,
                endpoint: None,
                api_key: None,
                model: None,
                temperature: None,
                max_tokens: None,
                scene_id: None,
                user_id: None,
                persona_id: None,
                swipe_candidates: Vec::new(),
                branch_from: None,
            },
            max_steps: 3,
            token_budget: None,
            wall_clock_secs: 60,
            capabilities: vec![],
            allowed_tools: None,
            confirm_tools: vec![],
        };

        // 角色平面字段（进 system prompt 的种子）
        let plane_seeds = [
            req.base.character_card_id.as_deref().unwrap_or(""),
            &req.base.message,
        ];
        let noise_markers = ["tool_call", "plan_action", "observe", "orchestrator"];
        for seed in &plane_seeds {
            for marker in &noise_markers {
                assert!(
                    !seed.to_lowercase().contains(marker),
                    "戒律#6 破裂：角色平面种子含协调器噪声标记 `{}`",
                    marker
                );
            }
        }
    }

    /// max_steps=1 时不进 loop 序列（退化单回合）。
    #[test]
    fn max_steps_one_is_single_turn() {
        // run_loop 内部依 max_steps 选 plan；这里仅断言默认值语义。
        assert_eq!(default_max_steps(), 1);
    }

    /// #26：把神圣不变式压到真实管线产物上。
    ///
    /// 旧测试只查 `req.base` 的种子字符串；本测试走 loop 派生 subagent 的
    /// **同一条** `prepare_pipeline` 路径（run_loop 的 Generate 分支，
    /// `engine/src/agent/mod.rs` Generate → prepare_pipeline(&req.base, state)），
    /// 断言装配出的最终 `system_prompt` / `messages` 不含协调器控制平面标记。
    /// 未来 M_AGENT/ReAct 改动若把 plan/tool/observe 状态误注入角色平面，此处立即红。
    #[test]
    fn subagent_prepared_pipeline_has_no_orchestrator_noise() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path().to_path_buf();
        for d in ["characters", "presets", "sessions"] {
            std::fs::create_dir_all(data_root.join(d)).unwrap();
        }
        let state = Arc::new(DaemonState {
            data_root,
            http_client: reqwest::Client::new(),
            fts: Default::default(),
            settings_update: Default::default(),
            config: std::sync::RwLock::new(crate::daemon::MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://127.0.0.1:1/v1/chat/completions".to_string(),
                api_key: None,
                model: "test-model".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });

        let card = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "a knight",
                "personality": "", "scenario": "", "first_mes": "Hello!",
                "mes_example": "", "creator_notes": "", "system_prompt": "",
                "post_history_instructions": "", "tags": [], "creator": "",
                "character_version": "", "alternate_greetings": [], "extensions": {}
            }
        })
        .to_string();

        let req = AgentRunRequest {
            base: ChatCompletionRequest {
                character_id: None,
                character_card_id: Some(card),
                lorebook_path: None,
                user_profile: crate::daemon::UserProfile {
                    name: "User".to_string(),
                    variables: std::collections::HashMap::new(),
                },
                message: "你好".to_string(),
                messages_history: None,
                regex_filters: None,
                preset_id: None,
                enabled_presets: None,
                session_id: None,
                provider: None,
                endpoint: None,
                api_key: None,
                model: None,
                temperature: None,
                max_tokens: None,
                scene_id: None,
                user_id: None,
                persona_id: None,
                swipe_candidates: Vec::new(),
                branch_from: None,
            },
            max_steps: 3,
            token_budget: None,
            wall_clock_secs: 60,
            capabilities: vec![],
            allowed_tools: None,
            confirm_tools: vec![],
        };

        // 与 run_loop Generate 分支完全相同的调用形态。
        let pipeline = prepare_pipeline(&req.base, &state).expect("prepare_pipeline");

        // 控制平面标记：loop 协议字段名 + 骨架 echo 探针参数。
        // 任何一个出现在角色平面即视为戒律#6 破裂。
        let noise_markers = [
            "tool_call",
            "tool_result",
            "plan_action",
            "observe",
            "orchestrator",
            "loop-skeleton",
            "stop_reason",
            "steps_taken",
            "dry_run",
        ];
        let mut plane = vec![("system_prompt".to_string(), pipeline.system_prompt.clone())];
        for (i, m) in pipeline.messages.iter().enumerate() {
            plane.push((format!("messages[{}]", i), m.content.clone()));
        }
        for (loc, text) in &plane {
            let lower = text.to_lowercase();
            for marker in &noise_markers {
                assert!(
                    !lower.contains(marker),
                    "戒律#6 破裂：{} 含协调器噪声标记 `{}`",
                    loc,
                    marker
                );
            }
        }

        // 正向 sanity：装配确实跑了（角色平面含卡内容，而不是空 prompt 侥幸通过）。
        let all_text = plane
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("Alice"),
            "装配产物应包含角色卡内容（防止空产物假绿）"
        );
        assert!(
            plane.iter().any(|(_, t)| t.contains("你好")),
            "装配产物应包含用户消息"
        );
    }

    /// AgentEvent / PlanAction 序列化 wire-shape 守门员（issue #43/#44/#45/#46 T 建议）。
    ///
    /// PR #41 曾因前端按 PascalCase (`action.CallTool`) 读 snake_case serde
    /// (`{"call_tool":{...}}`) 导致 PLAN 摘要全 fallback。本 test 锁死 wire 形态，
    /// 未来前端/契约改动若与此处漂移会立即红。
    #[test]
    fn agent_event_wire_shape() {
        // PlanAction: externally-tagged，snake_case
        assert_eq!(
            serde_json::to_value(PlanAction::Generate).unwrap(),
            serde_json::json!("generate")
        );
        assert_eq!(
            serde_json::to_value(PlanAction::Finish).unwrap(),
            serde_json::json!("finish")
        );
        assert_eq!(
            serde_json::to_value(PlanAction::CallTool {
                tool: "echo".to_string(),
                params: serde_json::json!({"probe": "x"}),
            })
            .unwrap(),
            serde_json::json!({"call_tool": {"tool": "echo", "params": {"probe": "x"}}})
        );

        // AgentEvent: #[serde(tag = "type", rename_all = "snake_case")]
        let plan = serde_json::to_value(AgentEvent::Plan {
            step: 2,
            action: PlanAction::CallTool {
                tool: "echo".to_string(),
                params: serde_json::json!({}),
            },
        })
        .unwrap();
        assert_eq!(plan["type"], "plan");
        assert_eq!(plan["step"], 2);
        assert_eq!(plan["action"]["call_tool"]["tool"], "echo");

        let done = serde_json::to_value(AgentEvent::Done {
            stop_reason: StopReason::UpstreamError,
            steps_taken: 1,
            tokens_estimated: 42,
        })
        .unwrap();
        assert_eq!(done["type"], "done");
        assert_eq!(done["stop_reason"], "upstream_error");
        assert_eq!(done["steps_taken"], 1);
        assert_eq!(done["tokens_estimated"], 42);
    }

    #[test]
    fn structured_tool_call_codecs_decode_to_one_internal_shape() {
        let openai = serde_json::json!({
            "choices": [{"message": {"tool_calls": [{"function": {
                "name": "echo", "arguments": "{\"probe\":\"openai\"}"
            }}]}}]
        });
        let anthropic = serde_json::json!({
            "content": [{"type": "tool_use", "name": "echo", "input": {"probe": "anthropic"}}]
        });
        let (name, params) = decode_tool_call(&crate::adapter::BackendEngine::Direct, &openai)
            .unwrap()
            .unwrap();
        assert_eq!(name, "echo");
        assert_eq!(params["probe"], "openai");
        let (name, params) = decode_tool_call(
            &crate::adapter::BackendEngine::AnthropicMessages,
            &anthropic,
        )
        .unwrap()
        .unwrap();
        assert_eq!(name, "echo");
        assert_eq!(params["probe"], "anthropic");
    }
}
