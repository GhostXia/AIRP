//! Soul-Drift 动态人格：Base + drift 双层 overlay。
//!
//! 存储路径：`data/characters/{id}/soul_drift.md`（每角色一份）
//! 格式：markdown 条目列表（`- ` 开头），与 resident memory 同构
//! 容量上限：~1500 字符（可配置）；超限触发 LLM 合并压缩

use crate::error::AirpError;
use crate::revision::atomic::{
    commit_revision, next_content_revision, read_current_revision, CommitOptions, StagedRevision,
};
use crate::revision::manifest::{AssetKind, AssetSource, RevisionManifest};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

/// 默认容量上限（字符数）。
pub const SOUL_DRIFT_DEFAULT_CAP: usize = 1500;

/// 每角色串行化锁：防止并发 read-modify-write 互相覆盖（审计修复）。
///
/// 审计再修复：用 Weak 引用持有锁，获取时清理已无持有者的 stale 条目，
/// 防止长生命周期进程中注册表无界增长。
static DRIFT_LOCKS: Lazy<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 获取角色的串行化锁。
fn drift_lock(character_id: &str) -> Arc<Mutex<()>> {
    let mut locks = DRIFT_LOCKS.lock().expect("drift locks poisoned");
    // 清理已无强引用的 stale 条目，保证注册表有界。
    locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(weak) = locks.get(character_id) {
        if let Some(strong) = weak.upgrade() {
            return strong;
        }
    }
    let strong = Arc::new(Mutex::new(()));
    locks.insert(character_id.to_string(), Arc::downgrade(&strong));
    strong
}

/// Soul-Drift 配置。
#[derive(Debug, Clone)]
pub struct SoulDriftConfig {
    /// 容量上限（字符数）。超限触发压缩。
    pub capacity_chars: usize,
}

impl Default for SoulDriftConfig {
    fn default() -> Self {
        Self {
            capacity_chars: SOUL_DRIFT_DEFAULT_CAP,
        }
    }
}

/// 返回角色的 soul_drift.md 路径。
fn drift_path(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("soul_drift.md")
}

fn drift_asset_dir(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("soul_drift")
}

fn load_revision_content(
    asset_dir: &Path,
    character_id: &str,
    revision: u64,
) -> Result<String, AirpError> {
    let revision_dir = asset_dir.join("revisions").join(revision.to_string());
    let manifest =
        RevisionManifest::from_json_bytes(&fs::read(revision_dir.join("manifest.json"))?)?;
    if manifest.content_revision != revision
        || manifest.asset_kind != AssetKind::SoulDrift
        || manifest.asset_id != character_id
    {
        return Err(AirpError::Internal(format!(
            "Soul-Drift revision {revision} manifest identity mismatch"
        )));
    }
    manifest.verify_against_disk(&revision_dir)?;
    Ok(fs::read_to_string(revision_dir.join("soul_drift.md"))?)
}

fn read_soul_drift_with_revision_unlocked(
    data_root: &Path,
    character_id: &str,
) -> Result<(String, Option<u64>), AirpError> {
    let asset_dir = drift_asset_dir(data_root, character_id);
    if let Some(revision) = read_current_revision(&asset_dir)? {
        return Ok((
            load_revision_content(&asset_dir, character_id, revision)?,
            Some(revision),
        ));
    }

    let path = drift_path(data_root, character_id);
    match fs::read_to_string(&path) {
        Ok(content) => Ok((content, None)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), None)),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 读取 soul drift 内容。文件不存在返回空字符串。
pub fn read_soul_drift(data_root: &Path, character_id: &str) -> Result<String, AirpError> {
    read_soul_drift_with_revision(data_root, character_id).map(|(content, _)| content)
}

/// Read Soul-Drift content together with its current immutable revision.
pub fn read_soul_drift_with_revision(
    data_root: &Path,
    character_id: &str,
) -> Result<(String, Option<u64>), AirpError> {
    read_soul_drift_with_revision_unlocked(data_root, character_id)
}

/// 写入 soul drift（覆盖）。
///
/// 审计修复：写入前强制容量上限，超限截断到最近完整行，防止超量内容被
/// 整体注入后续 system prompt。
///
/// 审计再修复（CodeRabbit 22:26）：直接写入也持有每角色锁，与 append
/// 互斥，防止 write-vs-append 竞态。
pub fn write_soul_drift(
    data_root: &Path,
    character_id: &str,
    content: &str,
) -> Result<u64, AirpError> {
    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    ensure_legacy_revision_unlocked(data_root, character_id)?;
    let config = SoulDriftConfig::default();
    let content = enforce_capacity(content, config.capacity_chars);
    let parent_revision = read_current_revision(&drift_asset_dir(data_root, character_id))?;
    commit_soul_drift_unlocked(
        data_root,
        character_id,
        &content,
        "manual_update",
        parent_revision,
    )
}

fn ensure_legacy_revision_unlocked(data_root: &Path, character_id: &str) -> Result<(), AirpError> {
    let asset_dir = drift_asset_dir(data_root, character_id);
    if read_current_revision(&asset_dir)?.is_some() {
        return Ok(());
    }
    let path = drift_path(data_root, character_id);
    if !path.exists() {
        return Ok(());
    }
    let legacy = fs::read_to_string(path)?;
    commit_soul_drift_unlocked(data_root, character_id, &legacy, "legacy_migration", None)?;
    Ok(())
}

fn commit_soul_drift_unlocked(
    data_root: &Path,
    character_id: &str,
    content: &str,
    source_kind: &str,
    parent_revision: Option<u64>,
) -> Result<u64, AirpError> {
    let asset_dir = drift_asset_dir(data_root, character_id);
    let revision = next_content_revision(&asset_dir)?;
    let staged = StagedRevision {
        content_revision: revision,
        asset_kind: AssetKind::SoulDrift,
        asset_id: character_id.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        source: AssetSource {
            source_kind: source_kind.to_string(),
            parent_revision,
            ..Default::default()
        },
        files: vec![("soul_drift.md".to_string(), content.as_bytes().to_vec())],
    };
    commit_revision(&staged, &CommitOptions::new(asset_dir))?;

    let path = drift_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::data_dir::replace_file(&path, content.as_bytes())?;
    Ok(revision)
}

/// 截断到容量上限，尽量保留完整行。
///
/// 审计再修复：若首行单独就超过容量（逐行截断会得到空串），回退为按
/// Unicode 字符边界截断，保证不会把超量单行输入清空。
fn enforce_capacity(content: &str, capacity: usize) -> String {
    if content.chars().count() <= capacity {
        return content.to_string();
    }
    // Soul-Drift is append-oriented: retain the newest complete lines.
    let mut kept = Vec::new();
    let mut count = 0;
    for line in content.lines().rev() {
        let line_len = line.chars().count() + 1; // +1 for newline
        if count + line_len > capacity {
            break;
        }
        kept.push(line);
        count += line_len;
    }
    if kept.is_empty() {
        let mut chars: Vec<char> = content.chars().rev().take(capacity).collect();
        chars.reverse();
        return chars.into_iter().collect();
    }
    kept.reverse();
    let mut result = kept.join("\n");
    result.push('\n');
    result
}

fn append_content(mut existing: String, patch: &str) -> String {
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(patch);
    existing
}

/// 追加内容到 soul drift。
///
/// 审计修复：整个 read-modify-write 过程持有每角色锁，防止并发丢失更新。
///
/// 审计再修复（CodeRabbit 22:26）：原实现 `let _guard = drift_lock(...)` 只持有
/// Arc 而未调用 `.lock()`，锁从未生效。现在真正获取 MutexGuard，且内部
/// 调用无锁版写入避免重入死锁。
pub fn append_soul_drift(
    data_root: &Path,
    character_id: &str,
    content: &str,
) -> Result<u64, AirpError> {
    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    ensure_legacy_revision_unlocked(data_root, character_id)?;
    let (mut existing, parent_revision) =
        read_soul_drift_with_revision_unlocked(data_root, character_id)?;
    existing = append_content(existing, content);
    let config = SoulDriftConfig::default();
    let existing = enforce_capacity(&existing, config.capacity_chars);
    commit_soul_drift_unlocked(
        data_root,
        character_id,
        &existing,
        "automated_append",
        parent_revision,
    )
}

/// Append a drift patch, invoking the LLM before persistence when the candidate
/// exceeds capacity. A bounded newest-content fallback is committed if the LLM
/// fails to produce a smaller result.
pub async fn append_soul_drift_with_compression(
    client: &reqwest::Client,
    provider_config: Arc<crate::adapter::ProviderConfig>,
    gen_params: crate::adapter::GenerationParams,
    data_root: &Path,
    character_id: &str,
    patch: &str,
) -> Result<u64, AirpError> {
    let config = SoulDriftConfig::default();
    let (base, base_revision) = {
        let lock = drift_lock(character_id);
        let _guard = lock.lock().expect("drift lock poisoned");
        ensure_legacy_revision_unlocked(data_root, character_id)?;
        read_soul_drift_with_revision_unlocked(data_root, character_id)?
    };
    let candidate = append_content(base.clone(), patch);
    if candidate.chars().count() <= config.capacity_chars {
        return append_soul_drift(data_root, character_id, patch);
    }

    let compressed = crate::memory::compress_resident_memory(
        client,
        provider_config,
        gen_params,
        &candidate,
        config.capacity_chars,
    )
    .await?;
    let llm_result_is_smaller =
        !compressed.trim().is_empty() && compressed.chars().count() < candidate.chars().count();
    let selected = if llm_result_is_smaller {
        &compressed
    } else {
        &candidate
    };
    let bounded = enforce_capacity(selected, config.capacity_chars);

    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    ensure_legacy_revision_unlocked(data_root, character_id)?;
    let (current, current_revision) =
        read_soul_drift_with_revision_unlocked(data_root, character_id)?;
    if current != base || current_revision != base_revision {
        let fresh_candidate = append_content(current, patch);
        let fallback = enforce_capacity(&fresh_candidate, config.capacity_chars);
        return commit_soul_drift_unlocked(
            data_root,
            character_id,
            &fallback,
            "automated_append_concurrent_fallback",
            current_revision,
        );
    }

    commit_soul_drift_unlocked(
        data_root,
        character_id,
        &bounded,
        if llm_result_is_smaller {
            "llm_compression"
        } else {
            "automated_append_fallback"
        },
        base_revision,
    )
}

/// Restore an immutable revision by committing its content as a new revision.
pub fn rollback_soul_drift(
    data_root: &Path,
    character_id: &str,
    target_revision: u64,
) -> Result<u64, AirpError> {
    if target_revision == 0 {
        return Err(AirpError::BadRequest(
            "Soul-Drift revision must be >= 1".to_string(),
        ));
    }
    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    ensure_legacy_revision_unlocked(data_root, character_id)?;
    let asset_dir = drift_asset_dir(data_root, character_id);
    let content =
        load_revision_content(&asset_dir, character_id, target_revision).map_err(|e| match e {
            AirpError::Io(ref io) if io.kind() == std::io::ErrorKind::NotFound => {
                AirpError::BadRequest(format!(
                    "Soul-Drift revision {target_revision} does not exist"
                ))
            }
            other => other,
        })?;
    commit_soul_drift_unlocked(
        data_root,
        character_id,
        &content,
        "rollback",
        Some(target_revision),
    )
}

/// 把 soul_drift.md 注入到 System Prompt 的 `[Soul Drift]` 段。
///
/// 注入位置：card_details 之后、lorebook 之前。
/// Frozen snapshot 语义：本轮写入，下轮 prepare 才注入。
pub fn inject_soul_drift(data_root: &Path, character_id: &str, prompt: &mut String) {
    let Ok(content) = read_soul_drift(data_root, character_id) else {
        return;
    };
    if content.trim().is_empty() {
        return;
    }
    prompt.push_str("\n[Soul Drift]\n");
    prompt.push_str(&content);
    if !content.ends_with('\n') {
        prompt.push('\n');
    }
}

/// #290 F-3：Soul-Drift 超容量时调用 LLM 合并压缩。
///
/// 复用 `memory::compress_resident_memory` 的压缩 prompt。压缩结果必须真的
/// 变小才落盘，否则保留原内容（enforce_capacity 已在写入时截断兜底）。
///
/// CodeRabbit #9：LLM 压缩是耗时 async 操作，读取与写入之间可能有并发
/// `append_soul_drift` 修改了文件。写入前持有 per-character 锁并重新读取
/// 验证内容未变（CAS），若已变则放弃本次压缩（避免陈旧压缩结果覆盖
/// 更新的 drift）。
pub async fn compress_soul_drift_if_needed(
    client: &reqwest::Client,
    provider_config: Arc<crate::adapter::ProviderConfig>,
    gen_params: crate::adapter::GenerationParams,
    data_root: &Path,
    character_id: &str,
) -> Result<bool, AirpError> {
    let config = SoulDriftConfig::default();
    let content = read_soul_drift(data_root, character_id)?;
    if content.chars().count() <= config.capacity_chars {
        return Ok(false);
    }
    let compressed = crate::memory::compress_resident_memory(
        client,
        provider_config,
        gen_params,
        &content,
        config.capacity_chars,
    )
    .await?;
    if compressed.is_empty() || compressed.chars().count() >= content.chars().count() {
        return Ok(false);
    }
    // CAS 验证：持有锁后重新读取，确保内容未被并发修改。
    let lock = drift_lock(character_id);
    let _guard = lock.lock().expect("drift lock poisoned");
    ensure_legacy_revision_unlocked(data_root, character_id)?;
    let (current, parent_revision) =
        read_soul_drift_with_revision_unlocked(data_root, character_id)?;
    if current != content {
        tracing::info!("soul drift 在压缩期间被并发修改，跳过写入");
        return Ok(false);
    }
    commit_soul_drift_unlocked(
        data_root,
        character_id,
        &compressed,
        "llm_compression",
        parent_revision,
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_nonexistent_returns_empty() {
        let tmp = tempdir().unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_and_read() {
        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", "- 语气更温柔").unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.contains("语气更温柔"));
    }

    #[test]
    fn test_append() {
        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", "- 第一条").unwrap();
        append_soul_drift(tmp.path(), "hero", "- 第二条").unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.contains("第一条"));
        assert!(content.contains("第二条"));
    }

    #[test]
    fn test_inject_soul_drift() {
        let tmp = tempdir().unwrap();
        let mut prompt = String::from("Base prompt.");

        inject_soul_drift(tmp.path(), "hero", &mut prompt);
        assert_eq!(prompt, "Base prompt.");

        write_soul_drift(tmp.path(), "hero", "- 更活泼").unwrap();
        inject_soul_drift(tmp.path(), "hero", &mut prompt);
        assert!(prompt.contains("[Soul Drift]"));
        assert!(prompt.contains("更活泼"));
    }

    #[test]
    fn test_write_enforces_capacity() {
        let tmp = tempdir().unwrap();
        // 默认容量 1500，写入超长内容应被截断。
        let long_content = "- 条目\n".repeat(1000);
        write_soul_drift(tmp.path(), "hero", &long_content).unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.chars().count() <= SOUL_DRIFT_DEFAULT_CAP);
    }

    #[test]
    fn test_write_oversize_single_line_not_emptied() {
        let tmp = tempdir().unwrap();
        // 单行超过容量：不应被清空，应按字符边界截断。
        let single_line = "长".repeat(SOUL_DRIFT_DEFAULT_CAP + 500);
        write_soul_drift(tmp.path(), "hero", &single_line).unwrap();
        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(!content.is_empty());
        assert_eq!(content.chars().count(), SOUL_DRIFT_DEFAULT_CAP);
    }

    #[test]
    fn writes_create_monotonic_revisions() {
        let tmp = tempdir().unwrap();
        let first = write_soul_drift(tmp.path(), "hero", "- first").unwrap();
        let second = append_soul_drift(tmp.path(), "hero", "- second").unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        let (content, revision) = read_soul_drift_with_revision(tmp.path(), "hero").unwrap();
        assert_eq!(revision, Some(2));
        assert_eq!(content, "- first\n- second");
        assert!(tmp
            .path()
            .join("characters/hero/soul_drift/revisions/1/manifest.json")
            .is_file());
        assert!(tmp
            .path()
            .join("characters/hero/soul_drift/revisions/2/manifest.json")
            .is_file());
    }

    #[test]
    fn first_revision_preserves_legacy_working_copy() {
        let tmp = tempdir().unwrap();
        let character_dir = tmp.path().join("characters/hero");
        fs::create_dir_all(&character_dir).unwrap();
        fs::write(character_dir.join("soul_drift.md"), "legacy").unwrap();

        let revision = write_soul_drift(tmp.path(), "hero", "updated").unwrap();

        assert_eq!(revision, 2);
        assert_eq!(
            fs::read_to_string(character_dir.join("soul_drift/revisions/1/soul_drift.md")).unwrap(),
            "legacy"
        );
        assert_eq!(read_soul_drift(tmp.path(), "hero").unwrap(), "updated");
    }

    #[test]
    fn rollback_commits_selected_content_as_new_revision() {
        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", "first").unwrap();
        write_soul_drift(tmp.path(), "hero", "second").unwrap();

        let rollback_revision = rollback_soul_drift(tmp.path(), "hero", 1).unwrap();

        assert_eq!(rollback_revision, 3);
        let (content, revision) = read_soul_drift_with_revision(tmp.path(), "hero").unwrap();
        assert_eq!(content, "first");
        assert_eq!(revision, Some(3));
        assert_eq!(
            fs::read_to_string(tmp.path().join("characters/hero/soul_drift.md")).unwrap(),
            "first"
        );
    }

    #[test]
    fn rollback_rejects_missing_revision() {
        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", "first").unwrap();

        let error = rollback_soul_drift(tmp.path(), "hero", 99).unwrap_err();

        assert!(matches!(error, AirpError::BadRequest(_)));
        assert_eq!(read_soul_drift(tmp.path(), "hero").unwrap(), "first");
    }

    #[test]
    fn capacity_fallback_retains_newest_lines() {
        let content = "- oldest\n- middle\n- newest";
        let bounded = enforce_capacity(content, 18);

        assert!(!bounded.contains("oldest"));
        assert!(bounded.contains("middle"));
        assert!(bounded.contains("newest"));
    }

    #[tokio::test]
    async fn oversized_append_calls_llm_before_committing() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let compressed = "- compressed drift";
        let event = serde_json::json!({"choices": [{"delta": {"content": compressed}}]});
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!("data: {event}\n\ndata: [DONE]\n\n")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", &"- old\n".repeat(200)).unwrap();
        let revision = append_soul_drift_with_compression(
            &reqwest::Client::new(),
            Arc::new(crate::adapter::ProviderConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: format!("{}/v1/chat/completions", server.uri()),
                api_key: Some("test-key".to_string()),
            }),
            crate::adapter::GenerationParams {
                model: "test-model".to_string(),
                temperature: None,
                max_tokens: None,
            },
            tmp.path(),
            "hero",
            &"- new\n".repeat(100),
        )
        .await
        .unwrap();

        assert_eq!(revision, 2);
        assert_eq!(read_soul_drift(tmp.path(), "hero").unwrap(), compressed);
    }

    #[tokio::test]
    async fn failed_compression_keeps_newest_patch_bounded() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempdir().unwrap();
        write_soul_drift(tmp.path(), "hero", &"- old\n".repeat(220)).unwrap();
        append_soul_drift_with_compression(
            &reqwest::Client::new(),
            Arc::new(crate::adapter::ProviderConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: format!("{}/v1/chat/completions", server.uri()),
                api_key: Some("test-key".to_string()),
            }),
            crate::adapter::GenerationParams {
                model: "test-model".to_string(),
                temperature: None,
                max_tokens: None,
            },
            tmp.path(),
            "hero",
            &format!("{}- newest patch", "- filler\n".repeat(40)),
        )
        .await
        .unwrap();

        let content = read_soul_drift(tmp.path(), "hero").unwrap();
        assert!(content.contains("- newest patch"));
        assert!(content.chars().count() <= SOUL_DRIFT_DEFAULT_CAP);
    }
}
