//! Explicit single-file provider-key persistence for the portable WebUI.

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SECRET_FILE_NAME: &str = "secrets.json";
const SECRET_FILE_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SecretFile {
    version: u32,
    provider_api_key: String,
}

pub(crate) fn persistence_enabled() -> bool {
    std::env::var("AIRP_PERSIST_PROVIDER_KEY")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(crate) fn provider_key_path(data_root: &Path) -> PathBuf {
    data_root.join(SECRET_FILE_NAME)
}

pub(crate) fn load_provider_key(data_root: &Path) -> Result<Option<String>, AirpError> {
    if !persistence_enabled() {
        return Ok(None);
    }
    let path = provider_key_path(data_root);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| AirpError::Internal(format!("cannot read secrets.json: {error}")))?;
    let secret: SecretFile = serde_json::from_slice(&bytes)
        .map_err(|error| AirpError::Internal(format!("cannot parse secrets.json: {error}")))?;
    if secret.version != SECRET_FILE_VERSION {
        return Err(AirpError::Internal(format!(
            "unsupported secrets.json version {}",
            secret.version
        )));
    }
    if secret.provider_api_key.is_empty() {
        return Err(AirpError::Internal(
            "secrets.json contains an empty provider_api_key".to_string(),
        ));
    }
    Ok(Some(secret.provider_api_key))
}

pub(crate) fn persist_provider_key(data_root: &Path, key: &str) -> Result<(), AirpError> {
    if !persistence_enabled() || key.is_empty() {
        return Ok(());
    }
    let file = SecretFile {
        version: SECRET_FILE_VERSION,
        provider_api_key: key.to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    crate::data_dir::replace_file(&provider_key_path(data_root), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_file_has_explicit_version_and_single_key() {
        let bytes = serde_json::to_vec(&SecretFile {
            version: SECRET_FILE_VERSION,
            provider_api_key: "smoke-provider-key".to_string(),
        })
        .unwrap();
        let decoded: SecretFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.provider_api_key, "smoke-provider-key");
    }

    #[test]
    fn secret_file_rejects_unknown_fields() {
        let value = br#"{"version":1,"provider_api_key":"key","unexpected":true}"#;
        assert!(serde_json::from_slice::<SecretFile>(value).is_err());
    }
}
