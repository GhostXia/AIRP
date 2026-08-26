//! Durable, layout-only user workspace asset.
//!
//! Workspace persistence is deliberately separate from ephemeral Surface
//! projection. Only layout nodes and Widget id/type declarations are stored;
//! runtime props, Chat, Memory, Character State, Activity, bearer tokens and
//! filesystem paths have no field in the machine contract.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use airp_state_protocol::{
    migrate_v1_blueprint, validate_workspace_document, Blueprint, BlueprintV2, LayoutNodeV2,
    SurfaceRevision, WorkspaceDocumentV1, WorkspaceLayoutV1, WorkspaceNodeV1, WorkspaceWidgetV1,
    WORKSPACE_MAX_DOCUMENT_BYTES, WORKSPACE_SCHEMA_MAJOR,
};
use sha2::{Digest, Sha256};

use crate::error::AirpError;
use crate::revision::atomic::{
    commit_revision, next_content_revision, read_current_revision, CommitOptions, StagedRevision,
};
use crate::revision::manifest::{AssetKind, AssetSource, RevisionManifest};

const WORKSPACE_ID: &str = "default";
const WORKSPACE_FILE: &str = "workspace.json";
const MAX_HISTORY_ENTRIES: usize = 256;

/// Global leaf lock for the workspace asset family. AIRP has one daemon
/// writer; this serializes read-current -> validate -> commit across service
/// instances. Lock order is WORKSPACE_LOCK -> revision COMMIT_LOCK.
static WORKSPACE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug)]
pub struct WorkspaceService {
    effective_root: PathBuf,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkspaceHistoryEntry {
    pub revision: SurfaceRevision,
    pub updated_at: String,
    pub source_kind: String,
    pub parent_revision: Option<SurfaceRevision>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceExport {
    pub schema: u16,
    pub sha256: String,
    /// Exact UTF-8 JSON text covered by `sha256`, including whitespace and
    /// unknown future-major fields.
    pub raw_json: String,
    pub document: serde_json::Value,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceMigrationPlan {
    pub source: String,
    pub source_sha256: String,
    pub candidate: WorkspaceDocumentV1,
    pub warnings: Vec<String>,
    pub writes_performed: bool,
}

impl WorkspaceService {
    pub fn new(effective_root: impl AsRef<Path>) -> Self {
        Self {
            effective_root: effective_root.as_ref().to_path_buf(),
        }
    }

    pub fn read(&self) -> Result<WorkspaceDocumentV1, AirpError> {
        let _guard = workspace_guard()?;
        self.read_under_lock()
    }

    pub fn save(
        &self,
        expected_revision: u64,
        layout: WorkspaceLayoutV1,
    ) -> Result<WorkspaceDocumentV1, AirpError> {
        let _guard = workspace_guard()?;
        self.commit_layout_under_lock(expected_revision, layout, "workspace_update", None, None)
    }

    pub fn history(&self, limit: usize) -> Result<Vec<WorkspaceHistoryEntry>, AirpError> {
        let _guard = workspace_guard()?;
        self.history_under_lock(limit.clamp(1, MAX_HISTORY_ENTRIES))
    }

    /// Export the exact manifest-approved JSON bytes as a parsed JSON value.
    /// This remains available for an unknown future schema major so users can
    /// recover their asset without allowing this Engine to mutate it.
    pub fn export(&self) -> Result<WorkspaceExport, AirpError> {
        let _guard = workspace_guard()?;
        let Some(revision) = read_current_revision(&self.asset_dir())? else {
            let document = default_document();
            let bytes = serde_json::to_vec_pretty(&document)?;
            return export_from_bytes(&bytes);
        };
        let (_, bytes) = self.load_verified_bytes(revision)?;
        export_from_bytes(&bytes)
    }

    /// Restore a historical layout by creating a new forward revision. The
    /// current pointer never moves backward and concurrent changes conflict.
    pub fn rollback(
        &self,
        expected_revision: u64,
        target_revision: u64,
    ) -> Result<WorkspaceDocumentV1, AirpError> {
        let _guard = workspace_guard()?;
        let current = read_current_revision(&self.asset_dir())?.unwrap_or(0);
        if current != expected_revision {
            return Err(workspace_conflict(expected_revision, current));
        }
        if target_revision == 0 || target_revision >= current {
            return Err(AirpError::BadRequest(
                "workspace rollback target must be an earlier committed revision".to_string(),
            ));
        }
        if !self.lineage_contains_under_lock(target_revision)? {
            return Err(AirpError::NotFound(format!(
                "workspace revision {target_revision} is not in committed history"
            )));
        }
        let (_, target, target_bytes) = self.load_revision(target_revision)?;
        let target_hash = sha256_hex(&target_bytes);
        self.commit_layout_under_lock(
            expected_revision,
            target.layout,
            "workspace_rollback",
            Some(target_hash),
            Some(format!("revision:{target_revision}")),
        )
    }

    /// Convert the legacy v1 Blueprint deterministically without writing. All
    /// Widget props/state/capabilities and theme data are intentionally absent
    /// from the candidate workspace contract.
    pub fn plan_v1_migration(
        &self,
        source: &Blueprint,
    ) -> Result<WorkspaceMigrationPlan, AirpError> {
        let source_bytes = serde_json::to_vec(source)?;
        if source_bytes.len() > WORKSPACE_MAX_DOCUMENT_BYTES {
            return Err(AirpError::BadRequest(
                "legacy Blueprint exceeds workspace migration byte limit".to_string(),
            ));
        }
        let migrated = migrate_v1_blueprint(source)
            .map_err(|error| AirpError::BadRequest(format!("workspace migration: {error}")))?;
        let dropped_props = migrated
            .widgets
            .iter()
            .filter(|widget| widget.props.is_some())
            .count();
        let layout = workspace_layout_from_blueprint(&migrated)?;
        let candidate = WorkspaceDocumentV1 {
            schema: WORKSPACE_SCHEMA_MAJOR,
            id: WORKSPACE_ID.to_string(),
            revision: SurfaceRevision::new(0),
            updated_at: "dry-run".to_string(),
            layout,
        };
        validate_workspace_document(&candidate)
            .map_err(|error| AirpError::BadRequest(format!("workspace migration: {error}")))?;
        let mut warnings = vec![
            "dry-run only; no workspace files or revision pointers were changed".to_string(),
            "legacy theme, state scopes and capabilities are not workspace data".to_string(),
        ];
        if dropped_props > 0 {
            warnings.push(format!(
                "dropped runtime/static props from {dropped_props} widget declarations"
            ));
        }
        Ok(WorkspaceMigrationPlan {
            source: "blueprint-v1".to_string(),
            source_sha256: sha256_hex(&source_bytes),
            candidate,
            warnings,
            writes_performed: false,
        })
    }

    fn asset_dir(&self) -> PathBuf {
        self.effective_root
            .join("ui")
            .join("workspaces")
            .join(WORKSPACE_ID)
    }

    fn read_under_lock(&self) -> Result<WorkspaceDocumentV1, AirpError> {
        let Some(revision) = read_current_revision(&self.asset_dir())? else {
            return Ok(default_document());
        };
        let (_, document, _) = self.load_revision(revision)?;
        Ok(document)
    }

    fn commit_layout_under_lock(
        &self,
        expected_revision: u64,
        layout: WorkspaceLayoutV1,
        source_kind: &str,
        source_hash: Option<String>,
        source_filename: Option<String>,
    ) -> Result<WorkspaceDocumentV1, AirpError> {
        let current = read_current_revision(&self.asset_dir())?.unwrap_or(0);
        if current != expected_revision {
            return Err(workspace_conflict(expected_revision, current));
        }
        // Never overwrite an unreadable or future-major current asset. Users
        // must retain raw export access without this Engine silently resetting
        // a workspace it does not understand.
        if current > 0 {
            self.load_revision(current)?;
        }
        let revision = next_content_revision(&self.asset_dir())?;
        let document = WorkspaceDocumentV1 {
            schema: WORKSPACE_SCHEMA_MAJOR,
            id: WORKSPACE_ID.to_string(),
            revision: revision.into(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            layout,
        };
        validate_workspace_document(&document)
            .map_err(|error| AirpError::BadRequest(format!("invalid workspace: {error}")))?;
        let bytes = serde_json::to_vec_pretty(&document)?;
        let staged = StagedRevision {
            content_revision: revision,
            asset_kind: AssetKind::Workspace,
            asset_id: WORKSPACE_ID.to_string(),
            created_at: document.updated_at.clone(),
            source: AssetSource {
                source_kind: source_kind.to_string(),
                source_hash,
                source_filename,
                converter_version: None,
                imported_at: None,
                parent_revision: (current > 0).then_some(current),
            },
            files: vec![(WORKSPACE_FILE.to_string(), bytes)],
        };
        commit_revision(&staged, &CommitOptions::new(self.asset_dir()))?;
        Ok(document)
    }

    fn history_under_lock(&self, limit: usize) -> Result<Vec<WorkspaceHistoryEntry>, AirpError> {
        let Some(mut revision) = read_current_revision(&self.asset_dir())? else {
            return Ok(Vec::new());
        };
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        while entries.len() < limit {
            if !seen.insert(revision) {
                return Err(AirpError::Internal(
                    "workspace revision lineage contains a cycle".to_string(),
                ));
            }
            let (manifest, document, _) = self.load_revision(revision)?;
            entries.push(WorkspaceHistoryEntry {
                revision: revision.into(),
                updated_at: document.updated_at,
                source_kind: manifest.source.source_kind.clone(),
                parent_revision: manifest.source.parent_revision.map(Into::into),
            });
            match manifest.source.parent_revision {
                Some(parent) if parent < revision => revision = parent,
                Some(_) => {
                    return Err(AirpError::Internal(
                        "workspace revision lineage does not move backward".to_string(),
                    ))
                }
                None => break,
            }
        }
        Ok(entries)
    }

    /// Traverse the complete committed parent chain. Unlike the public
    /// history response, rollback ancestry is not capped or paginated.
    fn lineage_contains_under_lock(&self, target_revision: u64) -> Result<bool, AirpError> {
        let Some(mut revision) = read_current_revision(&self.asset_dir())? else {
            return Ok(false);
        };
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(revision) {
                return Err(AirpError::Internal(
                    "workspace revision lineage contains a cycle".to_string(),
                ));
            }
            let (manifest, _, _) = self.load_revision(revision)?;
            if revision == target_revision {
                return Ok(true);
            }
            match manifest.source.parent_revision {
                Some(parent) if parent < revision => revision = parent,
                Some(_) => {
                    return Err(AirpError::Internal(
                        "workspace revision lineage does not move backward".to_string(),
                    ))
                }
                None => return Ok(false),
            }
        }
    }

    fn load_revision(
        &self,
        revision: u64,
    ) -> Result<(RevisionManifest, WorkspaceDocumentV1, Vec<u8>), AirpError> {
        let (manifest, bytes) = self.load_verified_bytes(revision)?;
        let document: WorkspaceDocumentV1 = serde_json::from_slice(&bytes).map_err(|error| {
            AirpError::Internal(format!(
                "workspace revision {revision} JSON is invalid: {error}"
            ))
        })?;
        validate_workspace_document(&document).map_err(|error| {
            AirpError::Internal(format!("workspace revision {revision} is invalid: {error}"))
        })?;
        if document.id != WORKSPACE_ID || document.revision.value() != revision {
            return Err(AirpError::Internal(format!(
                "workspace revision {revision} payload identity does not match its directory"
            )));
        }
        Ok((manifest, document, bytes))
    }

    fn load_verified_bytes(&self, revision: u64) -> Result<(RevisionManifest, Vec<u8>), AirpError> {
        let revision_dir = self
            .asset_dir()
            .join("revisions")
            .join(revision.to_string());
        if !revision_dir.is_dir() {
            return Err(AirpError::Internal(format!(
                "workspace current revision directory {revision} is missing"
            )));
        }
        let manifest =
            RevisionManifest::from_json_bytes(&fs::read(revision_dir.join("manifest.json"))?)?;
        manifest.verify_against_disk(&revision_dir)?;
        if manifest.asset_kind != AssetKind::Workspace
            || manifest.asset_id != WORKSPACE_ID
            || manifest.content_revision != revision
        {
            return Err(AirpError::Internal(format!(
                "workspace revision {revision} manifest identity mismatch"
            )));
        }
        if manifest.files.len() != 1 || manifest.files[0].path != WORKSPACE_FILE {
            return Err(AirpError::Internal(format!(
                "workspace revision {revision} approved file set is invalid"
            )));
        }
        let bytes = fs::read(revision_dir.join(WORKSPACE_FILE))?;
        if bytes.len() > WORKSPACE_MAX_DOCUMENT_BYTES {
            return Err(AirpError::Internal(format!(
                "workspace revision {revision} exceeds the byte limit"
            )));
        }
        Ok((manifest, bytes))
    }
}

fn workspace_guard() -> Result<std::sync::MutexGuard<'static, ()>, AirpError> {
    WORKSPACE_LOCK
        .lock()
        .map_err(|error| AirpError::Internal(format!("WORKSPACE_LOCK poisoned: {error}")))
}

fn workspace_conflict(expected: u64, current: u64) -> AirpError {
    AirpError::Conflict(format!(
        "workspace revision conflict: expected {expected}, current {current}"
    ))
}

fn export_from_bytes(bytes: &[u8]) -> Result<WorkspaceExport, AirpError> {
    if bytes.len() > WORKSPACE_MAX_DOCUMENT_BYTES {
        return Err(AirpError::Internal(
            "workspace export exceeds the byte limit".to_string(),
        ));
    }
    let document: serde_json::Value = serde_json::from_slice(bytes)?;
    let schema = document
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| AirpError::Internal("workspace export has no valid schema".to_string()))?;
    Ok(WorkspaceExport {
        schema,
        sha256: sha256_hex(bytes),
        raw_json: String::from_utf8(bytes.to_vec()).map_err(|error| {
            AirpError::Internal(format!("workspace export is not UTF-8 JSON: {error}"))
        })?,
        document,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn default_document() -> WorkspaceDocumentV1 {
    WorkspaceDocumentV1 {
        schema: WORKSPACE_SCHEMA_MAJOR,
        id: WORKSPACE_ID.to_string(),
        revision: SurfaceRevision::new(0),
        updated_at: "uncommitted".to_string(),
        layout: WorkspaceLayoutV1 {
            version: WORKSPACE_SCHEMA_MAJOR,
            root: WorkspaceNodeV1::Split {
                id: "workspace-root".to_string(),
                orientation: airp_state_protocol::SplitOrientation::Horizontal,
                ratio_basis_points: 6_500,
                children: vec![
                    WorkspaceNodeV1::Tabs {
                        id: "workspace-primary".to_string(),
                        active: "chat-node".to_string(),
                        children: vec![
                            WorkspaceNodeV1::Widget {
                                id: "chat-node".to_string(),
                                instance_id: "chat".to_string(),
                            },
                            WorkspaceNodeV1::Widget {
                                id: "memory-node".to_string(),
                                instance_id: "memory".to_string(),
                            },
                        ],
                    },
                    WorkspaceNodeV1::Stack {
                        id: "workspace-context".to_string(),
                        children: vec![
                            WorkspaceNodeV1::Widget {
                                id: "character-state-node".to_string(),
                                instance_id: "character-state".to_string(),
                            },
                            WorkspaceNodeV1::Widget {
                                id: "activity-node".to_string(),
                                instance_id: "activity".to_string(),
                            },
                        ],
                    },
                ],
            },
            widgets: vec![
                workspace_widget("chat", "core.chat"),
                workspace_widget("memory", "core.memory"),
                workspace_widget("character-state", "core.character-state"),
                workspace_widget("activity", "core.activity"),
            ],
        },
    }
}

fn workspace_widget(id: &str, widget_type: &str) -> WorkspaceWidgetV1 {
    WorkspaceWidgetV1 {
        id: id.to_string(),
        widget_type: widget_type.to_string(),
    }
}

fn workspace_layout_from_blueprint(
    blueprint: &BlueprintV2,
) -> Result<WorkspaceLayoutV1, AirpError> {
    Ok(WorkspaceLayoutV1 {
        version: WORKSPACE_SCHEMA_MAJOR,
        root: workspace_node_from_surface(&blueprint.root)?,
        widgets: blueprint
            .widgets
            .iter()
            .map(|widget| WorkspaceWidgetV1 {
                id: widget.id.clone(),
                widget_type: widget.widget_type.clone(),
            })
            .collect(),
    })
}

fn workspace_node_from_surface(node: &LayoutNodeV2) -> Result<WorkspaceNodeV1, AirpError> {
    Ok(match node {
        LayoutNodeV2::Split {
            id,
            orientation,
            children,
        } => WorkspaceNodeV1::Split {
            id: id.clone(),
            orientation: *orientation,
            ratio_basis_points: 5_000,
            children: children
                .iter()
                .map(workspace_node_from_surface)
                .collect::<Result<Vec<_>, _>>()?,
        },
        LayoutNodeV2::Tabs {
            id,
            active,
            children,
        } => WorkspaceNodeV1::Tabs {
            id: id.clone(),
            active: active.clone(),
            children: children
                .iter()
                .map(workspace_node_from_surface)
                .collect::<Result<Vec<_>, _>>()?,
        },
        LayoutNodeV2::Stack { id, children } => WorkspaceNodeV1::Stack {
            id: id.clone(),
            children: children
                .iter()
                .map(workspace_node_from_surface)
                .collect::<Result<Vec<_>, _>>()?,
        },
        LayoutNodeV2::Widget { id, instance_id } => WorkspaceNodeV1::Widget {
            id: id.clone(),
            instance_id: instance_id.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_uncommitted_and_first_save_uses_cas() {
        let root = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(root.path());
        let initial = service.read().unwrap();
        assert_eq!(initial.revision, SurfaceRevision::new(0));
        let saved = service.save(0, initial.layout).unwrap();
        assert_eq!(saved.revision, SurfaceRevision::new(1));
        assert!(matches!(
            service.save(0, saved.layout.clone()),
            Err(AirpError::Conflict(_))
        ));
        assert_eq!(service.read().unwrap().revision, SurfaceRevision::new(1));
    }

    #[test]
    fn history_skips_orphan_revision_and_rollback_moves_forward() {
        let root = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(root.path());
        let first = service.read().unwrap();
        let first = service.save(0, first.layout).unwrap();

        let orphan = StagedRevision {
            content_revision: 2,
            asset_kind: AssetKind::Workspace,
            asset_id: WORKSPACE_ID.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source: AssetSource {
                source_kind: "orphan-test".to_string(),
                parent_revision: Some(1),
                ..AssetSource::default()
            },
            files: vec![(
                WORKSPACE_FILE.to_string(),
                serde_json::to_vec_pretty(&first).unwrap(),
            )],
        };
        commit_revision(&orphan, &CommitOptions::new(service.asset_dir())).unwrap();
        fs::write(service.asset_dir().join("current_revision"), "1").unwrap();

        let second = service.save(1, first.layout.clone()).unwrap();
        assert_eq!(second.revision, SurfaceRevision::new(3));
        assert_eq!(
            service
                .history(10)
                .unwrap()
                .iter()
                .map(|entry| entry.revision.value())
                .collect::<Vec<_>>(),
            vec![3, 1]
        );
        let rolled = service.rollback(3, 1).unwrap();
        assert_eq!(rolled.revision, SurfaceRevision::new(4));
        assert_eq!(service.read().unwrap().revision, SurfaceRevision::new(4));
    }

    #[test]
    fn export_contains_no_surface_props_and_detects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(root.path());
        let initial = service.read().unwrap();
        service.save(0, initial.layout).unwrap();
        let export = service.export().unwrap();
        let approved =
            fs::read(service.asset_dir().join("revisions/1").join(WORKSPACE_FILE)).unwrap();
        assert_eq!(export.raw_json.as_bytes(), approved);
        assert_eq!(export.sha256, sha256_hex(export.raw_json.as_bytes()));
        let text = serde_json::to_string(&export.document).unwrap();
        assert!(!text.contains("props"));
        assert!(!text.contains("sessionStorage"));

        let path = service.asset_dir().join("revisions/1").join(WORKSPACE_FILE);
        fs::write(path, b"{}").unwrap();
        assert!(matches!(service.read(), Err(AirpError::Internal(_))));
    }

    #[test]
    fn v1_migration_is_dry_run_and_drops_props() {
        let root = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(root.path());
        let source = Blueprint {
            version: "legacy".to_string(),
            profile: None,
            theme: None,
            layout: airp_state_protocol::Layout {
                kind: airp_state_protocol::LayoutKind::Tabs,
                areas: vec![airp_state_protocol::Area {
                    id: "main".to_string(),
                    widgets: vec!["chat".to_string()],
                    props: None,
                }],
            },
            widgets: vec![airp_state_protocol::WidgetInstance {
                id: "chat".to_string(),
                kind: "core.chat".to_string(),
                props: Some(serde_json::json!({"content": "must-drop"})),
                state: Some("session".to_string()),
                capabilities: None,
            }],
        };
        let plan = service.plan_v1_migration(&source).unwrap();
        assert!(!plan.writes_performed);
        assert_eq!(plan.candidate.revision, SurfaceRevision::new(0));
        assert!(!service.asset_dir().exists());
        assert!(!serde_json::to_string(&plan.candidate)
            .unwrap()
            .contains("must-drop"));
    }

    #[test]
    fn concurrent_same_revision_cas_allows_exactly_one_commit() {
        let root = tempfile::tempdir().unwrap();
        let service = std::sync::Arc::new(WorkspaceService::new(root.path()));
        let layout = service.read().unwrap().layout;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let service = service.clone();
            let layout = layout.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                service.save(0, layout)
            }));
        }
        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AirpError::Conflict(_))))
                .count(),
            1
        );
    }

    #[test]
    fn unknown_major_can_export_but_cannot_be_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(root.path());
        let mut raw = serde_json::to_value(default_document()).unwrap();
        raw["schema"] = serde_json::json!(WORKSPACE_SCHEMA_MAJOR + 1);
        let bytes = serde_json::to_vec_pretty(&raw).unwrap();
        let staged = StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Workspace,
            asset_id: WORKSPACE_ID.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source: AssetSource {
                source_kind: "future-version-test".to_string(),
                ..AssetSource::default()
            },
            files: vec![(WORKSPACE_FILE.to_string(), bytes.clone())],
        };
        commit_revision(&staged, &CommitOptions::new(service.asset_dir())).unwrap();

        let export = service.export().unwrap();
        assert_eq!(export.schema, WORKSPACE_SCHEMA_MAJOR + 1);
        assert_eq!(export.sha256, sha256_hex(&bytes));
        assert_eq!(export.raw_json.as_bytes(), bytes);
        assert!(service.read().is_err());
        assert!(service.save(1, default_document().layout).is_err());
        assert_eq!(
            fs::read_to_string(service.asset_dir().join("current_revision")).unwrap(),
            "1"
        );
    }

    #[test]
    fn rollback_can_reach_ancestor_older_than_history_response_cap() {
        let root = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(root.path());
        let layout = service.read().unwrap().layout;
        let mut current = 0;
        for _ in 0..=MAX_HISTORY_ENTRIES {
            current = service
                .save(current, layout.clone())
                .unwrap()
                .revision
                .value();
        }
        assert_eq!(
            service.history(usize::MAX).unwrap().len(),
            MAX_HISTORY_ENTRIES
        );
        let rolled = service.rollback(current, 1).unwrap();
        assert!(rolled.revision.value() > current);
    }
}
