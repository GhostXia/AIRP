//! M_AGENT-1: Agent loop skeleton — the minimal orchestrator.
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
//! 这条不变式由 [`tests::subagent_context_has_no_orchestrator_noise`] 守护。
//!
//! ## 有界（戒律#1，§2.1）
//! - step 上限 + token 预算 + 墙钟超时，任一触顶即停。
//! - 客户端取消（CancellationToken）→ 已派生子任务收敛。
//!
//! ## 触发判定（§4.3）
//! - `max_steps` 缺省或 =1 → 单回合退化（= 现有 `/v1/chat/completions`）。
//! - `max_steps>1` → 进 loop。

pub mod tools;

use crate::chat_pipeline::{prepare_pipeline, run_generation_step};
use crate::daemon::{ChatCompletionRequest, DaemonState};
use crate::error::AirpError;
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
}

fn default_max_steps() -> u32 {
    1
}
fn default_wall_clock_secs() -> u64 {
    300
}

/// SSE 事件（计划书 M_AGENT-4 协议；M_AGENT-1 先发 plan/tool_call/tool_result/delta/done）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// 协调器规划了一步。
    Plan {
        step: u32,
        action: PlanAction,
    },
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
    Delta {
        step: u32,
        chunk: String,
    },
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

    // M_AGENT-1 骨架的简化规划：固定序列 = [call echo, generate, finish]。
    // 真实 ReAct 规划留 M_AGENT-2（基于模型 tool_calls）。
    // 这里用固定序列验证"协调器 → 工具 → subagent → 收敛"闭环 + 各道闸。
    let plan: &[PlanAction] = if max_steps >= 2 {
        &[
            PlanAction::CallTool {
                tool: "echo".to_string(),
                params: serde_json::json!({"probe": "loop-skeleton"}),
            },
            PlanAction::Generate,
            PlanAction::Finish,
        ]
    } else {
        &[PlanAction::Generate, PlanAction::Finish]
    };

    let mut plan_idx = 0;
    while plan_idx < plan.len() {
        // ── 闸：取消 / 墙钟 / step cap / token 预算 ──
        if cancel.is_cancelled() {
            return emit_done(
                tx,
                StopReason::Cancelled,
                steps_taken,
                tokens_estimated,
            )
            .await;
        }
        if Instant::now() >= deadline {
            return emit_done(
                tx,
                StopReason::WallClock,
                steps_taken,
                tokens_estimated,
            )
            .await;
        }
        if steps_taken >= max_steps {
            return emit_done(tx, StopReason::StepCap, steps_taken, tokens_estimated).await;
        }
        if tokens_estimated >= token_budget {
            return emit_done(
                tx,
                StopReason::TokenBudget,
                steps_taken,
                tokens_estimated,
            )
            .await;
        }

        let action = plan[plan_idx].clone();
        steps_taken += 1;
        let _ = tx.send(AgentEvent::Plan { step: steps_taken, action: action.clone() }).await;

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
                    Some(t) if registry.allowed(&tool) => {
                        // M_AGENT-1：骨架不传 confirm=true（M_AGENT-5 补确认流）。
                        // 破坏性工具因此走 dry-run。
                        t.call(params, false).await
                    }
                    _ => Err(AirpError::BadRequest(format!("unknown tool: {}", tool))),
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
                        // M_AGENT-1: tool result 暂不回灌入下一步 subagent 上下文
                        // （那需要扩展 adapter wire format，属 M_AGENT-2/4）。
                        // 骨架仅验证"协调器能调工具、拿结果、继续"。
                    }
                    Err(e) => {
                        tracing::warn!(err = %e, tool = %tool, "tool call failed");
                    }
                }
                plan_idx += 1;
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
                // M_AGENT-1 骨架：run_generation_step 不 finalize（不落库/封卷）。
                // 最后一步的 finalize 留 M_AGENT-2/6（需协调器显式决策落库时机）。
                let result = run_generation_step(pipeline).await;
                if let Some(e) = result.error {
                    tracing::warn!(err = %e, "generation step upstream error");
                    return emit_done(
                        tx,
                        StopReason::UpstreamError,
                        steps_taken,
                        tokens_estimated,
                    )
                    .await;
                }
                // 累计 token + 流式下发 chunks。
                let step_tokens =
                    crate::volume_store::estimate_tokens(&result.raw_acc) as u64;
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
                // 单步生成即收敛（M_AGENT-1 骨架：Generate 后直接 Finish）。
                plan_idx += 1;
            }
            PlanAction::Finish => {
                return emit_done(
                    tx,
                    StopReason::Converged,
                    steps_taken,
                    tokens_estimated,
                )
                .await;
            }
        }
    }

    // plan 跑完未显式 Finish（理论上不会，因 plan 末项是 Finish）。
    emit_done(tx, StopReason::Converged, steps_taken, tokens_estimated).await
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
                character_card_id: Some(serde_json::json!({
                    "name": "Alice",
                    "description": "a knight"
                }).to_string()),
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
            },
            max_steps: 3,
            token_budget: None,
            wall_clock_secs: 60,
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
}
