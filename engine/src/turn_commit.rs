//! Durable marker for the chat message + live-state commit boundary.
//!
//! A marker is created before either resource is mutated and removed only
//! after both stages complete.  A surviving or unreadable marker is a
//! conservative recovery signal: the Coordinator must reject new mutations
//! until a later recovery slice inspects and resolves it.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};

const SCHEMA_VERSION: u32 = 1;
const MARKER_FILE: &str = "turn_commit.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnCommitPhase {
    Prepared,
    MessageCommitted,
    StateCommitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TurnCommitMarker {
    schema_version: u32,
    pub(crate) generation_id: String,
    pub(crate) phase: TurnCommitPhase,
    message_expected: bool,
    state_expected: bool,
}

pub(crate) struct TurnCommit {
    path: PathBuf,
    marker: TurnCommitMarker,
}

impl TurnCommit {
    pub(crate) fn begin(
        data_root: &Path,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        generation_id: String,
        message_expected: bool,
        state_expected: bool,
    ) -> Result<Self, AirpError> {
        let path = marker_path(data_root, character_id, session_id);
        let parent = path
            .parent()
            .ok_or_else(|| AirpError::Internal("turn commit marker has no parent".to_string()))?;
        fs::create_dir_all(parent)?;
        let marker = TurnCommitMarker {
            schema_version: SCHEMA_VERSION,
            generation_id,
            phase: TurnCommitPhase::Prepared,
            message_expected,
            state_expected,
        };
        persist(&path, &marker)?;
        Ok(Self { path, marker })
    }

    pub(crate) fn mark_message_committed(&mut self) -> Result<(), AirpError> {
        if self.marker.phase != TurnCommitPhase::Prepared {
            return Err(AirpError::Internal(
                "turn commit message stage is out of order".to_string(),
            ));
        }
        self.marker.phase = TurnCommitPhase::MessageCommitted;
        persist(&self.path, &self.marker)
    }

    pub(crate) fn mark_state_committed(&mut self) -> Result<(), AirpError> {
        if self.marker.phase != TurnCommitPhase::MessageCommitted {
            return Err(AirpError::Internal(
                "turn commit state stage is out of order".to_string(),
            ));
        }
        self.marker.phase = TurnCommitPhase::StateCommitted;
        persist(&self.path, &self.marker)
    }

    pub(crate) fn complete(self) -> Result<(), AirpError> {
        if self.marker.phase != TurnCommitPhase::StateCommitted {
            return Err(AirpError::Internal(
                "turn commit cannot complete before all stages".to_string(),
            ));
        }
        fs::remove_file(&self.path)?;
        if let Some(parent) = self.path.parent() {
            crate::revision::atomic::sync_dir(parent)?;
        }
        Ok(())
    }
}

/// Returns the pending marker. Any unreadable or unsupported marker remains a
/// recovery signal, with its generation id unavailable.
pub(crate) fn pending_turn(
    data_root: &Path,
    character_id: &CharacterId,
    session_id: Option<&SessionId>,
) -> Option<TurnCommitMarker> {
    let path = marker_path(data_root, character_id, session_id);
    if !path.exists() {
        return None;
    }
    match fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<TurnCommitMarker>(&bytes).map_err(|error| error.to_string())
        }) {
        Ok(marker) if marker.schema_version == SCHEMA_VERSION => Some(marker),
        Ok(marker) => {
            tracing::error!(
                path = %path.display(),
                schema_version = marker.schema_version,
                "unsupported turn commit marker requires recovery"
            );
            Some(TurnCommitMarker {
                schema_version: marker.schema_version,
                generation_id: String::new(),
                phase: marker.phase,
                message_expected: marker.message_expected,
                state_expected: marker.state_expected,
            })
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "unreadable turn commit marker requires recovery");
            Some(TurnCommitMarker {
                schema_version: 0,
                generation_id: String::new(),
                phase: TurnCommitPhase::Prepared,
                message_expected: true,
                state_expected: true,
            })
        }
    }
}

fn marker_path(
    data_root: &Path,
    character_id: &CharacterId,
    session_id: Option<&SessionId>,
) -> PathBuf {
    let character_root = data_root.join("characters").join(character_id.as_str());
    let session_root = match session_id {
        Some(session_id) => character_root.join("sessions").join(session_id.to_string()),
        None => character_root,
    };
    session_root.join("history").join(MARKER_FILE)
}

fn persist(path: &Path, marker: &TurnCommitMarker) -> Result<(), AirpError> {
    crate::data_dir::replace_file(path, &serde_json::to_vec_pretty(marker)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_records_progress_and_is_removed_only_on_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-progress").unwrap();
        let mut commit = TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "generation-a".to_string(),
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::Prepared
        );
        commit.mark_message_committed().unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::MessageCommitted
        );
        commit.mark_state_committed().unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::StateCommitted
        );
        commit.complete().unwrap();
        assert!(pending_turn(tmp.path(), &character, None).is_none());
    }

    #[test]
    fn unreadable_marker_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-corrupt").unwrap();
        let path = marker_path(tmp.path(), &character, None);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{not-json").unwrap();

        let marker = pending_turn(tmp.path(), &character, None).unwrap();
        assert!(marker.generation_id.is_empty());
        assert_eq!(marker.phase, TurnCommitPhase::Prepared);
    }

    #[test]
    fn named_session_marker_is_isolated_from_other_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-named").unwrap();
        let first = SessionId::new();
        let second = SessionId::new();
        let mut commit = TurnCommit::begin(
            tmp.path(),
            &character,
            Some(&first),
            "generation-named".to_string(),
            true,
            false,
        )
        .unwrap();

        assert!(pending_turn(tmp.path(), &character, Some(&first)).is_some());
        assert!(pending_turn(tmp.path(), &character, Some(&second)).is_none());
        assert!(pending_turn(tmp.path(), &character, None).is_none());
        commit.mark_message_committed().unwrap();
        commit.mark_state_committed().unwrap();
        commit.complete().unwrap();
    }

    #[test]
    fn marker_cannot_be_cleared_before_all_stages_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-order").unwrap();
        let commit = TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "generation-order".to_string(),
            true,
            true,
        )
        .unwrap();

        assert!(matches!(commit.complete(), Err(AirpError::Internal(_))));
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::Prepared
        );
    }
}
