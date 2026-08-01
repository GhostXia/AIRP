//! Finalize phase: persist assistant message, live state, and volume side-effects.
//!
//! `run_finalize` 是 stream / stdout / generation_step 三条路径共用的提交点。
//! 关键纪律：用户消息已在 prepare 阶段先落盘，assistant 消息 / live state /
//! current.md / 封卷 / 维护任一失败都硬失败，绝不向客户端发送虚假 `done`。
//! #249 审计 B1 修复落点也在此：stripped 为空时回灌旧 swipe 候选，避免用户
//! 资产永久丢失。
//!
//! 2.2 自动事实抽取：finalize 后异步触发，从本轮对话中抽取关键事实写入 resident.md。

use crate::adapter::ChatMessage;
use crate::domain::ChatService;
use crate::error::AirpError;
use crate::{volume_manager, volume_store};

use super::state_extract::extract_state_content;
use super::types::FinalizerCtx;

/// #290 F-2：风格审查自动触发间隔（轮数）。读 env `AIRP_STYLE_REVIEW_INTERVAL`，
/// 默认 10；0 = 禁用自动审查。与 adapter 的 `AIRP_CHAT_REQUEST_TIMEOUT_MS` 同模式。
fn style_review_interval() -> u64 {
    const DEFAULT: u64 = 10;
    std::env::var("AIRP_STYLE_REVIEW_INTERVAL")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT)
}

// ── finalize ──────────────────────────────────────────────────────────────────

pub(super) async fn run_finalize(
    mut ctx: FinalizerCtx,
    raw_acc: String,
    cleaned_acc: String,
) -> Result<(), AirpError> {
    // A2-1: credit estimated LLM output tokens toward the per-(user)-root daily
    // quota. `ctx.data_root` is the effective root (DX-1 per-user isolation), so
    // record_tokens writes the same quota.json that check_and_increment gated on.
    // raw_acc = full raw generation (pre-filter), the truest proxy for billed
    // output. Best-effort: record_tokens never blocks a completed response.
    let out_tokens = crate::volume_store::estimate_tokens(&raw_acc);
    crate::quota::record_tokens_async(&ctx.data_root, out_tokens.min(u32::MAX as usize) as u32)
        .await;

    // (1) Persist assistant message to ChatLog
    //     M_LS-1: strip <state>…</state> before persisting; side-persist state/live.json.
    //     审计 Bug D 修复：先追加 assistant 消息到 chat_log.jsonl，再持久化
    //     live state。旧顺序（state → message）在 message append 失败时：state
    //     已更新但 chat log 无对应消息，下次 prepare 读历史时无法对账——
    //     state 反映了一条用户从未见到的助手回复。新顺序（message → state）
    //     在 state persist 失败时：消息已落盘用户可见，state 略滞后但下轮
    //     可重新抽取；消息丢失比 state 滞后更不可恢复。
    if let Some(ref cid) = ctx.character_id {
        let (stripped, live_state) = extract_state_content(&cleaned_acc);
        if !stripped.trim().is_empty() {
            if let Some(snapshot) = ctx.regen_snapshot.as_ref() {
                if !ctx
                    .session_operation_lease
                    .as_ref()
                    .is_some_and(|lease| lease.matches_generation(&snapshot.generation_id))
                {
                    return Err(AirpError::Conflict("generation_lease_lost".to_string()));
                }
                ChatService::new(&ctx.data_root).commit_regen(
                    cid,
                    ctx.session_id.as_ref(),
                    snapshot,
                    &stripped,
                )?;
            } else if ctx.continue_mode {
                // Continue: append generated text to the existing last assistant message.
                ChatService::new(&ctx.data_root).append_to_last(
                    cid,
                    ctx.session_id.as_ref(),
                    &stripped,
                )?;
            } else if !ctx.swipe_candidates.is_empty() {
                // #249 Swipe: regen 时捕获了旧候选，将新生成文本追加为最后一个候选。
                let mut candidates = ctx.swipe_candidates.clone();
                candidates.push(stripped);
                ChatService::new(&ctx.data_root).append_with_candidates(
                    cid,
                    ctx.session_id.as_ref(),
                    candidates,
                )?;
            } else {
                ChatService::new(&ctx.data_root).append(
                    cid,
                    ctx.session_id.as_ref(),
                    ChatMessage {
                        role: crate::adapter::MessageRole::Assistant,
                        content: stripped,
                    },
                )?;
            }
        } else if ctx.regen_snapshot.is_none() && !ctx.swipe_candidates.is_empty() {
            // Legacy swipe finalization path: if a generated body is empty,
            // restore the candidate list rather than losing it.
            ChatService::new(&ctx.data_root).append_with_candidates(
                cid,
                ctx.session_id.as_ref(),
                ctx.swipe_candidates.clone(),
            )?;
        }
        // 先确认 assistant 消息成功落盘，再持久化 live state。
        // 若上面的消息追加失败（`?` 传播 Err），state 不会被写入，
        // 避免 live.json 领先于 chat_log 的不一致。
        if let Some(ref state) = live_state {
            persist_live_state(&ctx.data_root, cid.as_str(), state).await?;
        }
    }

    // The ChatLog and required live-state mutation are complete. Keep slow
    // volume/maintenance work outside the session generation contract so a
    // secondary LLM call cannot hold a session busy indefinitely.
    if let Some(mut lease) = ctx.session_operation_lease.take() {
        lease.release();
    }

    // (2) Volume side-effects
    if let Some(sd) = ctx.session_dir {
        let (cleaned, signal) = volume_manager::parse_seal_signal(&raw_acc);

        if !cleaned.trim().is_empty() {
            // R3: 旧实现 `let _ = ...` 静默吞掉 `append_to_current` 的错误，
            // 包括磁盘满、权限拒绝、`commit_memory_revision` 因并发 commit
            // 同号 revision 被拒等。结果：刚生成的助手消息对客户端已可见，
            // 但 `current.md` 与 memory revision 都没记录，用户体感为"AI 忘了
            // 刚才说过什么"。因此改为硬失败，只有关键持久化全部成功后
            // 才向客户端发送 done；详细错误仅写内部日志。
            volume_store::append_to_current(&sd, &cleaned)?;
            // Phase 2.1: 导演指令单轮交付——assistant 消息成功写入后
            // 清除 directive 文件，使非 observe 指令只生效一轮。
            // 如果生成失败（走不到这里），指令保持 pending，下轮重新注入。
            crate::agent::director::acknowledge_directive(&sd);
        }

        let should_seal = signal.as_ref().map(|s| s.should_seal).unwrap_or(false)
            || volume_manager::should_force_seal(&sd, ctx.volume_config.hard_threshold_tokens);

        // JoinSet 结构化管理：封卷 + 维护子任务，finalize 等待两者完成。
        let mut join_set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

        if should_seal {
            let sd_clone = sd.clone();
            // M4.2：封卷派生新 gen_params（覆盖 temperature / 可选 model）；
            // provider_config 直接复用同一 Arc，连接层不变。
            let mut seal_params = ctx.gen_params.clone();
            seal_params.temperature = Some(ctx.volume_config.seal_temperature);
            if let Some(model_override) = ctx.volume_config.seal_model.clone() {
                seal_params.model = model_override;
            }
            let seal_provider = ctx.provider_config.clone();
            let seal_client = ctx.http_client.clone();
            // #283：传 character_id + session_id 给 run_seal_flow，使其能在写盘段
            // 持 session_lock + baseline 校验。scene 模式（character_id=None）时
            // run_seal_flow 内部跳过持锁，保持既有行为。
            let seal_character_id = ctx.character_id.as_ref().map(|c| c.as_str().to_string());
            let seal_session_id = ctx.session_id;
            join_set.spawn(async move {
                if let Err(e) = volume_manager::run_seal_flow(
                    &seal_client,
                    &sd_clone,
                    seal_character_id.as_deref(),
                    seal_session_id.as_ref(),
                    seal_provider,
                    seal_params,
                )
                .await
                {
                    tracing::error!(err = %e, "封卷流程失败");
                }
            });
        }

        if let Ok(turn_count) = volume_store::increment_turn_counter(&sd) {
            let interval = ctx.volume_config.maintenance_interval.max(1) as u64;
            // 审计 Bug E 修复：当本轮触发封卷（should_seal=true）时，跳过
            // 周期维护。run_seal_flow 与 run_maintenance 都对 index.md 做
            // read-modify-write（seal 在 volume_manager.rs:219/280，
            // maintenance 在 430/470），并发执行时后写者覆盖先写者的 diff，
            // 导致卷索引或跨卷实体晋升丢失。封卷已涵盖本轮的卷过渡，维护
            // 推迟到下一个非封卷轮次触发。
            if !should_seal && turn_count > 0 && turn_count % interval == 0 {
                let sd_maint = sd.clone();
                join_set.spawn(async move {
                    if let Err(e) = volume_manager::run_maintenance(&sd_maint) {
                        tracing::error!(err = %e, "维护任务失败");
                    }
                });
            }

            // #290 F-2：风格审查自动触发。每 N 轮一次（N 由 env
            // AIRP_STYLE_REVIEW_INTERVAL 控制，默认 10，0 = 禁用）。best-effort。
            let review_interval = style_review_interval();
            if review_interval > 0 && turn_count > 0 && turn_count % review_interval == 0 {
                if let Some(ref cid) = ctx.character_id {
                    let data_root = ctx.data_root.clone();
                    let cid_clone = cid.clone();
                    let session_id = ctx.session_id;
                    let provider_config = ctx.provider_config.clone();
                    let gen_params = ctx.gen_params.clone();
                    let http_client = ctx.http_client.clone();
                    tokio::spawn(async move {
                        match crate::style::run_style_review_for_character(
                            &http_client,
                            provider_config,
                            gen_params,
                            &data_root,
                            &cid_clone,
                            session_id.as_ref(),
                        )
                        .await
                        {
                            Ok(true) => tracing::info!("风格审查已应用 drift"),
                            Ok(false) => {}
                            Err(e) => {
                                tracing::warn!(err = %e, "风格审查失败（best-effort）")
                            }
                        }
                    });
                }
            }
        }

        // 2.2 自动事实抽取：异步触发，best-effort。
        // 从本轮 user+assistant 对话中抽取关键事实写入 resident.md。
        if let Some(ref cid) = ctx.character_id {
            let sd_extract = sd.clone();
            let data_root = ctx.data_root.clone();
            let cid_clone = cid.clone();
            let session_id = ctx.session_id;
            let provider_config = ctx.provider_config.clone();
            let gen_params = ctx.gen_params.clone();
            let http_client = ctx.http_client.clone();
            let assistant_content = cleaned_acc.clone();

            join_set.spawn(async move {
                if let Err(e) = run_memory_extraction(
                    &http_client,
                    provider_config,
                    gen_params,
                    &data_root,
                    &cid_clone,
                    session_id.as_ref(),
                    &sd_extract,
                    &assistant_content,
                )
                .await
                {
                    tracing::warn!(err = %e, "记忆抽取失败（best-effort）");
                }
            });
        }

        // 等待全部子任务结束；JoinError（panic / cancel）单独 tracing
        while let Some(res) = join_set.join_next().await {
            if let Err(je) = res {
                if je.is_panic() {
                    tracing::error!(err = %je, "封卷/维护子任务 panic");
                } else if je.is_cancelled() {
                    tracing::warn!("封卷/维护子任务被取消");
                }
            }
        }
    }

    // 阶段二补全 D1：用户模型自动抽取。仅当 user_id 存在时（此时
    // data_root 已是该用户独立根），异步 best-effort 抽取用户偏好。
    //
    // CodeRabbit #1+#2：此任务不依赖 session_dir（只用 data_root /
    // character_id / session_id），故移出 volume side-effects 块。
    // 保持 fire-and-forget，避免用户模型抽取拖住 run_finalize。
    // best-effort：失败只 tracing，不影响主流程。
    if ctx.user_id.is_some() {
        let data_root = ctx.data_root.clone();
        let session_id = ctx.session_id;
        let character_id = ctx.character_id.clone();
        let provider_config = ctx.provider_config.clone();
        let gen_params = ctx.gen_params.clone();
        let http_client = ctx.http_client.clone();
        let assistant_content = cleaned_acc.clone();

        tokio::spawn(async move {
            if let Err(e) = run_user_model_extraction(
                &http_client,
                provider_config,
                gen_params,
                &data_root,
                character_id.as_ref(),
                session_id.as_ref(),
                &assistant_content,
            )
            .await
            {
                tracing::warn!(err = %e, "用户模型抽取失败（best-effort）");
            }
        });
    }

    Ok(())
}

/// Writes `state` to `characters/{character_id}/state/live.json` (overwrite).
///
/// Failures are silently logged; state persistence is best-effort.
pub(super) async fn persist_live_state(
    data_root: &std::path::Path,
    character_id: &str,
    state: &serde_json::Value,
) -> Result<(), AirpError> {
    let character = crate::types::CharacterId::new(character_id)?;
    crate::domain::StateService::new(data_root)
        .write(&character, state)
        .map(|_| ())
}

/// Commit one converged Agent generation through the same persistence, state,
/// volume, and maintenance finalizer used by the ordinary chat pipeline.
pub async fn finalize_generation(finalizer: FinalizerCtx, raw_acc: String, cleaned_acc: String) {
    if let Err(error) = run_finalize(finalizer, raw_acc, cleaned_acc).await {
        tracing::error!(%error, "agent generation finalization failed");
    }
}

/// 2.2 自动事实抽取：从本轮对话中抽取关键事实写入 resident.md。
///
/// Best-effort：失败不影响主流程。
#[allow(clippy::too_many_arguments)]
async fn run_memory_extraction(
    client: &reqwest::Client,
    provider_config: std::sync::Arc<crate::adapter::ProviderConfig>,
    gen_params: crate::adapter::GenerationParams,
    data_root: &std::path::Path,
    character_id: &crate::types::CharacterId,
    session_id: Option<&crate::types::SessionId>,
    session_dir: &std::path::Path,
    assistant_content: &str,
) -> Result<(), AirpError> {
    use crate::memory::{extract_facts, ExtractionConfig};

    // 读取最后一条 user 消息
    let history = ChatService::new(data_root).history(character_id, session_id)?;
    let last_user_msg = history
        .messages
        .iter()
        .rev()
        .find(|m| m.role == crate::adapter::MessageRole::User)
        .map(|m| m.content.as_str())
        .unwrap_or("");

    if last_user_msg.is_empty() || assistant_content.trim().is_empty() {
        return Ok(());
    }

    // 抽取事实
    let config = ExtractionConfig::default();
    let facts = extract_facts(
        client,
        provider_config.clone(),
        gen_params.clone(),
        data_root,
        last_user_msg,
        assistant_content,
        &config,
    )
    .await?;

    if facts.trim().is_empty() {
        return Ok(());
    }

    // 检查新抽取的条目是否已存在于 decay store 中：
    // 已存在的条目调用 reinforce_entry 刷新 last_reinforced 和 importance，
    // 避免重复追加相同内容到 resident.md
    let new_lines: Vec<&str> = facts
        .lines()
        .filter(|l| l.trim().starts_with("- "))
        .collect();
    let existing_store = crate::memory::decay::load_decay_store(session_dir);
    let mut facts_to_append = Vec::new();
    for line in &new_lines {
        let hash = crate::memory::decay::line_hash(line);
        if existing_store.contains_key(&hash) {
            // 已存在：强化而非重复追加
            crate::memory::decay::reinforce_entry(session_dir, line)?;
        } else {
            facts_to_append.push(*line);
        }
    }

    if !facts_to_append.is_empty() {
        let new_facts = facts_to_append.join("\n");
        crate::memory::append_resident_memory(session_dir, &new_facts)?;
    }

    // Phase 2.5: 遗忘曲线衰减 pass（压缩前先淡出低权重条目）
    let decay_config = crate::memory::DecayConfig::default();
    let content_before_decay = crate::memory::read_resident_memory(session_dir)?;
    if !content_before_decay.trim().is_empty() {
        match crate::memory::apply_decay(session_dir, &content_before_decay, &decay_config) {
            Ok(result) => {
                if result.faded_count > 0 {
                    tracing::info!(
                        faded = result.faded_count,
                        total = result.total_entries,
                        "resident memory 遗忘曲线：淡出 {} 条低权重记忆",
                        result.faded_count
                    );
                    crate::memory::write_resident_memory(session_dir, &result.retained)?;
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "decay pass 失败，跳过");
            }
        }
    }

    // 检查是否需要压缩
    let resident_config = crate::memory::ResidentMemoryConfig::default();
    if crate::memory::is_over_capacity(session_dir, &resident_config) {
        tracing::info!("resident memory 超过容量上限，触发压缩");
        let content = crate::memory::read_resident_memory(session_dir)?;
        let compressed = crate::memory::compress_resident_memory(
            client,
            provider_config.clone(),
            gen_params.clone(),
            &content,
            resident_config.capacity_chars,
        )
        .await?;
        crate::memory::write_resident_memory(session_dir, &compressed)?;
    }

    Ok(())
}

/// 阶段二补全 D1：用户模型自动抽取。从本轮对话中抽取用户偏好，
/// 写入用户主目录（effective root）下的 user_model.md。
///
/// Best-effort：失败不影响主流程。`data_root` 必须是用户独立根
/// （`data/users/{uid}/`），调用前已由 `ctx.user_id.is_some()` 保证。
#[allow(clippy::too_many_arguments)]
async fn run_user_model_extraction(
    client: &reqwest::Client,
    provider_config: std::sync::Arc<crate::adapter::ProviderConfig>,
    gen_params: crate::adapter::GenerationParams,
    data_root: &std::path::Path,
    character_id: Option<&crate::types::CharacterId>,
    session_id: Option<&crate::types::SessionId>,
    assistant_content: &str,
) -> Result<(), AirpError> {
    use crate::memory::{extract_user_preferences, ExtractionConfig};

    // scene 模式下 character_id 为 None，无法读取历史，跳过。
    let Some(cid) = character_id else {
        return Ok(());
    };

    // 读取最后一条 user 消息
    let history = ChatService::new(data_root).history(cid, session_id)?;
    let last_user_msg = history
        .messages
        .iter()
        .rev()
        .find(|m| m.role == crate::adapter::MessageRole::User)
        .map(|m| m.content.as_str())
        .unwrap_or("");

    if last_user_msg.is_empty() || assistant_content.trim().is_empty() {
        return Ok(());
    }

    // 抽取用户偏好
    let config = ExtractionConfig::default();
    let prefs = extract_user_preferences(
        client,
        provider_config,
        gen_params,
        data_root,
        last_user_msg,
        assistant_content,
        &config,
    )
    .await?;

    if prefs.trim().is_empty() {
        return Ok(());
    }

    // 追加到用户模型（data_root 即用户主目录，容量强制在内部完成）。
    crate::memory::append_user_model_in_home(data_root, &prefs)?;

    Ok(())
}
