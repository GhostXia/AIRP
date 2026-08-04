//! Session recovery HTTP handler — user-directed escape hatch for sessions
//! locked by a pending TurnCommit marker (BUG-2 mitigation slice).
//!
//! The endpoint never replays or repairs the interrupted turn; it only
//! archives (quarantines) the durable marker so the session stops failing
//! closed.  Payload-aware replay remains a separate future slice, which is
//! why the marker bytes are preserved instead of deleted.
//!
//! 端点：
//! - `POST /v1/chat/session-recover` — 归档 pending marker 并解除会话锁死

use crate::daemon::types::{SessionRecoverRequest, SessionRecoverResponse};
use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::session_coordinator::SessionPhase;
use axum::Json;
use std::sync::Arc;

/// POST /v1/chat/session-recover — quarantine the pending TurnCommit marker
/// of a fail-closed session so new mutations are admitted again.
///
/// Semantics:
/// - Session has an active command lease → 409 `session_busy`.
/// - Session has no pending marker → 404 (nothing to recover; terminal
///   markers are already auto-cleared by the Coordinator observation path).
/// - Pending marker present → the marker is moved into
///   `<data_root>/quarantine/turn-commit/...` (never deleted) and the
///   quarantine result is reported.
pub(in crate::daemon) async fn recover_chat_session(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<SessionRecoverRequest>,
) -> Result<Json<SessionRecoverResponse>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    // Status observation doubles as the admission guard: while a marker
    // exists no command lease can be granted, so `Recovering` here means
    // "marker present and no active owner".  `Idle` means there is nothing
    // left to recover.
    let status = state.session_coordinators.status(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
    );
    match status.phase {
        SessionPhase::Recovering => {}
        SessionPhase::Idle => {
            tracing::info!(
                event = "session_recover_rejected",
                reason = "no_pending_marker",
                character_id = %req.character_id,
                session_id = ?req.session_id,
                user_id = ?req.user_id,
                "session recover requested but no pending marker exists"
            );
            return Err(AirpError::NotFound(
                "no pending turn commit marker for this session".to_string(),
            ));
        }
        SessionPhase::Generating | SessionPhase::Committing => {
            return Err(AirpError::Conflict("session_busy".to_string()));
        }
    }
    tracing::info!(
        event = "session_recover_requested",
        character_id = %req.character_id,
        session_id = ?req.session_id,
        user_id = ?req.user_id,
        generation_id = ?status.generation_id,
        "quarantining pending turn commit marker"
    );
    let quarantined = crate::turn_commit::quarantine_pending_marker(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
    )?;
    let phase = serde_json::to_value(quarantined.phase)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    tracing::info!(
        event = "session_recovered",
        character_id = %req.character_id,
        session_id = ?req.session_id,
        user_id = ?req.user_id,
        quarantine_path = %quarantined.quarantine_path.display(),
        "session unblocked; marker archived for future replay"
    );
    Ok(Json(SessionRecoverResponse {
        status: "recovered",
        character_id: req.character_id.as_str().to_string(),
        session_id: req.session_id.as_ref().map(ToString::to_string),
        generation_id: quarantined.generation_id,
        phase,
        quarantined_marker: quarantined.quarantine_path.display().to_string(),
    }))
}
