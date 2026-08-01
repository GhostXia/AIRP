//! Durable marker for the chat message + live-state + current-volume commit
//! boundary.
//!
//! A marker is created before either resource is mutated and removed only
//! after both stages complete.  A surviving or unreadable marker is a
//! conservative recovery signal: the Coordinator must reject new mutations
//! until a later recovery slice inspects and resolves it.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};

const SCHEMA_VERSION: u32 = 2;
const MARKER_FILE: &str = "turn_commit.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnCommitPhase {
    Prepared,
    MessageCommitted,
    StateCommitted,
    VolumeCommitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TurnCommitMarker {
    schema_version: u32,
    pub(crate) generation_id: String,
    pub(crate) phase: TurnCommitPhase,
    message_expected: bool,
    state_expected: bool,
    volume_expected: bool,
}

pub(crate) struct TurnCommit {
    path: PathBuf,
    marker: TurnCommitMarker,
    completed: bool,
}

impl TurnCommit {
    pub(crate) fn begin(
        data_root: &Path,
        character_id: &CharacterId,
        session_id: Option<&SessionId>,
        generation_id: String,
        message_expected: bool,
        state_expected: bool,
        volume_expected: bool,
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
            volume_expected,
        };
        persist_new(&path, &marker)?;
        Ok(Self {
            path,
            marker,
            completed: false,
        })
    }

    pub(crate) fn mark_message_committed(&mut self) -> Result<(), AirpError> {
        if self.marker.phase != TurnCommitPhase::Prepared {
            return Err(AirpError::Internal(
                "turn commit message stage is out of order".to_string(),
            ));
        }
        if self.marker.message_expected {
            self.marker.phase = TurnCommitPhase::MessageCommitted;
            persist(&self.path, &self.marker)?;
        }
        Ok(())
    }

    pub(crate) fn mark_state_committed(&mut self) -> Result<(), AirpError> {
        let expected_phase = if self.marker.message_expected {
            TurnCommitPhase::MessageCommitted
        } else {
            TurnCommitPhase::Prepared
        };
        if self.marker.phase != expected_phase {
            return Err(AirpError::Internal(
                "turn commit state stage is out of order".to_string(),
            ));
        }
        if self.marker.state_expected {
            self.marker.phase = TurnCommitPhase::StateCommitted;
            persist(&self.path, &self.marker)?;
        }
        Ok(())
    }

    pub(crate) fn mark_volume_committed(&mut self) -> Result<(), AirpError> {
        let expected_phase = if self.marker.state_expected {
            TurnCommitPhase::StateCommitted
        } else if self.marker.message_expected {
            TurnCommitPhase::MessageCommitted
        } else {
            TurnCommitPhase::Prepared
        };
        if self.marker.phase != expected_phase {
            return Err(AirpError::Internal(
                "turn commit volume stage is out of order".to_string(),
            ));
        }
        if self.marker.volume_expected {
            self.marker.phase = TurnCommitPhase::VolumeCommitted;
            persist(&self.path, &self.marker)?;
        }
        Ok(())
    }

    pub(crate) fn complete(mut self) -> Result<(), AirpError> {
        let expected_phase = if self.marker.volume_expected {
            TurnCommitPhase::VolumeCommitted
        } else if self.marker.state_expected {
            TurnCommitPhase::StateCommitted
        } else if self.marker.message_expected {
            TurnCommitPhase::MessageCommitted
        } else {
            TurnCommitPhase::Prepared
        };
        if self.marker.phase != expected_phase {
            return Err(AirpError::Internal(
                "turn commit cannot complete before all stages".to_string(),
            ));
        }
        fs::remove_file(&self.path)?;
        if let Some(parent) = self.path.parent() {
            crate::revision::atomic::sync_dir(parent)?;
        }
        self.completed = true;
        Ok(())
    }
}

impl Drop for TurnCommit {
    fn drop(&mut self) {
        if !self.completed {
            record_recovery_signal(&self.path, &self.marker);
        }
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
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "unreadable turn commit marker requires recovery");
            return Some(unreadable_marker());
        }
    };
    match serde_json::from_slice::<TurnCommitMarker>(&bytes).map_err(|error| error.to_string()) {
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
                volume_expected: marker.volume_expected,
            })
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "unreadable turn commit marker requires recovery");
            Some(unreadable_marker())
        }
    }
}

fn unreadable_marker() -> TurnCommitMarker {
    TurnCommitMarker {
        schema_version: 0,
        generation_id: String::new(),
        phase: TurnCommitPhase::Prepared,
        message_expected: true,
        state_expected: true,
        volume_expected: true,
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

fn persist_new(path: &Path, marker: &TurnCommitMarker) -> Result<(), AirpError> {
    let bytes = serde_json::to_vec_pretty(marker)?;
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(AirpError::Conflict("session_recovery_required".to_string()));
        }
        Err(error) => return Err(error.into()),
    };
    let write_result = (|| -> Result<(), AirpError> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        if let Some(parent) = path.parent() {
            crate::revision::atomic::sync_dir(parent)?;
        }
        Ok(())
    })();
    if let Err(error) = write_result {
        record_recovery_signal(path, marker);
        return Err(error);
    }
    Ok(())
}

fn record_recovery_signal(path: &Path, marker: &TurnCommitMarker) {
    tracing::error!(
        event = "turn_commit_recovery_required",
        path = %path.display(),
        generation_id = %marker.generation_id,
        phase = ?marker.phase,
        "turn commit marker retained for recovery"
    );
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
            false,
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
    fn unsupported_schema_fails_closed_without_generation_id() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-future-schema").unwrap();
        let path = marker_path(tmp.path(), &character, None);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&TurnCommitMarker {
                schema_version: SCHEMA_VERSION + 1,
                generation_id: "future-generation".to_string(),
                phase: TurnCommitPhase::Prepared,
                message_expected: true,
                state_expected: true,
                volume_expected: true,
            })
            .unwrap(),
        )
        .unwrap();

        let marker = pending_turn(tmp.path(), &character, None).unwrap();
        assert!(marker.generation_id.is_empty());
        assert_eq!(marker.schema_version, SCHEMA_VERSION + 1);
    }

    #[test]
    fn begin_does_not_overwrite_a_pending_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-existing").unwrap();
        let _pending = TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "generation-first".to_string(),
            true,
            true,
            false,
        )
        .unwrap();

        assert!(matches!(
            TurnCommit::begin(
                tmp.path(),
                &character,
                None,
                "generation-second".to_string(),
                true,
                true,
                false,
            ),
            Err(AirpError::Conflict(message)) if message == "session_recovery_required"
        ));
        assert_eq!(
            pending_turn(tmp.path(), &character, None)
                .unwrap()
                .generation_id,
            "generation-first"
        );
    }

    #[test]
    fn skipped_stages_do_not_report_false_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-skipped-stages").unwrap();
        let mut state_only = TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "generation-state".to_string(),
            false,
            true,
            false,
        )
        .unwrap();
        state_only.mark_message_committed().unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::Prepared
        );
        state_only.mark_state_committed().unwrap();
        state_only.complete().unwrap();

        let mut message_only = TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "generation-message".to_string(),
            true,
            false,
            false,
        )
        .unwrap();
        message_only.mark_message_committed().unwrap();
        message_only.mark_state_committed().unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::MessageCommitted
        );
        message_only.complete().unwrap();
    }

    #[test]
    fn marker_tracks_volume_before_completion() {
        let tmp = tempfile::tempdir().unwrap();
        let character = CharacterId::new("marker-volume").unwrap();
        let mut commit = TurnCommit::begin(
            tmp.path(),
            &character,
            None,
            "generation-volume".to_string(),
            true,
            true,
            true,
        )
        .unwrap();

        commit.mark_message_committed().unwrap();
        commit.mark_state_committed().unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::StateCommitted
        );
        commit.mark_volume_committed().unwrap();
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::VolumeCommitted
        );
        commit.complete().unwrap();
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
            false,
        )
        .unwrap();

        assert!(matches!(commit.complete(), Err(AirpError::Internal(_))));
        assert_eq!(
            pending_turn(tmp.path(), &character, None).unwrap().phase,
            TurnCommitPhase::Prepared
        );
    }
}
