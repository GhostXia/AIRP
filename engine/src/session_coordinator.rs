//! Per-session command admission and observable lifecycle state.
//!
//! The coordinator is a daemon-owned façade: transport handlers submit a
//! `SessionCommand` before touching durable chat state, and generation
//! pipelines carry the returned lease through their commit point. Registry
//! entries are weak and therefore disappear when a session becomes idle.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionCommand {
    Completion,
    Regen,
    Continue,
    Rollback,
    DeleteMessage,
    EditMessage,
    Swipe,
    SwitchBranch,
    /// A durable mutation initiated by an Agent tool.  This is intentionally
    /// separate from chat HTTP mutations so session-state observers can tell
    /// why a session is busy without exposing individual tool parameters.
    AgentToolMutation,
}

impl SessionCommand {
    fn initial_phase(self) -> SessionPhase {
        match self {
            Self::Completion | Self::Regen | Self::Continue => SessionPhase::Generating,
            Self::Rollback
            | Self::DeleteMessage
            | Self::EditMessage
            | Self::Swipe
            | Self::SwitchBranch
            | Self::AgentToolMutation => SessionPhase::Committing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionPhase {
    Idle,
    Generating,
    Committing,
    Recovering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SessionCoordinatorStatus {
    pub(crate) phase: SessionPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<SessionCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) generation_id: Option<String>,
}

impl SessionCoordinatorStatus {
    fn idle() -> Self {
        Self {
            phase: SessionPhase::Idle,
            command: None,
            generation_id: None,
        }
    }
}

#[derive(Debug)]
struct CoordinatorState {
    status: SessionCoordinatorStatus,
    cancellation: Option<CancellationToken>,
}

type Registry = HashMap<String, Weak<Mutex<CoordinatorState>>>;

/// Daemon-local registry of active session coordinators.
///
/// A registry lookup never keeps an idle session alive. While a lease exists,
/// its strong `Arc` prevents a second coordinator instance for the same key.
#[derive(Debug, Clone, Default)]
pub struct SessionCoordinatorRegistry {
    entries: Arc<Mutex<Registry>>,
}

impl SessionCoordinatorRegistry {
    pub(crate) fn try_submit(
        &self,
        data_root: &Path,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        command: SessionCommand,
    ) -> Result<SessionCommandLease, AirpError> {
        let key = session_key(data_root, character_id, session_id);
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|_, entry| entry.strong_count() > 0);
        let state = match entries.get(&key).and_then(Weak::upgrade) {
            Some(state) => state,
            None => {
                let state = Arc::new(Mutex::new(CoordinatorState {
                    status: SessionCoordinatorStatus::idle(),
                    cancellation: None,
                }));
                entries.insert(key.clone(), Arc::downgrade(&state));
                state
            }
        };
        #[cfg(test)]
        assert_registry_guard_is_held(&self.entries);
        // Acquire the per-session guard while the registry entry is still
        // protected.  Lease release uses this same entries -> state order;
        // keeping both guards through the idle check prevents a release from
        // removing the weak entry between upgrade and state admission.
        let mut coordinator = state.lock().unwrap_or_else(|p| p.into_inner());
        if coordinator.status.phase != SessionPhase::Idle {
            return Err(AirpError::Conflict("session_busy".to_string()));
        }
        drop(entries);
        // Do not hold the global registry mutex across filesystem access: a
        // slow data root for one session must not serialize unrelated sessions.
        // Keep this per-session state guard while inspecting/removing a
        // terminal marker so an active owner cannot race its final `complete`.
        if crate::turn_commit::recover_completed_turn(data_root, character_id, session_id).is_some()
        {
            return Err(AirpError::Conflict("session_recovery_required".to_string()));
        }
        let generation_id = crate::ulid::new_id();
        coordinator.status = SessionCoordinatorStatus {
            phase: command.initial_phase(),
            command: Some(command),
            generation_id: Some(generation_id.clone()),
        };
        let cancellation = matches!(
            command,
            SessionCommand::Completion | SessionCommand::Regen | SessionCommand::Continue
        )
        .then(CancellationToken::new);
        coordinator.cancellation = cancellation.clone();
        drop(coordinator);
        Ok(SessionCommandLease {
            registry: self.clone(),
            key,
            state,
            generation_id,
            cancellation,
            released: false,
        })
    }

    /// Requests cooperative cancellation for the exact active generation.
    ///
    /// The state mutex linearizes this request against `begin_commit`: either
    /// cancellation wins, or the caller receives `generation_committing`.
    pub(crate) fn cancel_generation(
        &self,
        data_root: &Path,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        generation_id: &str,
    ) -> Result<SessionCoordinatorStatus, AirpError> {
        let key = session_key(data_root, character_id, session_id);
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|_, entry| entry.strong_count() > 0);
        let state = entries
            .get(&key)
            .and_then(Weak::upgrade)
            .ok_or_else(|| AirpError::Conflict("stale_generation".to_string()))?;
        let coordinator = state.lock().unwrap_or_else(|p| p.into_inner());
        if coordinator.status.generation_id.as_deref() != Some(generation_id) {
            return Err(AirpError::Conflict("stale_generation".to_string()));
        }
        if coordinator.status.phase == SessionPhase::Committing {
            return Err(AirpError::Conflict("generation_committing".to_string()));
        }
        let cancellation = coordinator.cancellation.as_ref().ok_or_else(|| {
            AirpError::Internal("generating coordinator has no cancellation token".to_string())
        })?;
        cancellation.cancel();
        Ok(coordinator.status.clone())
    }

    pub(crate) fn status(
        &self,
        data_root: &Path,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
    ) -> SessionCoordinatorStatus {
        let key = session_key(data_root, character_id, session_id);
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|_, entry| entry.strong_count() > 0);
        let active = entries.get(&key).and_then(Weak::upgrade).map(|state| {
            state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .status
                .clone()
        });
        drop(entries);
        active.unwrap_or_else(|| {
            crate::turn_commit::recover_completed_turn(data_root, character_id, session_id)
                .map(|marker| SessionCoordinatorStatus {
                    phase: SessionPhase::Recovering,
                    command: None,
                    generation_id: (!marker.generation_id.is_empty())
                        .then_some(marker.generation_id),
                })
                .unwrap_or_else(SessionCoordinatorStatus::idle)
        })
    }

    #[cfg(test)]
    fn active_entry_count(&self) -> usize {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| entry.strong_count() > 0);
        entries.len()
    }
}

#[cfg(test)]
fn assert_registry_guard_is_held(entries: &Mutex<Registry>) {
    assert!(
        matches!(entries.try_lock(), Err(std::sync::TryLockError::WouldBlock)),
        "try_submit must hold the registry lock while acquiring session state"
    );
}

pub(crate) struct SessionCommandLease {
    registry: SessionCoordinatorRegistry,
    key: String,
    state: Arc<Mutex<CoordinatorState>>,
    generation_id: String,
    cancellation: Option<CancellationToken>,
    released: bool,
}

impl SessionCommandLease {
    pub(crate) fn generation_id(&self) -> &str {
        &self.generation_id
    }

    pub(crate) fn matches_generation(&self, generation_id: &str) -> bool {
        !self.released && self.generation_id == generation_id
    }

    /// Returns the cooperative token for generation commands.
    pub(crate) fn cancellation(&self) -> Option<CancellationToken> {
        self.cancellation.clone()
    }

    pub(crate) fn begin_commit(&mut self) -> Result<(), AirpError> {
        let mut coordinator = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if self.released
            || coordinator.status.generation_id.as_deref() != Some(self.generation_id.as_str())
        {
            return Err(AirpError::Conflict("generation_lease_lost".to_string()));
        }
        if coordinator
            .cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(AirpError::Conflict("generation_cancelled".to_string()));
        }
        coordinator.status.phase = SessionPhase::Committing;
        Ok(())
    }

    pub(crate) fn release(&mut self) {
        if self.released {
            return;
        }
        let mut entries = self
            .registry
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut coordinator = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if coordinator.status.generation_id.as_deref() == Some(self.generation_id.as_str()) {
            coordinator.status = SessionCoordinatorStatus::idle();
            coordinator.cancellation = None;
        }
        if entries
            .get(&self.key)
            .is_some_and(|entry| entry.ptr_eq(&Arc::downgrade(&self.state)))
        {
            entries.remove(&self.key);
        }
        self.released = true;
    }
}

impl Drop for SessionCommandLease {
    fn drop(&mut self) {
        self.release();
    }
}

fn session_key(
    data_root: &Path,
    character_id: &CharacterId,
    session_id: Option<&SessionId>,
) -> String {
    let session = session_id
        .map(ToString::to_string)
        .unwrap_or_else(|| "legacy".to_string());
    format!("{}::{character_id}/{session}", data_root.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_reports_generation_and_commit_states_then_reclaims_idle_entry() {
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("char-a").unwrap();
        let root = Path::new("data");
        let mut lease = registry
            .try_submit(root, &character, None, SessionCommand::Regen)
            .unwrap();
        assert_eq!(
            registry.status(root, &character, None).phase,
            SessionPhase::Generating
        );
        assert!(matches!(
            registry.try_submit(root, &character, None, SessionCommand::Swipe),
            Err(AirpError::Conflict(message)) if message == "session_busy"
        ));
        lease.begin_commit().unwrap();
        assert_eq!(
            registry.status(root, &character, None).phase,
            SessionPhase::Committing
        );
        drop(lease);
        assert_eq!(
            registry.status(root, &character, None).phase,
            SessionPhase::Idle
        );
        assert_eq!(registry.active_entry_count(), 0);
    }

    #[test]
    fn different_sessions_do_not_share_an_owner() {
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("char-a").unwrap();
        let first = SessionId::new();
        let second = SessionId::new();
        let root = Path::new("data");
        let _first = registry
            .try_submit(root, &character, Some(&first), SessionCommand::Completion)
            .unwrap();
        let _second = registry
            .try_submit(root, &character, Some(&second), SessionCommand::Completion)
            .unwrap();
    }

    #[test]
    fn cancellation_is_generation_scoped_and_rejected_after_commit_starts() {
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("char-a").unwrap();
        let root = Path::new("data");
        let mut lease = registry
            .try_submit(root, &character, None, SessionCommand::Regen)
            .unwrap();
        let cancellation = lease.cancellation().unwrap();

        assert!(matches!(
            registry.cancel_generation(root, &character, None, "older-generation"),
            Err(AirpError::Conflict(message)) if message == "stale_generation"
        ));
        assert!(!cancellation.is_cancelled());

        registry
            .cancel_generation(root, &character, None, lease.generation_id())
            .unwrap();
        assert!(cancellation.is_cancelled());

        assert!(matches!(
            lease.begin_commit(),
            Err(AirpError::Conflict(message)) if message == "generation_cancelled"
        ));
        drop(lease);

        let committing = registry
            .try_submit(root, &character, None, SessionCommand::Swipe)
            .unwrap();
        assert!(matches!(
            registry.cancel_generation(root, &character, None, committing.generation_id()),
            Err(AirpError::Conflict(message)) if message == "generation_committing"
        ));
    }

    #[test]
    fn durable_marker_reports_recovering_and_blocks_new_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("recovering-char").unwrap();
        let mut commit = crate::turn_commit::TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "interrupted-generation".to_string(),
            true,
            true,
            false,
        )
        .unwrap();

        let status = registry.status(tmp.path(), &character, None);
        assert_eq!(status.phase, SessionPhase::Recovering);
        assert_eq!(
            status.generation_id.as_deref(),
            Some("interrupted-generation")
        );
        assert!(matches!(
            registry.try_submit(
                tmp.path(),
                &character,
                None,
                SessionCommand::Completion
            ),
            Err(AirpError::Conflict(message)) if message == "session_recovery_required"
        ));

        commit.mark_message_committed().unwrap();
        commit.mark_state_committed().unwrap();
        commit.complete().unwrap();
        assert_eq!(
            registry.status(tmp.path(), &character, None).phase,
            SessionPhase::Idle
        );
    }

    #[test]
    fn terminal_marker_is_cleared_before_new_command_admission() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("terminal-recovery-char").unwrap();
        let mut commit = crate::turn_commit::TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "terminal-generation".to_string(),
            true,
            true,
            false,
        )
        .unwrap();
        commit.mark_message_committed().unwrap();
        commit.mark_state_committed().unwrap();
        // Simulate a process exit after all resource stages but before marker
        // cleanup.  The next observer/admission call may safely remove it.
        std::mem::forget(commit);

        assert_eq!(
            registry.status(tmp.path(), &character, None).phase,
            SessionPhase::Idle
        );
        let lease = registry
            .try_submit(tmp.path(), &character, None, SessionCommand::Completion)
            .expect("terminal marker recovery must unblock a new command");
        assert_eq!(
            registry.status(tmp.path(), &character, None).phase,
            SessionPhase::Generating
        );
        drop(lease);
        assert!(crate::turn_commit::pending_turn(tmp.path(), &character, None).is_none());
    }

    #[test]
    fn active_owner_keeps_terminal_marker_until_complete_finishes() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("active-terminal-recovery-char").unwrap();
        let mut lease = registry
            .try_submit(tmp.path(), &character, None, SessionCommand::Completion)
            .unwrap();
        lease.begin_commit().unwrap();
        let mut commit = crate::turn_commit::TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            lease.generation_id().to_string(),
            true,
            true,
            false,
        )
        .unwrap();
        commit.mark_message_committed().unwrap();
        commit.mark_state_committed().unwrap();

        assert!(matches!(
            registry.try_submit(tmp.path(), &character, None, SessionCommand::Swipe),
            Err(AirpError::Conflict(message)) if message == "session_busy"
        ));
        assert!(crate::turn_commit::pending_turn(tmp.path(), &character, None).is_some());

        // The active owner still completes the marker after the rejected
        // competing command; recovery cleanup must not have stolen it.
        commit.complete().unwrap();
        drop(lease);
        assert!(crate::turn_commit::pending_turn(tmp.path(), &character, None).is_none());
    }

    #[test]
    fn release_and_new_admission_keep_a_single_registered_owner() {
        // Start release and admission together repeatedly.  Depending on
        // which side acquires the registry first, either the admission is
        // rejected as busy (and the third submit becomes the owner) or the
        // admission becomes the owner (and the third submit is rejected).
        // The lock order must never allow both to succeed on distinct state
        // objects after the old lease removes its weak entry.
        // The deterministic historical-gap check is the test-only
        // `assert_registry_guard_is_held` above; this loop is runtime race
        // coverage rather than a proof that the old implementation reproduces.
        for _ in 0..64 {
            let tmp = tempfile::tempdir().unwrap();
            let registry = SessionCoordinatorRegistry::default();
            let character = CharacterId::new("release-admission-race").unwrap();
            let mut old_lease = registry
                .try_submit(tmp.path(), &character, None, SessionCommand::Completion)
                .unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(3));

            let release_barrier = barrier.clone();
            let release_thread = std::thread::spawn(move || {
                release_barrier.wait();
                std::thread::yield_now();
                old_lease.release();
            });

            let admission_registry = registry.clone();
            let admission_root = tmp.path().to_path_buf();
            let admission_character = character.clone();
            let admission_barrier = barrier.clone();
            let admission_thread = std::thread::spawn(move || {
                admission_barrier.wait();
                std::thread::yield_now();
                admission_registry.try_submit(
                    &admission_root,
                    &admission_character,
                    None,
                    SessionCommand::Swipe,
                )
            });

            barrier.wait();
            let admission = admission_thread.join().unwrap();
            release_thread.join().unwrap();

            match admission {
                Ok(admission_lease) => {
                    assert!(matches!(
                        registry.try_submit(
                            tmp.path(),
                            &character,
                            None,
                            SessionCommand::AgentToolMutation,
                        ),
                        Err(AirpError::Conflict(message)) if message == "session_busy"
                    ));
                    drop(admission_lease);
                }
                Err(AirpError::Conflict(message)) if message == "session_busy" => {
                    let third_lease = registry
                        .try_submit(tmp.path(), &character, None, SessionCommand::Swipe)
                        .expect("the third submit owns the session after the old lease releases");
                    drop(third_lease);
                }
                Err(error) => panic!("unexpected admission result: {error:?}"),
            }

            assert_eq!(registry.active_entry_count(), 0);
        }
    }

    #[test]
    fn active_commit_marker_remains_session_busy_until_owner_is_lost() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SessionCoordinatorRegistry::default();
        let character = CharacterId::new("active-marker-char").unwrap();
        let mut lease = registry
            .try_submit(tmp.path(), &character, None, SessionCommand::Completion)
            .unwrap();
        lease.begin_commit().unwrap();
        let mut commit = crate::turn_commit::TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            lease.generation_id().to_string(),
            true,
            true,
            false,
        )
        .unwrap();

        assert_eq!(
            registry.status(tmp.path(), &character, None).phase,
            SessionPhase::Committing
        );
        assert!(matches!(
            registry.try_submit(tmp.path(), &character, None, SessionCommand::Swipe),
            Err(AirpError::Conflict(message)) if message == "session_busy"
        ));

        drop(lease);
        assert_eq!(
            registry.status(tmp.path(), &character, None).phase,
            SessionPhase::Recovering
        );
        commit.mark_message_committed().unwrap();
        commit.mark_state_committed().unwrap();
        commit.complete().unwrap();
    }
}
