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
}

impl SessionCommand {
    fn initial_phase(self) -> SessionPhase {
        match self {
            Self::Completion | Self::Regen | Self::Continue => SessionPhase::Generating,
            Self::Rollback
            | Self::DeleteMessage
            | Self::EditMessage
            | Self::Swipe
            | Self::SwitchBranch => SessionPhase::Committing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionPhase {
    Idle,
    Generating,
    Committing,
    #[allow(dead_code)] // Reserved for Phase O3 journal recovery admission.
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
        let mut entries = self
            .entries
            .lock()
            .expect("session coordinator registry poisoned");
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
        let mut coordinator = state.lock().expect("session coordinator state poisoned");
        if coordinator.status.phase != SessionPhase::Idle {
            return Err(AirpError::Conflict("session_busy".to_string()));
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
        let mut entries = self
            .entries
            .lock()
            .expect("session coordinator registry poisoned");
        entries.retain(|_, entry| entry.strong_count() > 0);
        let state = entries
            .get(&key)
            .and_then(Weak::upgrade)
            .ok_or_else(|| AirpError::Conflict("stale_generation".to_string()))?;
        let coordinator = state.lock().expect("session coordinator state poisoned");
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
        let mut entries = self
            .entries
            .lock()
            .expect("session coordinator registry poisoned");
        entries.retain(|_, entry| entry.strong_count() > 0);
        entries
            .get(&key)
            .and_then(Weak::upgrade)
            .map(|state| {
                state
                    .lock()
                    .expect("session coordinator state poisoned")
                    .status
                    .clone()
            })
            .unwrap_or_else(SessionCoordinatorStatus::idle)
    }

    #[cfg(test)]
    fn active_entry_count(&self) -> usize {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| entry.strong_count() > 0);
        entries.len()
    }
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
        let mut coordinator = self
            .state
            .lock()
            .expect("session coordinator state poisoned");
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
            .expect("session coordinator registry poisoned");
        let mut coordinator = self
            .state
            .lock()
            .expect("session coordinator state poisoned");
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
}
