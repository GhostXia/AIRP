/// DX-3: Per-user daily quota enforcement.
///
/// Quotas are stored in `data/users/{user_id}/quota.json` (user-scoped) or
/// `data/quota.json` (global, single-user mode).  Each quota record tracks:
/// - `requests_today`: number of POST /v1/chat/completions calls
/// - `tokens_today`: estimated tokens generated (LLM output only)
/// - `date`: ISO-8601 date string; resets counters when day changes
///
/// Limits are configured via `QuotaConfig` in `MutableConfig`/`settings.json`.
/// When a limit is 0 (default), that dimension is uncapped.
///
/// `check_and_increment` returns `Err(QuotaExceeded)` before the request
/// proceeds; callers map this to HTTP 429.
use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Config ──────────────────────────────────────────────────────────────────

/// Per-user daily quota limits.  0 = unlimited for that dimension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct QuotaConfig {
    /// Max chat-completion requests per calendar day (UTC).  0 = unlimited.
    pub max_requests_per_day: u32,
    /// Max estimated output tokens per calendar day.  0 = unlimited.
    pub max_tokens_per_day: u32,
}

impl QuotaConfig {
    /// Returns `true` if all limits are 0 (quota checking is a no-op).
    pub fn is_unlimited(&self) -> bool {
        self.max_requests_per_day == 0 && self.max_tokens_per_day == 0
    }
}

// ─── State ───────────────────────────────────────────────────────────────────

/// Daily usage counters, persisted as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaState {
    /// ISO-8601 date (YYYY-MM-DD, UTC) this record belongs to.
    pub date: String,
    /// Number of requests made today.
    pub requests_today: u32,
    /// Estimated tokens consumed today.
    pub tokens_today: u32,
}

impl QuotaState {
    fn today_utc() -> String {
        // Use chrono if available; otherwise fall back to manual UTC derivation.
        use chrono::Utc;
        Utc::now().format("%Y-%m-%d").to_string()
    }

    /// Load from file, resetting counters if the stored date ≠ today (UTC).
    pub fn load(path: &Path) -> Self {
        let today = Self::today_utc();
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(mut s) = serde_json::from_str::<QuotaState>(&raw) {
                if s.date == today {
                    return s;
                }
                // Day rolled over — reset counters, keep the file structure.
                s.date = today;
                s.requests_today = 0;
                s.tokens_today = 0;
                return s;
            }
        }
        QuotaState {
            date: today,
            requests_today: 0,
            tokens_today: 0,
        }
    }

    /// Persist to file (best-effort; failures are logged but not fatal).
    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(path, json) {
                tracing::warn!(err = %e, path = ?path, "quota: 持久化失败");
            }
        }
    }
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

/// Path to the quota file for the given effective data root.
pub fn quota_file_path(effective_root: &Path) -> PathBuf {
    effective_root.join("quota.json")
}

// ─── Core logic ───────────────────────────────────────────────────────────────

/// Check current usage against `config` limits, then increment `requests_today`.
///
/// Call **before** the request is processed.  After the LLM response completes,
/// call `record_tokens` to credit output tokens.
///
/// Returns `Err(AirpError::QuotaExceeded)` if any limit would be breached.
/// On success, the incremented state is persisted immediately.
pub fn check_and_increment(effective_root: &Path, config: &QuotaConfig) -> Result<(), AirpError> {
    if config.is_unlimited() {
        return Ok(());
    }

    let path = quota_file_path(effective_root);
    let mut state = QuotaState::load(&path);

    if config.max_requests_per_day > 0 && state.requests_today >= config.max_requests_per_day {
        return Err(AirpError::QuotaExceeded(format!(
            "请求配额已达上限：今日已发 {} 次，上限 {} 次/天",
            state.requests_today, config.max_requests_per_day
        )));
    }

    // A2-1: token 维度同样在请求前 gate。若今日已用 token 达上限，拒绝新请求。
    // record_tokens 在响应完成后回填实际输出 token，所以这里用的是上一轮累计值。
    if config.max_tokens_per_day > 0 && state.tokens_today >= config.max_tokens_per_day {
        return Err(AirpError::QuotaExceeded(format!(
            "token 配额已达上限：今日已用约 {} tokens，上限 {} tokens/天",
            state.tokens_today, config.max_tokens_per_day
        )));
    }

    state.requests_today += 1;
    state.save(&path);
    Ok(())
}

/// Record `tokens` additional output tokens consumed by a completed response.
///
/// Best-effort: if the quota file doesn't exist or is unreadable, the call
/// is silently ignored (we never block a completed response).
pub fn record_tokens(effective_root: &Path, tokens: u32) {
    if tokens == 0 {
        return;
    }
    let path = quota_file_path(effective_root);
    let mut state = QuotaState::load(&path);
    state.tokens_today = state.tokens_today.saturating_add(tokens);
    state.save(&path);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests_dx3 {
    use super::*;
    use tempfile::tempdir;

    fn unlimited() -> QuotaConfig {
        QuotaConfig::default()
    }

    fn limited(reqs: u32, tokens: u32) -> QuotaConfig {
        QuotaConfig {
            max_requests_per_day: reqs,
            max_tokens_per_day: tokens,
        }
    }

    #[test]
    fn test_quota_config_default_is_unlimited() {
        assert!(QuotaConfig::default().is_unlimited());
    }

    #[test]
    fn test_check_unlimited_always_ok() {
        let dir = tempdir().unwrap();
        for _ in 0..100 {
            assert!(check_and_increment(dir.path(), &unlimited()).is_ok());
        }
    }

    #[test]
    fn test_check_increments_counter() {
        let dir = tempdir().unwrap();
        let cfg = limited(10, 0);
        check_and_increment(dir.path(), &cfg).unwrap();
        check_and_increment(dir.path(), &cfg).unwrap();
        let state = QuotaState::load(&quota_file_path(dir.path()));
        assert_eq!(state.requests_today, 2);
    }

    #[test]
    fn test_check_rejects_when_limit_reached() {
        let dir = tempdir().unwrap();
        let cfg = limited(2, 0);
        check_and_increment(dir.path(), &cfg).unwrap();
        check_and_increment(dir.path(), &cfg).unwrap();
        let err = check_and_increment(dir.path(), &cfg);
        assert!(err.is_err(), "third request should be rejected");
        match err.unwrap_err() {
            AirpError::QuotaExceeded(_) => {}
            e => panic!("expected QuotaExceeded, got {:?}", e),
        }
    }

    #[test]
    fn test_check_rejects_when_token_limit_reached() {
        // A2-1: max_tokens_per_day must actually gate requests.
        let dir = tempdir().unwrap();
        let cfg = limited(0, 100); // unlimited requests, 100-token/day cap
                                   // First request passes (tokens_today = 0).
        check_and_increment(dir.path(), &cfg).unwrap();
        // A completed response credits 150 output tokens — over budget.
        record_tokens(dir.path(), 150);
        // Next request must be rejected on the token dimension.
        let err = check_and_increment(dir.path(), &cfg);
        assert!(err.is_err(), "request past token cap should be rejected");
        match err.unwrap_err() {
            AirpError::QuotaExceeded(msg) => assert!(msg.contains("token")),
            e => panic!("expected QuotaExceeded, got {:?}", e),
        }
    }

    #[test]
    fn test_record_tokens_accumulates() {
        let dir = tempdir().unwrap();
        record_tokens(dir.path(), 100);
        record_tokens(dir.path(), 50);
        let state = QuotaState::load(&quota_file_path(dir.path()));
        assert_eq!(state.tokens_today, 150);
    }

    #[test]
    fn test_quota_state_resets_on_new_day() {
        let dir = tempdir().unwrap();
        // Write a quota file with yesterday's date
        let yesterday = "1970-01-01";
        let old = QuotaState {
            date: yesterday.to_string(),
            requests_today: 99,
            tokens_today: 9999,
        };
        let path = quota_file_path(dir.path());
        std::fs::write(&path, serde_json::to_string(&old).unwrap()).unwrap();

        // Loading should reset counters
        let loaded = QuotaState::load(&path);
        assert_eq!(loaded.requests_today, 0);
        assert_eq!(loaded.tokens_today, 0);
        assert_ne!(loaded.date, yesterday);
    }

    #[test]
    fn test_quota_no_file_is_fresh_start() {
        let dir = tempdir().unwrap();
        let state = QuotaState::load(&quota_file_path(dir.path()));
        assert_eq!(state.requests_today, 0);
        assert_eq!(state.tokens_today, 0);
    }
}
