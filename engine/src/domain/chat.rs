//! Chat session domain service: message append, rollback, regen, swipe, and
//! branching conversation tree management.
//!
//! Extracted from `domain/mod.rs` (E-P1-1 slice 5). Zero behavior change.

use std::path::{Path, PathBuf};

use crate::adapter::ChatMessage;
use crate::chat_store::ChatLog;
use crate::data_dir;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};
use crate::ulid;

use super::lock_order;
use super::locks::{
    character_lock, remove_deleted_character_lock, remove_deleted_session_lock,
    remove_deleted_state_lock, session_lock,
};

/// Immutable target state captured before a regen proposal is generated.
#[derive(Debug, Clone)]
pub(crate) struct RegenSnapshot {
    pub(crate) generation_id: String,
    pub(crate) target_message_id: String,
    pub(crate) revision: u64,
    pub(crate) content: String,
    /// Exact persisted representation; an empty vector is distinct from a
    /// one-item explicit candidate list for stale-snapshot comparison.
    pub(crate) stored_candidates: Vec<String>,
    pub(crate) candidates: Vec<String>,
    pub(crate) swipe_index: usize,
}

#[derive(Clone, Debug)]
pub struct ChatService {
    data_root: PathBuf,
}

/// #37 cursor 分页窗口（`ChatService::history_window` 返回）。
///
/// `messages` / `message_ids` / `message_timestamps` 等长，按时间正序排列，
/// 是原 session 的一个切片（更早的一段或最近的一段）。
///
/// - `has_more`：cursor 之前还有更早消息可加载。
/// - `oldest_id`：本窗口最老消息的 durable ID，前端下次作 `before` cursor。
/// - `total`：session 消息总数（含未加载），前端显示 "X / N"。
/// - `scope_session_id`：#85 O1——当前 window 所属的 scope session id（None = legacy），
///   前端用它关联 session 列表。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryWindow {
    pub messages: Vec<ChatMessage>,
    pub message_ids: Vec<String>,
    pub message_timestamps: Vec<Option<String>>,
    /// #249 Swipe：每条消息的候选回复列表。空 Vec = 单候选（content 即唯一候选）。
    pub message_candidates: Vec<Vec<String>>,
    /// #249 Swipe：每条消息当前激活候选的下标（0-based）。
    pub message_swipe_index: Vec<usize>,
    /// 分支对话树：每条消息的父消息 durable ID。
    pub message_parents: Vec<Option<String>>,
    /// 分支对话树：当前激活路径的叶节点 durable ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_leaf: Option<String>,
    /// 分支对话树：当前激活路径（根 → 叶的 durable ID 列表）。
    pub active_path: Vec<String>,
    pub has_more: bool,
    pub oldest_id: Option<String>,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_session_id: Option<String>,
}

/// #249 Swipe：每条消息候选数上限。审计 C4 修复。
/// 超过时丢弃最旧的候选，保留最近 SWIPE_CANDIDATES_CAP 个。
/// 20 足够覆盖 ST 用户"尝试几次找好回复"场景，且控制 jsonl 增长。
pub const SWIPE_CANDIDATES_CAP: usize = 20;

/// `POST /v1/chat/swipe` 响应体。#252 D3：swipe 增量返回，不再回完整 ChatLog。
///
/// 客户端只需受影响消息的新 content 与确认 index；返回完整 ChatLog 是过量传输
/// （会话长时单条消息 JSON 体积膨胀）。`role` 与 `candidates_count` 便于 UI 显示
/// （如 "1/3" 候选计数、确认角色不变）。
///
/// 字段：
/// - `message_id`：受影响消息的 durable ID（与请求一致，便于客户端确认）。
/// - `index`：新激活候选的下标（0-based，与请求一致）。
/// - `content`：受影响消息切换后的 content。
/// - `role`：受影响消息的角色（不变，便于客户端不重新拉取历史）。
/// - `candidates_count`：候选总数（便于 UI 显示 "index+1/total"）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SwipeResponse {
    pub message_id: String,
    pub index: usize,
    pub content: String,
    pub role: crate::adapter::MessageRole,
    pub candidates_count: usize,
}

impl ChatService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    fn with_session<R>(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        operation: impl FnOnce() -> Result<R, AirpError>,
    ) -> Result<R, AirpError> {
        // LOCK-ORDER: character.read → session.lock（§2.1）。同步 std 锁，闭包内不 await。
        // 合同：docs/LOCK-ORDER-CONTRACT.md §2.1 / §3 R1 / §4 A1。
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
        let _character_track = lock_order::track_character_read();
        let session = session_lock(character_id.as_str(), session_id);
        let _session_guard = session.lock().unwrap_or_else(|p| p.into_inner());
        let _session_track = lock_order::track_session();
        // A never-seen named ID retains the legacy lazy-create behavior. Only
        // an explicitly deleted ID is rejected, using a tombstone so it cannot
        // be silently revived by load_or_create_for_session.
        if let Some(sid) = session_id {
            if data_dir::session_was_deleted(&self.data_root, character_id.as_str(), sid) {
                return Err(AirpError::NotFound(format!(
                    "session {sid} for character {character_id} not found"
                )));
            }
        }
        operation()
    }

    pub fn history(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            ChatLog::load_or_create_for_session(&self.data_root, character_id.as_str(), session_id)
        })
    }

    /// #37 cursor 分页窗口：返回 `before` ID 严格之前（更早）的消息，limit 上界。
    ///
    /// **cursor 语义**：`before` = 某条消息的 durable ID，返回该 ID **严格之前**的消息
    /// （更早的），按时间正序排列。`before` 必须命中当前 session 的某条 durable ID
    /// （含 legacy 派生 ID），否则 `BadRequest`——**cursor 不能跨 character/session 使用**。
    ///
    /// 不传 `before` → 返回最近 `limit` 条（时间正序）。
    /// 不传 `limit` → 默认 50；上界 200，超过 clamp。
    ///
    /// `has_more` = cursor 之前还有更早消息。`oldest_id` = 本窗口里最老消息的 ID，
    /// 供前端下次作 `before`。`total` = session 消息总数（含未加载）。
    pub fn history_window(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<HistoryWindow, AirpError> {
        let limit = limit.unwrap_or(50).clamp(1, 200);
        let log = self.history(character_id, session_id)?;

        // PR #270 audit B5 修复：history_window 必须按**激活路径**返回消息。
        // 旧实现返回物理 slice [start..end]，分支后会混入 sibling-branch 消息，
        // WebUI 不做客户端过滤 → 用户看到其他分支的消息串在当前分支里。
        // 新实现：用 `active_path_indices()` 拿到根→叶的物理 index 列表，
        // 在其上做 cursor 分页，再 project 所有 parallel 数组。
        //
        // Legacy 兼容：无 parent 的旧 log，`active_path_indices()` 走线性回退，
        // 返回 [0, 1, ..., n-1]，行为与旧实现一致。
        let active_indices = log.active_path_indices();
        let total = active_indices.len();

        // 找 cursor 切点：before ID 在 active path 里的位置；返回该位置严格之前。
        let cut = match before {
            Some(id) => {
                if !ulid::is_valid_id(id) {
                    return Err(AirpError::BadRequest(format!(
                        "cursor is not a valid durable message id: {id}"
                    )));
                }
                // 先验证 cursor 在本 session 中存在（cross-session 拒绝，保留原 contract）。
                // 再验证它在 active path 上（cross-branch 拒绝，B5 新 contract）。
                let in_session = log.message_ids.iter().any(|mid| ulid::matches(mid, id));
                if !in_session {
                    return Err(AirpError::BadRequest(format!(
                        "cursor {id} not in this session (cursor cannot cross character/session)"
                    )));
                }
                // cursor 必须在 active path 上，否则分页会跨分支。
                let pos = active_indices
                    .iter()
                    .position(|&phys_idx| {
                        log.message_ids
                            .get(phys_idx)
                            .map(|mid| ulid::matches(mid, id))
                            .unwrap_or(false)
                    })
                    .ok_or_else(|| {
                        AirpError::BadRequest(format!(
                            "cursor {id} not on active branch (cursor cannot cross branch)"
                        ))
                    })?;
                pos // 返回 [0, pos) 即更早的
            }
            None => total, // 无 cursor → 取最近 limit 条 = 尾部
        };

        // 窗口 = active_indices[start..end)，按时间正序。
        let end = cut.min(total);
        let start = end.saturating_sub(limit);
        let window_phys = &active_indices[start..end];

        // Project 所有 parallel 数组到窗口。
        let window_messages: Vec<ChatMessage> = window_phys
            .iter()
            .filter_map(|&i| log.messages.get(i).cloned())
            .collect();
        let window_ids: Vec<String> = window_phys
            .iter()
            .filter_map(|&i| log.message_ids.get(i).cloned())
            .collect();
        let window_ts: Vec<Option<String>> = window_phys
            .iter()
            .filter_map(|&i| log.message_timestamps.get(i).cloned())
            .collect();
        let window_cands: Vec<Vec<String>> = window_phys
            .iter()
            .filter_map(|&i| log.message_candidates.get(i).cloned())
            .collect();
        let window_swidx: Vec<usize> = window_phys
            .iter()
            .filter_map(|&i| log.message_swipe_index.get(i).copied())
            .collect();
        let window_parents: Vec<Option<String>> = window_phys
            .iter()
            .filter_map(|&i| log.message_parents.get(i).cloned())
            .collect();

        // has_more = 切点之前还有消息（start > 0）。
        let has_more = start > 0;
        // oldest_id = 本窗口最老消息的 ID（窗口首条）。
        let oldest_id = window_ids.first().cloned();
        // active_path = 当前激活路径的完整 ID 列表（与窗口无关，供前端 UI 标记分支点）。
        let active_path = log.active_path();

        Ok(HistoryWindow {
            messages: window_messages,
            message_ids: window_ids,
            message_timestamps: window_ts,
            message_candidates: window_cands,
            message_swipe_index: window_swidx,
            message_parents: window_parents,
            active_leaf: log.active_leaf.clone(),
            active_path,
            has_more,
            oldest_id,
            total,
            scope_session_id: log.scope_session_id().map(|s| s.to_string()),
        })
    }

    /// #37 rollback-by-ID：找到 `message_id` 在 `messages` 里的位置，调 `rollback_to(index)`。
    ///
    /// ID 不存在 → `BadRequest`。ID 寻址仍走 `with_session` 串行化，与并发 append 不产生半态。
    /// 同 `rollback`，返回 `(ChatLog, dropped_count)`。
    pub fn rollback_to_id(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message_id: &str,
    ) -> Result<(ChatLog, usize), AirpError> {
        if !ulid::is_valid_id(message_id) {
            return Err(AirpError::BadRequest(format!(
                "message_id is not a valid durable message id: {message_id}"
            )));
        }
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total = log.messages.len();
            if total == 0 {
                return Err(AirpError::BadRequest(format!(
                    "message_id {message_id} not in this empty session"
                )));
            }
            let idx = log
                .message_ids
                .iter()
                .position(|x| ulid::matches(x, message_id))
                .ok_or_else(|| {
                    AirpError::BadRequest(format!("message_id {message_id} not in this session"))
                })?;
            let dropped = total - idx - 1;
            log.rollback_to(&self.data_root, idx)?;
            Ok((log, dropped))
        })
    }

    pub fn recent(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, AirpError> {
        self.history(character_id, session_id)
            .map(|log| log.recent(limit))
    }

    /// Read existing history without lazy creation, migration, or metadata repair.
    pub fn recent_read_only(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, AirpError> {
        self.with_session(character_id, session_id, || {
            ChatLog::recent_existing_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
                limit,
            )
        })
    }

    pub fn append(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message: ChatMessage,
    ) -> Result<(ChatLog, usize), AirpError> {
        self.append_with_branch(character_id, session_id, message, None)
    }

    /// 追加消息，支持分支对话树。
    ///
    /// `branch_from` = `Some(id)` 时，新消息的 parent = 该 ID（从任意消息分叉）。
    /// `branch_from` = `None` 时，线性追加（parent = 当前 active_leaf）。
    pub fn append_with_branch(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message: ChatMessage,
        branch_from: Option<String>,
    ) -> Result<(ChatLog, usize), AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total_before = log.messages.len();
            match branch_from {
                Some(parent_id) => {
                    // 验证 branch_from ID 存在（case-insensitive，与 #37 contract 一致）。
                    if !log
                        .message_ids
                        .iter()
                        .any(|id| ulid::matches(id, &parent_id))
                    {
                        return Err(AirpError::BadRequest(format!(
                            "branch_from ID {parent_id} not found in session"
                        )));
                    }
                    log.append_with_parent(&self.data_root, message, Some(parent_id))?;
                }
                None => {
                    log.append(&self.data_root, message)?;
                }
            }
            Ok((log, total_before))
        })
    }

    /// 切换激活分支。
    pub fn switch_branch(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        target_leaf_id: &str,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            log.switch_branch(&self.data_root, target_leaf_id)?;
            Ok(log)
        })
    }

    pub fn rollback(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        index: usize,
    ) -> Result<(ChatLog, usize), AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total = log.messages.len();
            if total == 0 && index == 0 {
                return Ok((log, 0));
            }
            if index >= total {
                return Err(AirpError::BadRequest(format!(
                    "index {index} out of range (total {total})"
                )));
            }
            let dropped = total - index - 1;
            log.rollback_to(&self.data_root, index)?;
            Ok((log, dropped))
        })
    }

    pub fn rollback_preview(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        index: usize,
    ) -> Result<usize, AirpError> {
        self.with_session(character_id, session_id, || {
            let log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let total = log.messages.len();
            if total == 0 && index == 0 {
                return Ok(0);
            }
            if index >= total {
                return Err(AirpError::BadRequest(format!(
                    "index {index} out of range (total {total})"
                )));
            }
            Ok(total - index - 1)
        })
    }

    /// Capture the active assistant tail for deferred regen without modifying
    /// durable history. The caller supplies the already-reserved generation id.
    pub(crate) fn regen_snapshot(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        generation_id: String,
    ) -> Result<RegenSnapshot, AirpError> {
        self.with_session(character_id, session_id, || {
            let log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let target_index = log.active_path_indices().last().copied().ok_or_else(|| {
                AirpError::BadRequest("cannot regen: chat history is empty".to_string())
            })?;
            let target = log.messages.get(target_index).ok_or_else(|| {
                AirpError::Internal("active chat path references a missing message".to_string())
            })?;
            if target.role != crate::adapter::MessageRole::Assistant {
                return Err(AirpError::BadRequest(
                    "cannot regen: active message is not from assistant".to_string(),
                ));
            }
            let stored_candidates = log
                .message_candidates
                .get(target_index)
                .cloned()
                .unwrap_or_default();
            let candidates = if stored_candidates.is_empty() {
                vec![target.content.clone()]
            } else {
                stored_candidates.clone()
            };
            Ok(RegenSnapshot {
                generation_id,
                target_message_id: log.message_ids.get(target_index).cloned().ok_or_else(|| {
                    AirpError::Internal("active assistant message has no durable id".to_string())
                })?,
                revision: log.revision,
                content: target.content.clone(),
                stored_candidates,
                candidates,
                swipe_index: log
                    .message_swipe_index
                    .get(target_index)
                    .copied()
                    .unwrap_or(0),
            })
        })
    }

    /// Atomically replace one snapshotted assistant message with its old and new
    /// candidates. A stale snapshot never overwrites newer session state.
    pub(crate) fn commit_regen(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        snapshot: &RegenSnapshot,
        generated: &str,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            if log.revision != snapshot.revision {
                return Err(AirpError::Conflict(
                    "session changed during generation".to_string(),
                ));
            }
            let target_index = log
                .message_ids
                .iter()
                .position(|id| crate::ulid::matches(id, &snapshot.target_message_id))
                .ok_or_else(|| AirpError::Conflict("regen target no longer exists".to_string()))?;
            if log.active_path_indices().last().copied() != Some(target_index)
                || log.messages[target_index].role != crate::adapter::MessageRole::Assistant
                || log.messages[target_index].content != snapshot.content
                || log
                    .message_candidates
                    .get(target_index)
                    .cloned()
                    .unwrap_or_default()
                    != snapshot.stored_candidates
                || log
                    .message_swipe_index
                    .get(target_index)
                    .copied()
                    .unwrap_or(0)
                    != snapshot.swipe_index
            {
                return Err(AirpError::Conflict("regen snapshot is stale".to_string()));
            }

            let mut candidates = snapshot.candidates.clone();
            candidates.push(generated.to_string());
            candidates.retain(|candidate| !candidate.trim().is_empty());
            if candidates.is_empty() {
                return Ok(log);
            }
            if candidates.len() > SWIPE_CANDIDATES_CAP {
                let dropped = candidates.len() - SWIPE_CANDIDATES_CAP;
                candidates.drain(0..dropped);
            }
            let swipe_index = candidates.len() - 1;
            log.messages[target_index].content = candidates[swipe_index].clone();
            log.message_candidates[target_index] = candidates;
            log.message_swipe_index[target_index] = swipe_index;
            log.updated_at = chrono::Utc::now().to_rfc3339();
            log.save(&self.data_root)?;
            Ok(log)
        })
    }

    /// Append text to the last assistant message's content (used by /v1/chat/continue).
    ///
    /// If the last message is not an assistant message or the log is empty,
    /// returns `BadRequest`.
    pub fn append_to_last(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        text: &str,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let last = log.messages.last_mut().ok_or_else(|| {
                AirpError::BadRequest("cannot continue: chat history is empty".into())
            })?;
            if last.role != crate::adapter::MessageRole::Assistant {
                return Err(AirpError::BadRequest(
                    "cannot continue: last message is not from assistant".into(),
                ));
            }
            last.content.push_str(text);
            log.save(&self.data_root)?;
            Ok(log)
        })
    }

    /// #249 Swipe：追加一条带候选的 assistant 消息。
    ///
    /// `candidates` 含全部候选（含旧 + 新），`content` 设为最后一个候选（新生成的），
    /// `swipe_index` 指向最后一个候选。
    ///
    /// 审计 C4 修复：候选数上限 `SWIPE_CANDIDATES_CAP`（20）。超过时丢弃最旧的候选，
    /// 保留最近 20 个。swipe_index 指向最后一个（新候选）。cap 防止 jsonl 无界增长。
    pub fn append_with_candidates(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        mut candidates: Vec<String>,
    ) -> Result<ChatLog, AirpError> {
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            // #252 D2：防御性过滤——丢弃 trim 后为空的候选（whitespace-only）。
            // 上游 finalize.rs:39 已对新生成的 stripped 做 trim 检查；此处独立过滤
            // 保证历史数据或异常上游传入的空白候选不会污染持久化状态。
            let original_count = candidates.len();
            candidates.retain(|c| !c.trim().is_empty());
            if candidates.is_empty() {
                return Err(AirpError::BadRequest(
                    "append_with_candidates: candidates cannot be empty or all whitespace".into(),
                ));
            }
            if candidates.len() != original_count {
                tracing::warn!(
                    dropped = original_count - candidates.len(),
                    "append_with_candidates: dropped whitespace-only candidates"
                );
            }
            // 审计 C4：候选 cap。超过时丢弃最旧的，保留最近 SWIPE_CANDIDATES_CAP 个。
            if candidates.len() > SWIPE_CANDIDATES_CAP {
                let drop_count = candidates.len() - SWIPE_CANDIDATES_CAP;
                candidates.drain(0..drop_count);
            }
            let swipe_index = candidates.len() - 1;
            let content = candidates[swipe_index].clone();
            // 分支对话树：parent = 当前 active_leaf（线性链 = 前一条消息 ID）。
            // PR #270 audit B2 修复：regen 流程中 `regen()` 已 delete_last_n(1)
            // 把 active_leaf 回退到 user 消息，此处 parent = 该 user 消息 ID。
            // 旧实现漏写 `message_parents` / `active_leaf`，导致 regen 后的
            // assistant 消息成为 orphan root，破坏分支树结构。
            let parent = log
                .active_leaf
                .clone()
                .or_else(|| log.message_ids.last().cloned());
            let new_id = crate::ulid::new_id();
            log.messages.push(ChatMessage {
                role: crate::adapter::MessageRole::Assistant,
                content,
            });
            log.message_ids.push(new_id.clone());
            log.message_timestamps
                .push(Some(chrono::Utc::now().to_rfc3339()));
            log.message_candidates.push(candidates);
            log.message_swipe_index.push(swipe_index);
            log.message_parents.push(parent);
            log.active_leaf = Some(new_id);
            log.updated_at = chrono::Utc::now().to_rfc3339();
            log.save(&self.data_root)?;
            Ok(log)
        })
    }

    /// #249 Swipe：切换指定消息的激活候选。
    ///
    /// `message_id` 是 durable ID，`new_index` 是候选下标（0-based）。
    /// 切换后 `messages[i].content` 更新为 `candidates[new_index]`。
    /// ID 不变（解耦优先：role 可变，ID 不应变）。
    ///
    /// #252 D3：返回 `SwipeResponse` 增量响应，不再回完整 `ChatLog`（性能优化）。
    pub fn switch_swipe(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message_id: &str,
        new_index: usize,
    ) -> Result<SwipeResponse, AirpError> {
        if !crate::ulid::is_valid_id(message_id) {
            return Err(AirpError::BadRequest(format!(
                "message_id is not a valid durable message id: {message_id}"
            )));
        }
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let idx = log
                .message_ids
                .iter()
                .position(|x| crate::ulid::matches(x, message_id))
                .ok_or_else(|| {
                    AirpError::BadRequest(format!("message_id {message_id} not in this session"))
                })?;
            let candidates_count = log
                .message_candidates
                .get(idx)
                .ok_or_else(|| {
                    AirpError::BadRequest(format!("message {message_id} has no candidates"))
                })?
                .len();
            let cands = log.message_candidates.get(idx).expect("checked above");
            if cands.is_empty() {
                return Err(AirpError::BadRequest(format!(
                    "message {message_id} has no candidates to switch"
                )));
            }
            if new_index >= cands.len() {
                return Err(AirpError::BadRequest(format!(
                    "swipe index {new_index} out of range (candidates: {})",
                    cands.len()
                )));
            }
            // 更新 content 和 swipe_index。
            log.messages[idx].content = cands[new_index].clone();
            log.message_swipe_index[idx] = new_index;
            // 审计 D1 修复：与其他 mutation 保持一致，更新 updated_at。
            log.updated_at = chrono::Utc::now().to_rfc3339();
            log.save(&self.data_root)?;
            // #252 D3：增量返回，不再回完整 ChatLog。
            Ok(SwipeResponse {
                message_id: message_id.to_string(),
                index: new_index,
                content: log.messages[idx].content.clone(),
                role: log.messages[idx].role,
                candidates_count,
            })
        })
    }

    /// Edit a single user message's content by its durable ID.
    ///
    /// Only `role=user` messages can be edited (assistant editing = regen/swipe semantics).
    /// ID, timestamp, and role are preserved; only `content` is replaced.
    /// Persistence: full JSONL rewrite via `save()` under `with_session` serialization.
    pub fn edit_message(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message_id: &str,
        new_content: &str,
    ) -> Result<ChatLog, AirpError> {
        if !crate::ulid::is_valid_id(message_id) {
            return Err(AirpError::BadRequest(format!(
                "message_id is not a valid durable message id: {message_id}"
            )));
        }
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let idx = log
                .message_ids
                .iter()
                .position(|x| crate::ulid::matches(x, message_id))
                .ok_or_else(|| {
                    AirpError::BadRequest(format!("message_id {message_id} not in this session"))
                })?;
            // Only user messages can be edited.
            if log.messages[idx].role != crate::adapter::MessageRole::User {
                return Err(AirpError::BadRequest(
                    "only user messages can be edited; use regen/swipe for assistant messages"
                        .to_string(),
                ));
            }
            log.messages[idx].content = new_content.to_string();
            log.updated_at = chrono::Utc::now().to_rfc3339();
            log.save(&self.data_root)?;
            Ok(log)
        })
    }

    /// Delete a single message by its durable ID, preserving the order of remaining messages.
    ///
    /// (ST calibration: SillyTavern deletes a single message, not rollback-to.)
    pub fn delete_message(
        &self,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        message_id: &str,
    ) -> Result<ChatLog, AirpError> {
        if !crate::ulid::is_valid_id(message_id) {
            return Err(AirpError::BadRequest(format!(
                "message_id is not a valid durable message id: {message_id}"
            )));
        }
        self.with_session(character_id, session_id, || {
            let mut log = ChatLog::load_or_create_for_session(
                &self.data_root,
                character_id.as_str(),
                session_id,
            )?;
            let idx = log
                .message_ids
                .iter()
                .position(|x| crate::ulid::matches(x, message_id))
                .ok_or_else(|| {
                    AirpError::BadRequest(format!("message_id {message_id} not in this session"))
                })?;
            // PR #270 audit B3 修复：捕获被删消息的 parent，同步移除 message_parents，
            // 并在被删消息是当前 active_leaf 时回退 active_leaf 到其 parent。
            // 旧实现漏写 message_parents.remove + active_leaf 重置，导致：
            // (1) message_parents 长度与 messages 不一致（不变式破坏）；
            // (2) 删除 leaf 后 active_leaf 指向已不存在的 ID，后续 recent() /
            //     active_path_indices() 找不到 leaf → 返回空，破坏 LLM 上下文。
            let deleted_parent = log
                .message_parents
                .get(idx)
                .and_then(|p| p.clone())
                .or_else(|| {
                    // Legacy 无 parent：线性链中删除中间消息，parent 回退为前一条消息 ID。
                    if idx > 0 {
                        log.message_ids.get(idx - 1).cloned()
                    } else {
                        None
                    }
                });
            let was_active_leaf = log
                .active_leaf
                .as_ref()
                .map(|leaf| crate::ulid::matches(leaf, message_id))
                .unwrap_or(false);
            log.messages.remove(idx);
            log.message_ids.remove(idx);
            // Legacy chat logs may have fewer timestamps than messages;
            // defensively check bounds to avoid out-of-bounds panic.
            if idx < log.message_timestamps.len() {
                log.message_timestamps.remove(idx);
            }
            // #249：同步移除 candidates / swipe_index。
            if idx < log.message_candidates.len() {
                log.message_candidates.remove(idx);
            }
            if idx < log.message_swipe_index.len() {
                log.message_swipe_index.remove(idx);
            }
            // PR #270 audit B3：同步移除 message_parents。
            if idx < log.message_parents.len() {
                log.message_parents.remove(idx);
            }
            // PR #270 audit B3：被删消息是 active_leaf 时回退。
            if was_active_leaf {
                log.active_leaf = if log.messages.is_empty() {
                    None
                } else {
                    deleted_parent.or_else(|| log.message_ids.last().cloned())
                };
            }
            log.save(&self.data_root)?;
            Ok(log)
        })
    }

    pub fn list_sessions(&self, character_id: &CharacterId) -> Result<Vec<SessionId>, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().unwrap_or_else(|p| p.into_inner());
        data_dir::list_sessions(&self.data_root, character_id.as_str())
    }

    pub fn create_session(&self, character_id: &CharacterId) -> Result<SessionId, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().unwrap_or_else(|p| p.into_inner());
        data_dir::create_session(&self.data_root, character_id.as_str())
    }

    /// #342 E-P2-1：删除角色目录。
    ///
    /// 默认在 `fs::remove_dir_all` 前创建 `BackupSource::PreDelete` +
    /// `BackupScope::Character { id }` scoped backup，让删除可恢复。
    /// `force = true` 时跳过 pre-delete backup（advanced / testing）。
    ///
    /// pre-delete backup 失败 → `Err`，**不**删除数据（fail-closed）。
    /// backup 内**不**含 secrets.json / settings.json（denylist 排除）。
    pub fn delete_character(
        &self,
        character_id: &CharacterId,
        force: bool,
    ) -> Result<Option<String>, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.write().unwrap_or_else(|p| p.into_inner());
        let _character_track = lock_order::track_character_write();

        // pre-delete backup（默认开启）
        let backup_id = if !force {
            let opts = crate::backup::CreateBackupOptions {
                data_root: self.data_root.clone(),
                source: crate::backup::BackupSource::PreDelete,
                scope: crate::backup::BackupScope::Character {
                    character_id: character_id.as_str().to_string(),
                },
            };
            let created = crate::backup::create_backup(&opts).map_err(|e| {
                AirpError::Internal(format!(
                    "pre-delete backup 失败，已拒绝删除 character {}（fail-closed）: {e}",
                    character_id.as_str()
                ))
            })?;
            Some(created.backup_id)
        } else {
            None
        };

        let result = data_dir::delete_character(&self.data_root, character_id);
        // #440: 必须在 write guard 释放之后再清理 lock-map 条目。否则新 caller
        // 调 `character_lock` 会拿到新 Arc（条目已移除），不与本次 write guard
        // 互斥，绕过 R1 串行化——Windows 上观察到 `DirectoryNotEmpty`，Linux
        // 上理论 TOCTOU（advance_plot 用新 Arc 复活已删 dir 的部分文件）。
        // 显式 drop 确保 cleanup 与临界区不重叠。
        drop(_character_track);
        drop(_guard);
        if result.is_ok() {
            // #422: character 目录 durable 删除后清理 lock-map stale 条目。
            // 正在等待旧 Arc 的 waiter 拿到锁后操作已删除资源会 fail closed
            //（NotFound）；新 caller 调 `character_lock`/`state_lock` 会创建新
            // Arc 走正常 create 流程。与 `delete_session` 清理 session lock 同模式。
            // 已知 gap：SESSION_LOCKS 中该 character 下所有 `{cid}/*` 条目也会
            // stale，但批量前缀清理需遍历整表且 `delete_session` 已有 per-session
            // 清理路径；character 级批量 session lock 清理留作后续。
            remove_deleted_character_lock(character_id.as_str());
            remove_deleted_state_lock(character_id.as_str());
        }
        result?;
        Ok(backup_id)
    }

    /// #35：删除一个命名会话目录。走 character read lock + session lock，与 append/
    /// rollback/regen 同边界串行化，避免并发写期间删到半态。
    ///
    /// #342 E-P2-1：默认创建 `BackupSource::PreDelete` +
    /// `BackupScope::Session { .. }` scoped backup，让删除可恢复。
    /// `force = true` 时跳过 pre-delete backup。
    ///
    /// 会话不存在 → `NotFound`。destructive：调用方负责确认。
    /// pre-delete backup 失败 → `Err`，**不**删除数据（fail-closed）。
    pub fn delete_session(
        &self,
        character_id: &CharacterId,
        session_id: &SessionId,
        force: bool,
    ) -> Result<Option<String>, AirpError> {
        let character = character_lock(character_id.as_str());
        let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
        let _character_track = lock_order::track_character_read();
        let session = session_lock(character_id.as_str(), Some(session_id));
        let _session_guard = session.lock().unwrap_or_else(|p| p.into_inner());
        let _session_track = lock_order::track_session();

        // pre-delete backup（默认开启）
        let backup_id = if !force {
            let opts = crate::backup::CreateBackupOptions {
                data_root: self.data_root.clone(),
                source: crate::backup::BackupSource::PreDelete,
                scope: crate::backup::BackupScope::Session {
                    character_id: character_id.as_str().to_string(),
                    session_id: session_id.to_string(),
                },
            };
            let created = crate::backup::create_backup(&opts).map_err(|e| {
                AirpError::Internal(format!(
                    "pre-delete backup 失败，已拒绝删除 session {}（fail-closed）: {e}",
                    session_id
                ))
            })?;
            Some(created.backup_id)
        } else {
            None
        };

        // A previous attempt may have written the fail-closed tombstone but
        // failed to remove the directory. Deletion must bypass `with_session`'s
        // tombstone rejection so a retry can finish that cleanup.
        let result = data_dir::delete_session(&self.data_root, character_id.as_str(), session_id);
        // #440: 同 delete_character，在 guard 释放后再清理 lock-map 条目，
        // 避免新 caller 用新 Arc 绕过 R1 串行化。
        drop(_session_track);
        drop(_session_guard);
        drop(_character_track);
        drop(_character_guard);
        if result.is_ok() {
            remove_deleted_session_lock(character_id.as_str(), session_id);
        }
        result?;
        Ok(backup_id)
    }
}
