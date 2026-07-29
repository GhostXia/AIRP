//! Explicit, copy-only adapters from legacy chat/scene/Council data into the
//! Engine-owned Conversation contract.
//!
//! Legacy APIs and source files remain authoritative during the compatibility
//! window. Migration never runs at startup, never rewrites a legacy source, and
//! never guesses a speaker when the source cannot prove attribution.

use crate::adapter::MessageRole;
use crate::conversation::{
    AppendConversationEventRequest, ConversationParticipant, ConversationResourceRef,
    ConversationService, CreateConversationRequest,
};
use crate::error::AirpError;
use crate::types::{CharacterId, SceneId, SessionId, UserId};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const CONVERSATION_MIGRATION_SCHEMA_VERSION: u32 = 1;
pub const CONVERSATION_COMPAT_ADAPTER_VERSION: &str = "airp.conversation.compat.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LegacyConversationSource {
    CharacterChat {
        character_id: CharacterId,
        #[serde(default)]
        session_id: Option<SessionId>,
    },
    SceneGroup {
        scene_id: SceneId,
    },
    Council {
        character_id: CharacterId,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanConversationMigrationRequest {
    #[serde(default)]
    pub user_id: Option<UserId>,
    pub source: LegacyConversationSource,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteConversationMigrationRequest {
    #[serde(default)]
    pub user_id: Option<UserId>,
    pub migration_id: String,
    pub source: LegacyConversationSource,
    pub expected_source_sha256: String,
    #[serde(default)]
    pub title: Option<String>,
    pub confirm: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConversationMigrationScope {
    #[serde(default)]
    pub user_id: Option<UserId>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMigrationStatus {
    Ready,
    NeedsReview,
    Completed,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationMigrationReport {
    pub schema_version: u32,
    pub adapter_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_id: Option<String>,
    pub source: LegacyConversationSource,
    pub status: ConversationMigrationStatus,
    pub source_sha256: String,
    pub source_record_count: usize,
    pub migrated_message_count: usize,
    pub unresolved_record_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_sha256: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rolled_back_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegacyConversationExport {
    pub schema_version: u32,
    pub adapter_version: String,
    pub source: LegacyConversationSource,
    pub source_metadata: Value,
    pub records: Vec<LegacyConversationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegacyConversationRecord {
    pub source_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
    #[serde(default)]
    pub selected_candidate: usize,
}

struct PreparedMigration {
    report: ConversationMigrationReport,
    export: LegacyConversationExport,
    create: Option<CreateConversationRequest>,
    events: Vec<AppendConversationEventRequest>,
}

pub fn plan_conversation_migration(
    root: &Path,
    request: PlanConversationMigrationRequest,
) -> Result<ConversationMigrationReport, AirpError> {
    Ok(prepare_migration(root, request.source, request.title)?.report)
}

pub async fn execute_conversation_migration(
    root: PathBuf,
    request: ExecuteConversationMigrationRequest,
) -> Result<ConversationMigrationReport, AirpError> {
    validate_migration_id(&request.migration_id)?;
    if !request.confirm {
        return Err(AirpError::BadRequest(
            "explicit migration requires confirm=true".to_string(),
        ));
    }
    let migration_id = request.migration_id.clone();
    let migration_dir = migration_dir(&root, &migration_id);
    if migration_dir.is_dir() {
        let existing = load_report(&migration_dir)?;
        if existing.source == request.source
            && constant_time_hex_eq(&existing.source_sha256, &request.expected_source_sha256)
            && existing.status == ConversationMigrationStatus::Completed
        {
            verify_completed_migration(&root, &migration_dir, &existing)?;
            return Ok(existing);
        }
        return Err(AirpError::Conflict(format!(
            "migration_id {} is already used",
            request.migration_id
        )));
    }
    let source = request.source.clone();
    let title = request.title.clone();
    let prepare_root = root.clone();
    let prepared =
        tokio::task::spawn_blocking(move || prepare_migration(&prepare_root, source, title))
            .await
            .map_err(|error| {
                AirpError::Internal(format!("migration planning task failed: {error}"))
            })??;
    if prepared.report.status != ConversationMigrationStatus::Ready {
        return Err(AirpError::BadRequest(
            "legacy source requires manual review and cannot be auto-migrated".to_string(),
        ));
    }
    if !constant_time_hex_eq(
        &prepared.report.source_sha256,
        &request.expected_source_sha256,
    ) {
        return Err(AirpError::Conflict(
            "legacy source changed after migration planning".to_string(),
        ));
    }

    fs::create_dir_all(migration_dir.parent().expect("migration dir has parent"))?;
    if let Err(error) = fs::create_dir(&migration_dir) {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(AirpError::Conflict(format!(
                "migration_id {} is already used",
                request.migration_id
            )));
        }
        return Err(error.into());
    }

    let export_bytes = serde_json::to_vec_pretty(&prepared.export)?;
    let backup_sha256 = sha256_bytes(&export_bytes);
    crate::data_dir::replace_file(&migration_dir.join("source-export.json"), &export_bytes)?;
    verify_file_digest(
        &migration_dir.join("source-export.json"),
        &backup_sha256,
        "migration backup",
    )?;
    let mut reserved_report = prepared.report.clone();
    reserved_report.migration_id = Some(migration_id.clone());
    reserved_report.backup_sha256 = Some(backup_sha256.clone());
    write_report(&migration_dir, &reserved_report)?;

    let service = ConversationService::new(&root);
    let create = prepared.create.ok_or_else(|| {
        AirpError::Internal("ready migration did not produce a create request".to_string())
    })?;
    let manifest = match service.create(create).await {
        Ok(manifest) => manifest,
        Err(error) => {
            let report = failed_report(prepared.report, migration_id, backup_sha256);
            let _ = write_report(&migration_dir, &report);
            return Err(error);
        }
    };
    let _write_guard = service.acquire_write(manifest.conversation_id).await;
    for event in prepared.events {
        if let Err(error) = service
            .append_event_locked_async(manifest.conversation_id, event)
            .await
        {
            let _ = remove_conversation_dir(&service, manifest.conversation_id);
            let report =
                failed_report(prepared.report, migration_id.clone(), backup_sha256.clone());
            let _ = write_report(&migration_dir, &report);
            return Err(error);
        }
    }
    let conversation_sha256 = conversation_digest(&service, manifest.conversation_id)?;
    let mut report = prepared.report;
    report.migration_id = Some(migration_id);
    report.status = ConversationMigrationStatus::Completed;
    report.backup_sha256 = Some(backup_sha256);
    report.conversation_id = Some(manifest.conversation_id);
    report.conversation_sha256 = Some(conversation_sha256);
    write_report(&migration_dir, &report)?;
    verify_completed_migration(&root, &migration_dir, &report)?;
    Ok(report)
}

pub async fn rollback_conversation_migration(
    root: PathBuf,
    migration_id: &str,
) -> Result<ConversationMigrationReport, AirpError> {
    validate_migration_id(migration_id)?;
    let migration_dir = migration_dir(&root, migration_id);
    let mut report = load_report(&migration_dir)?;
    if report.status == ConversationMigrationStatus::RolledBack {
        verify_file_digest(
            &migration_dir.join("source-export.json"),
            report.backup_sha256.as_deref().unwrap_or_default(),
            "migration backup",
        )?;
        return Ok(report);
    }
    if report.status != ConversationMigrationStatus::Completed {
        return Err(AirpError::Conflict(
            "only a completed migration can be rolled back".to_string(),
        ));
    }
    let service = ConversationService::new(&root);
    let conversation_id = report
        .conversation_id
        .ok_or_else(|| AirpError::Internal("migration report has no conversation".to_string()))?;
    let _write_guard = service.acquire_write(conversation_id).await;
    verify_completed_migration(&root, &migration_dir, &report)?;
    remove_conversation_dir(&service, conversation_id)?;
    report.status = ConversationMigrationStatus::RolledBack;
    report.rolled_back_at = Some(Utc::now().to_rfc3339());
    write_report(&migration_dir, &report)?;
    Ok(report)
}

pub fn load_migration_export(
    root: &Path,
    migration_id: &str,
) -> Result<LegacyConversationExport, AirpError> {
    validate_migration_id(migration_id)?;
    let directory = migration_dir(root, migration_id);
    let report = load_report(&directory)?;
    let expected = report
        .backup_sha256
        .as_deref()
        .ok_or_else(|| AirpError::Conflict("migration has no verified backup".to_string()))?;
    let path = directory.join("source-export.json");
    verify_file_digest(&path, expected, "migration backup")?;
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn prepare_migration(
    root: &Path,
    source: LegacyConversationSource,
    title: Option<String>,
) -> Result<PreparedMigration, AirpError> {
    match source {
        LegacyConversationSource::CharacterChat {
            character_id,
            session_id,
        } => prepare_character_chat(root, character_id, session_id, title),
        LegacyConversationSource::Council { character_id } => {
            prepare_council(root, character_id, title)
        }
        LegacyConversationSource::SceneGroup { scene_id } => {
            prepare_scene_group(root, scene_id, title)
        }
    }
}

fn prepare_character_chat(
    root: &Path,
    character_id: CharacterId,
    session_id: Option<SessionId>,
    title: Option<String>,
) -> Result<PreparedMigration, AirpError> {
    let log = crate::chat_store::ChatLog::load_existing_read_only(
        root,
        character_id.as_str(),
        session_id.as_ref(),
    )?
    .ok_or_else(|| AirpError::NotFound("legacy chat source not found".to_string()))?;
    let active_indices: HashSet<usize> = log.active_path_indices().into_iter().collect();
    let mut records = Vec::with_capacity(log.messages.len());
    let mut events = Vec::with_capacity(log.messages.len());
    let mut migrated_message_count = 0usize;
    let mut unresolved_record_count = 0usize;
    for (index, message) in log.messages.iter().enumerate() {
        let active = active_indices.contains(&index);
        let (speaker_id, role, event_kind) = match message.role {
            MessageRole::User => (
                Some("user".to_string()),
                Some("user"),
                if active {
                    "message.created"
                } else {
                    "legacy.chat.branch_message"
                },
            ),
            MessageRole::Assistant => (
                Some(format!("character:{character_id}")),
                Some("assistant"),
                if active {
                    "message.created"
                } else {
                    "legacy.chat.branch_message"
                },
            ),
            MessageRole::System => {
                unresolved_record_count += 1;
                (None, None, "legacy.chat.system_message")
            }
        };
        if active && role.is_some() {
            migrated_message_count += 1;
        }
        let source_id = log
            .message_ids
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("legacy-index-{index}"));
        let candidates = log
            .message_candidates
            .get(index)
            .cloned()
            .unwrap_or_default();
        let selected_candidate = log.message_swipe_index.get(index).copied().unwrap_or(0);
        let parent_id = log.message_parents.get(index).cloned().flatten();
        let occurred_at = log.message_timestamps.get(index).cloned().flatten();
        records.push(LegacyConversationRecord {
            source_id: source_id.clone(),
            kind: event_kind.to_string(),
            speaker_id: speaker_id.clone(),
            role: role.map(str::to_string),
            content: message.content.clone(),
            occurred_at: occurred_at.clone(),
            parent_id: parent_id.clone(),
            active,
            candidates: candidates.clone(),
            selected_candidate,
        });
        events.push(AppendConversationEventRequest {
            user_id: None,
            kind: event_kind.to_string(),
            actor_id: speaker_id,
            causation_id: Some(source_id.clone()),
            correlation_id: None,
            payload: serde_json::json!({
                "content": message.content,
                "role": role,
            }),
            extensions: BTreeMap::from([(
                "airp.compat.v1".to_string(),
                serde_json::json!({
                    "source_message_id": source_id,
                    "source_timestamp": occurred_at,
                    "parent_id": parent_id,
                    "active": active,
                    "candidates": candidates,
                    "selected_candidate": selected_candidate,
                }),
            )]),
            expected_next_sequence: Some(index as u64),
        });
    }
    let source = LegacyConversationSource::CharacterChat {
        character_id: character_id.clone(),
        session_id,
    };
    let export = LegacyConversationExport {
        schema_version: CONVERSATION_MIGRATION_SCHEMA_VERSION,
        adapter_version: CONVERSATION_COMPAT_ADAPTER_VERSION.to_string(),
        source: source.clone(),
        source_metadata: serde_json::json!({
            "legacy_session_id": log.session_id,
            "created_at": log.created_at,
            "updated_at": log.updated_at,
            "active_leaf": log.active_leaf,
        }),
        records,
    };
    let source_sha256 = canonical_export_digest(&export)?;
    let mut warnings = Vec::new();
    if unresolved_record_count > 0 {
        warnings.push(
            "system messages are preserved as non-projected legacy events because they have no speaker"
                .to_string(),
        );
    }
    if migrated_message_count < export.records.len().saturating_sub(unresolved_record_count) {
        warnings.push(
            "inactive branch messages are preserved as non-projected legacy events".to_string(),
        );
    }
    let participants = vec![
        ConversationParticipant {
            participant_id: "user".to_string(),
            kind: "user".to_string(),
            display_name: None,
            resource: None,
            extensions: BTreeMap::new(),
        },
        ConversationParticipant {
            participant_id: format!("character:{character_id}"),
            kind: "character".to_string(),
            display_name: None,
            resource: Some(ConversationResourceRef {
                kind: "character".to_string(),
                id: character_id.to_string(),
                revision: None,
                extensions: BTreeMap::new(),
            }),
            extensions: BTreeMap::new(),
        },
    ];
    let create = CreateConversationRequest {
        user_id: None,
        title: title.or_else(|| Some(format!("Legacy chat: {character_id}"))),
        participants,
        resources: vec![ConversationResourceRef {
            kind: "legacy_chat".to_string(),
            id: log.session_id,
            revision: Some(source_sha256.clone()),
            extensions: BTreeMap::new(),
        }],
        orchestration: None,
        extensions: BTreeMap::from([(
            "airp.compat.v1".to_string(),
            serde_json::json!({
                "adapter_version": CONVERSATION_COMPAT_ADAPTER_VERSION,
                "source_sha256": source_sha256,
            }),
        )]),
    };
    Ok(PreparedMigration {
        report: base_report(
            source,
            ConversationMigrationStatus::Ready,
            source_sha256,
            export.records.len(),
            migrated_message_count,
            unresolved_record_count,
            warnings,
        ),
        export,
        create: Some(create),
        events,
    })
}

fn prepare_council(
    root: &Path,
    character_id: CharacterId,
    title: Option<String>,
) -> Result<PreparedMigration, AirpError> {
    let council = crate::agent::council::load_council(root, character_id.as_str())?
        .ok_or_else(|| AirpError::NotFound("legacy Council source not found".to_string()))?;
    let configured: HashSet<_> = council.config.participants.iter().cloned().collect();
    let unresolved_speakers: Vec<_> = council
        .turns
        .iter()
        .filter(|turn| !turn.is_intervention && !configured.contains(&turn.speaker))
        .map(|turn| turn.speaker.clone())
        .collect();
    let has_user = council.turns.iter().any(|turn| turn.is_intervention);
    let mut participants = Vec::new();
    if has_user {
        participants.push(ConversationParticipant {
            participant_id: "user".to_string(),
            kind: "user".to_string(),
            display_name: None,
            resource: None,
            extensions: BTreeMap::new(),
        });
    }
    let mut unique = HashSet::new();
    for participant in &council.config.participants {
        if unique.insert(participant.clone()) {
            participants.push(ConversationParticipant {
                participant_id: format!("character:{participant}"),
                kind: "character".to_string(),
                display_name: None,
                resource: Some(ConversationResourceRef {
                    kind: "character".to_string(),
                    id: participant.clone(),
                    revision: None,
                    extensions: BTreeMap::new(),
                }),
                extensions: BTreeMap::new(),
            });
        }
    }
    let source = LegacyConversationSource::Council {
        character_id: character_id.clone(),
    };
    let records: Vec<_> = council
        .turns
        .iter()
        .enumerate()
        .map(|(index, turn)| LegacyConversationRecord {
            source_id: format!("round-{}-turn-{index}", turn.round),
            kind: "message.created".to_string(),
            speaker_id: Some(if turn.is_intervention {
                "user".to_string()
            } else {
                format!("character:{}", turn.speaker)
            }),
            role: Some(if turn.is_intervention {
                "user".to_string()
            } else {
                "assistant".to_string()
            }),
            content: turn.content.clone(),
            occurred_at: None,
            parent_id: None,
            active: true,
            candidates: Vec::new(),
            selected_candidate: 0,
        })
        .collect();
    let export = LegacyConversationExport {
        schema_version: CONVERSATION_MIGRATION_SCHEMA_VERSION,
        adapter_version: CONVERSATION_COMPAT_ADAPTER_VERSION.to_string(),
        source: source.clone(),
        source_metadata: serde_json::to_value(&council)?,
        records,
    };
    let source_sha256 = canonical_export_digest(&export)?;
    if !unresolved_speakers.is_empty() {
        return Ok(PreparedMigration {
            report: base_report(
                source,
                ConversationMigrationStatus::NeedsReview,
                source_sha256,
                export.records.len(),
                0,
                unresolved_speakers.len(),
                vec![format!(
                    "Council contains speakers absent from its participant list: {}",
                    unresolved_speakers.join(", ")
                )],
            ),
            export,
            create: None,
            events: Vec::new(),
        });
    }
    let events = export
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| AppendConversationEventRequest {
            user_id: None,
            kind: "message.created".to_string(),
            actor_id: record.speaker_id.clone(),
            causation_id: Some(record.source_id.clone()),
            correlation_id: None,
            payload: serde_json::json!({
                "content": record.content,
                "role": record.role,
            }),
            extensions: BTreeMap::from([(
                "airp.compat.v1".to_string(),
                serde_json::json!({"source_record_id": record.source_id}),
            )]),
            expected_next_sequence: Some(index as u64),
        })
        .collect();
    let create = CreateConversationRequest {
        user_id: None,
        title: title.or_else(|| Some(format!("Council: {}", council.config.topic))),
        participants,
        resources: vec![ConversationResourceRef {
            kind: "legacy_council".to_string(),
            id: character_id.to_string(),
            revision: Some(source_sha256.clone()),
            extensions: BTreeMap::new(),
        }],
        orchestration: None,
        extensions: BTreeMap::from([(
            "airp.compat.v1".to_string(),
            serde_json::json!({
                "adapter_version": CONVERSATION_COMPAT_ADAPTER_VERSION,
                "source_sha256": source_sha256,
            }),
        )]),
    };
    Ok(PreparedMigration {
        report: base_report(
            source,
            ConversationMigrationStatus::Ready,
            source_sha256,
            export.records.len(),
            export.records.len(),
            0,
            vec![
                "legacy Council scheduling metadata is preserved in the export; no execution policy is inferred"
                    .to_string(),
            ],
        ),
        export,
        create: Some(create),
        events,
    })
}

fn prepare_scene_group(
    root: &Path,
    scene_id: SceneId,
    _title: Option<String>,
) -> Result<PreparedMigration, AirpError> {
    let scene = crate::scene::SceneConfig::load(root, &scene_id)?;
    let source = LegacyConversationSource::SceneGroup {
        scene_id: scene_id.clone(),
    };
    let export = LegacyConversationExport {
        schema_version: CONVERSATION_MIGRATION_SCHEMA_VERSION,
        adapter_version: CONVERSATION_COMPAT_ADAPTER_VERSION.to_string(),
        source: source.clone(),
        source_metadata: serde_json::to_value(scene)?,
        records: Vec::new(),
    };
    let source_sha256 = canonical_export_digest(&export)?;
    Ok(PreparedMigration {
        report: base_report(
            source,
            ConversationMigrationStatus::NeedsReview,
            source_sha256,
            0,
            0,
            1,
            vec![
                "legacy scene/group storage has shared memory but no authoritative per-message speaker attribution"
                    .to_string(),
                "no history was imported; use the scene Conversation adapter for a new empty Conversation"
                    .to_string(),
            ],
        ),
        export,
        create: None,
        events: Vec::new(),
    })
}

fn base_report(
    source: LegacyConversationSource,
    status: ConversationMigrationStatus,
    source_sha256: String,
    source_record_count: usize,
    migrated_message_count: usize,
    unresolved_record_count: usize,
    warnings: Vec<String>,
) -> ConversationMigrationReport {
    ConversationMigrationReport {
        schema_version: CONVERSATION_MIGRATION_SCHEMA_VERSION,
        adapter_version: CONVERSATION_COMPAT_ADAPTER_VERSION.to_string(),
        migration_id: None,
        source,
        status,
        source_sha256,
        source_record_count,
        migrated_message_count,
        unresolved_record_count,
        backup_sha256: None,
        conversation_id: None,
        conversation_sha256: None,
        warnings,
        created_at: Utc::now().to_rfc3339(),
        rolled_back_at: None,
    }
}

fn failed_report(
    mut report: ConversationMigrationReport,
    migration_id: String,
    backup_sha256: String,
) -> ConversationMigrationReport {
    report.migration_id = Some(migration_id);
    report.status = ConversationMigrationStatus::Failed;
    report.backup_sha256 = Some(backup_sha256);
    report
}

fn migration_dir(root: &Path, migration_id: &str) -> PathBuf {
    root.join("conversation_migrations").join(migration_id)
}

fn validate_migration_id(migration_id: &str) -> Result<(), AirpError> {
    if !crate::ulid::is_valid_id(migration_id) {
        return Err(AirpError::BadRequest(
            "migration_id must be a valid durable ID".to_string(),
        ));
    }
    Ok(())
}

fn canonical_export_digest(export: &LegacyConversationExport) -> Result<String, AirpError> {
    Ok(sha256_bytes(&serde_json::to_vec(export)?))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn constant_time_hex_eq(left: &str, right: &str) -> bool {
    if left.len() != 64 || right.len() != 64 {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn verify_file_digest(path: &Path, expected: &str, label: &str) -> Result<(), AirpError> {
    let actual = sha256_bytes(&fs::read(path)?);
    if !constant_time_hex_eq(&actual, expected) {
        return Err(AirpError::Conflict(format!(
            "{label} integrity verification failed"
        )));
    }
    Ok(())
}

fn write_report(
    migration_dir: &Path,
    report: &ConversationMigrationReport,
) -> Result<(), AirpError> {
    crate::data_dir::replace_file(
        &migration_dir.join("report.json"),
        &serde_json::to_vec_pretty(report)?,
    )
}

fn load_report(migration_dir: &Path) -> Result<ConversationMigrationReport, AirpError> {
    let path = migration_dir.join("report.json");
    if !path.is_file() {
        return Err(AirpError::NotFound(
            "conversation migration report not found".to_string(),
        ));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn conversation_digest(
    service: &ConversationService,
    conversation_id: SessionId,
) -> Result<String, AirpError> {
    let directory = service.conversation_dir(conversation_id);
    let manifest = fs::read(directory.join("manifest.json"))?;
    let events = match fs::read(directory.join("events.jsonl")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let mut hasher = Sha256::new();
    hasher.update(b"AIRP-CONVERSATION-MIGRATION\0v1\0");
    hasher.update((manifest.len() as u64).to_le_bytes());
    hasher.update(manifest);
    hasher.update((events.len() as u64).to_le_bytes());
    hasher.update(events);
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_completed_migration(
    root: &Path,
    migration_dir: &Path,
    report: &ConversationMigrationReport,
) -> Result<(), AirpError> {
    let backup_sha256 = report
        .backup_sha256
        .as_deref()
        .ok_or_else(|| AirpError::Internal("migration report has no backup digest".to_string()))?;
    verify_file_digest(
        &migration_dir.join("source-export.json"),
        backup_sha256,
        "migration backup",
    )?;
    let conversation_id = report
        .conversation_id
        .ok_or_else(|| AirpError::Internal("migration report has no conversation".to_string()))?;
    let expected = report.conversation_sha256.as_deref().ok_or_else(|| {
        AirpError::Internal("migration report has no conversation digest".to_string())
    })?;
    let actual = conversation_digest(&ConversationService::new(root), conversation_id)?;
    if !constant_time_hex_eq(&actual, expected) {
        return Err(AirpError::Conflict(
            "migrated Conversation changed; rollback requires manual review".to_string(),
        ));
    }
    Ok(())
}

fn remove_conversation_dir(
    service: &ConversationService,
    conversation_id: SessionId,
) -> Result<(), AirpError> {
    let directory = service.conversation_dir(conversation_id);
    let expected_parent = service.data_root().join("conversations");
    if directory.parent() != Some(expected_parent.as_path()) {
        return Err(AirpError::Internal(
            "refusing to remove conversation outside storage root".to_string(),
        ));
    }
    if directory.is_dir() {
        fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::ChatMessage;
    use crate::agent::council::{CouncilConfig, CouncilSession, CouncilTurn, SpeakingOrder};
    use crate::chat_store::ChatLog;
    use tempfile::tempdir;

    fn plan(source: LegacyConversationSource) -> PlanConversationMigrationRequest {
        PlanConversationMigrationRequest {
            user_id: None,
            source,
            title: None,
        }
    }

    #[tokio::test]
    async fn character_chat_migration_preserves_source_and_rolls_back() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let character = CharacterId::new("alice").unwrap();
        let mut log = ChatLog::new(character.as_str());
        log.append(
            root,
            ChatMessage {
                role: MessageRole::User,
                content: "Hello".to_string(),
            },
        )
        .unwrap();
        log.append(
            root,
            ChatMessage {
                role: MessageRole::Assistant,
                content: "Hi".to_string(),
            },
        )
        .unwrap();
        let source = LegacyConversationSource::CharacterChat {
            character_id: character,
            session_id: None,
        };
        let planned = plan_conversation_migration(root, plan(source.clone())).unwrap();
        assert_eq!(planned.status, ConversationMigrationStatus::Ready);
        assert_eq!(planned.migrated_message_count, 2);
        let source_before = fs::read(root.join("characters/alice/history/chat_log.jsonl")).unwrap();
        let planned_sha256 = planned.source_sha256.clone();

        let migration_id = crate::ulid::new_id();
        let completed = execute_conversation_migration(
            root.to_path_buf(),
            ExecuteConversationMigrationRequest {
                user_id: None,
                migration_id: migration_id.clone(),
                source: source.clone(),
                expected_source_sha256: planned_sha256.clone(),
                title: None,
                confirm: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, ConversationMigrationStatus::Completed);
        assert_eq!(
            fs::read(root.join("characters/alice/history/chat_log.jsonl")).unwrap(),
            source_before
        );
        let conversation_id = completed.conversation_id.unwrap();
        let projection = ConversationService::new(root)
            .message_projection(conversation_id)
            .await
            .unwrap();
        assert_eq!(projection.messages.len(), 2);
        let export = load_migration_export(root, &migration_id).unwrap();
        assert_eq!(export.records.len(), 2);
        let export_path = root
            .join("conversation_migrations")
            .join(&migration_id)
            .join("source-export.json");
        let verified_export = fs::read(&export_path).unwrap();
        fs::write(&export_path, b"{}").unwrap();
        assert!(matches!(
            load_migration_export(root, &migration_id),
            Err(AirpError::Conflict(_))
        ));
        fs::write(&export_path, verified_export).unwrap();
        log.append(
            root,
            ChatMessage {
                role: MessageRole::User,
                content: "legacy source continues".to_string(),
            },
        )
        .unwrap();
        let replayed = execute_conversation_migration(
            root.to_path_buf(),
            ExecuteConversationMigrationRequest {
                user_id: None,
                migration_id: migration_id.clone(),
                source,
                expected_source_sha256: planned_sha256,
                title: None,
                confirm: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(replayed.conversation_id, Some(conversation_id));

        let rolled_back = rollback_conversation_migration(root.to_path_buf(), &migration_id)
            .await
            .unwrap();
        assert_eq!(rolled_back.status, ConversationMigrationStatus::RolledBack);
        assert!(!root
            .join("conversations")
            .join(conversation_id.to_string())
            .exists());
        assert!(root
            .join("conversation_migrations")
            .join(migration_id)
            .join("source-export.json")
            .is_file());
    }

    #[test]
    fn legacy_json_planning_is_read_only() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let character_dir = root.join("characters/alice");
        fs::create_dir_all(&character_dir).unwrap();
        let mut legacy = ChatLog::new("alice");
        legacy.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "legacy".to_string(),
        });
        let legacy_path = character_dir.join("chat_log.json");
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&legacy_path, &original).unwrap();

        let report = plan_conversation_migration(
            root,
            plan(LegacyConversationSource::CharacterChat {
                character_id: CharacterId::new("alice").unwrap(),
                session_id: None,
            }),
        )
        .unwrap();

        assert_eq!(report.status, ConversationMigrationStatus::Ready);
        assert_eq!(fs::read(legacy_path).unwrap(), original);
        assert!(!character_dir.join("history").exists());
    }

    #[tokio::test]
    async fn migration_detects_source_drift_and_post_migration_changes() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let character = CharacterId::new("alice").unwrap();
        let mut log = ChatLog::new(character.as_str());
        log.append(
            root,
            ChatMessage {
                role: MessageRole::User,
                content: "one".to_string(),
            },
        )
        .unwrap();
        let source = LegacyConversationSource::CharacterChat {
            character_id: character.clone(),
            session_id: None,
        };
        let planned = plan_conversation_migration(root, plan(source.clone())).unwrap();
        log.append(
            root,
            ChatMessage {
                role: MessageRole::Assistant,
                content: "two".to_string(),
            },
        )
        .unwrap();
        let error = execute_conversation_migration(
            root.to_path_buf(),
            ExecuteConversationMigrationRequest {
                user_id: None,
                migration_id: crate::ulid::new_id(),
                source: source.clone(),
                expected_source_sha256: planned.source_sha256,
                title: None,
                confirm: true,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AirpError::Conflict(_)));

        let planned = plan_conversation_migration(root, plan(source.clone())).unwrap();
        let migration_id = crate::ulid::new_id();
        let completed = execute_conversation_migration(
            root.to_path_buf(),
            ExecuteConversationMigrationRequest {
                user_id: None,
                migration_id: migration_id.clone(),
                source,
                expected_source_sha256: planned.source_sha256,
                title: None,
                confirm: true,
            },
        )
        .await
        .unwrap();
        ConversationService::new(root)
            .append_event(
                completed.conversation_id.unwrap(),
                AppendConversationEventRequest {
                    user_id: None,
                    kind: "message.created".to_string(),
                    actor_id: Some("user".to_string()),
                    causation_id: None,
                    correlation_id: None,
                    payload: serde_json::json!({"role": "user", "content": "new"}),
                    extensions: BTreeMap::new(),
                    expected_next_sequence: Some(2),
                },
            )
            .await
            .unwrap();
        let error = rollback_conversation_migration(root.to_path_buf(), &migration_id)
            .await
            .unwrap_err();
        assert!(matches!(error, AirpError::Conflict(_)));
    }

    #[test]
    fn scene_group_requires_review_without_guessing_speakers() {
        let temp = tempdir().unwrap();
        let scene = crate::scene::SceneConfig {
            scene_id: SceneId::new("tavern").unwrap(),
            description: "Tavern".to_string(),
            characters: Vec::new(),
            narrator_style: String::new(),
            lorebook_merge: crate::scene::LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(temp.path()).unwrap();
        let report = plan_conversation_migration(
            temp.path(),
            plan(LegacyConversationSource::SceneGroup {
                scene_id: scene.scene_id,
            }),
        )
        .unwrap();
        assert_eq!(report.status, ConversationMigrationStatus::NeedsReview);
        assert_eq!(report.unresolved_record_count, 1);
    }

    #[tokio::test]
    async fn council_turns_keep_explicit_speaker_attribution() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let character = CharacterId::new("host").unwrap();
        let mut council = CouncilSession::new(CouncilConfig {
            topic: "Plan".to_string(),
            participants: vec!["alice".to_string(), "bob".to_string()],
            max_rounds: 1,
            order: SpeakingOrder::RoundRobin,
            allow_intervention: true,
            max_tokens_per_turn: 100,
        });
        council.turns = vec![
            CouncilTurn {
                speaker: "alice".to_string(),
                content: "A".to_string(),
                round: 1,
                is_intervention: false,
            },
            CouncilTurn {
                speaker: "user".to_string(),
                content: "U".to_string(),
                round: 1,
                is_intervention: true,
            },
        ];
        crate::agent::council::save_council(root, character.as_str(), &council).unwrap();
        let source = LegacyConversationSource::Council {
            character_id: character,
        };
        let planned = plan_conversation_migration(root, plan(source.clone())).unwrap();
        let completed = execute_conversation_migration(
            root.to_path_buf(),
            ExecuteConversationMigrationRequest {
                user_id: None,
                migration_id: crate::ulid::new_id(),
                source,
                expected_source_sha256: planned.source_sha256,
                title: None,
                confirm: true,
            },
        )
        .await
        .unwrap();
        let projection = ConversationService::new(root)
            .message_projection(completed.conversation_id.unwrap())
            .await
            .unwrap();
        assert_eq!(projection.messages[0].actor_id, "character:alice");
        assert_eq!(projection.messages[1].actor_id, "user");
    }

    #[test]
    fn council_with_unlisted_speaker_requires_review() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let character = CharacterId::new("host").unwrap();
        let mut council = CouncilSession::new(CouncilConfig {
            topic: "Plan".to_string(),
            participants: vec!["alice".to_string()],
            max_rounds: 1,
            order: SpeakingOrder::RoundRobin,
            allow_intervention: false,
            max_tokens_per_turn: 100,
        });
        council.turns.push(CouncilTurn {
            speaker: "unknown".to_string(),
            content: "?".to_string(),
            round: 1,
            is_intervention: false,
        });
        crate::agent::council::save_council(root, character.as_str(), &council).unwrap();

        let report = plan_conversation_migration(
            root,
            plan(LegacyConversationSource::Council {
                character_id: character,
            }),
        )
        .unwrap();
        assert_eq!(report.status, ConversationMigrationStatus::NeedsReview);
        assert_eq!(report.unresolved_record_count, 1);
    }
}
