//! Durable, redacted control-plane receipts for the Activity widget.
//!
//! This store is deliberately disconnected from prompt assembly. It accepts
//! only closed enums and generated identifiers: no RP text, prompt content,
//! tool parameters/output, provider endpoints, or arbitrary error messages.

use std::{fs, path::Path, time::SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::AirpError;

const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "ui-activity.json";
const MAX_RECEIPTS: usize = 32;
const MAX_FILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivitySource {
    Chat,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityFailureCode {
    UpstreamError,
    Timeout,
    FinalizationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivityReceipt {
    schema_version: u32,
    activity_id: String,
    occurred_at_unix_ms: u64,
    source: ActivitySource,
    kind: ActivityKind,
    severity: ActivitySeverity,
    code: ActivityFailureCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActivityKind {
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActivitySeverity {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivityWindow {
    schema_version: u32,
    items: Vec<ActivityReceipt>,
}

pub(crate) fn record_failure(
    session_dir: &Path,
    source: ActivitySource,
    code: ActivityFailureCode,
    generation_id: Option<&str>,
) -> Result<(), AirpError> {
    crate::memory::with_memory_mutation(session_dir, || {
        let mut window = read_window(session_dir)?;
        window.items.push(ActivityReceipt {
            schema_version: SCHEMA_VERSION,
            activity_id: crate::ulid::new_id(),
            occurred_at_unix_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            source,
            kind: ActivityKind::Failed,
            severity: ActivitySeverity::Error,
            code,
            generation_id: generation_id.map(ToOwned::to_owned),
        });
        if window.items.len() > MAX_RECEIPTS {
            window.items.drain(..window.items.len() - MAX_RECEIPTS);
        }
        let bytes = serde_json::to_vec_pretty(&window)?;
        crate::data_dir::replace_file(&activity_path(session_dir), &bytes)
    })
}

pub(crate) fn read_window(session_dir: &Path) -> Result<ActivityWindow, AirpError> {
    let path = activity_path(session_dir);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ActivityWindow {
                schema_version: SCHEMA_VERSION,
                items: Vec::new(),
            })
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Err(AirpError::Config(
            "UI activity receipt file exceeds its size limit".to_string(),
        ));
    }
    let window: ActivityWindow = serde_json::from_slice(&fs::read(path)?)?;
    if window.schema_version != SCHEMA_VERSION
        || window.items.len() > MAX_RECEIPTS
        || window
            .items
            .iter()
            .any(|item| item.schema_version != SCHEMA_VERSION)
    {
        return Err(AirpError::Config(
            "UI activity receipt file has an unsupported shape".to_string(),
        ));
    }
    Ok(window)
}

fn activity_path(session_dir: &Path) -> std::path::PathBuf {
    session_dir.join(FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_survive_reload_and_are_bounded_without_content_fields() {
        let tmp = tempfile::tempdir().unwrap();
        for index in 0..40 {
            record_failure(
                tmp.path(),
                ActivitySource::Chat,
                ActivityFailureCode::UpstreamError,
                Some(&format!("generation-{index}")),
            )
            .unwrap();
        }

        let reloaded = read_window(tmp.path()).unwrap();
        assert_eq!(reloaded.items.len(), MAX_RECEIPTS);
        let serialized = serde_json::to_string(&reloaded).unwrap();
        assert!(!serialized.contains("message"));
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("params"));
        assert!(!serialized.contains("output"));
        assert!(serialized.contains("generation-39"));
        assert!(!serialized.contains("generation-0\""));
    }

    #[test]
    fn malformed_or_oversized_windows_fail_closed_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let path = activity_path(tmp.path());
        fs::write(&path, b"not-json").unwrap();
        assert!(read_window(tmp.path()).is_err());
        assert!(record_failure(
            tmp.path(),
            ActivitySource::Agent,
            ActivityFailureCode::FinalizationFailed,
            None,
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not-json");

        fs::write(&path, vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(read_window(tmp.path()).is_err());
    }

    #[test]
    fn concurrent_failures_do_not_drop_receipts() {
        let tmp = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for index in 0..16 {
                let session_dir = tmp.path().to_path_buf();
                scope.spawn(move || {
                    record_failure(
                        &session_dir,
                        ActivitySource::Chat,
                        ActivityFailureCode::UpstreamError,
                        Some(&format!("generation-{index}")),
                    )
                    .unwrap();
                });
            }
        });

        let window = read_window(tmp.path()).unwrap();
        assert_eq!(window.items.len(), 16);
    }
}
