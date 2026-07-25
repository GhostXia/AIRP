//! 长期记忆遗忘曲线（Phase 2.5）。
//!
//! 每轮 compress 时按 `importance * recency` 衰减；低于阈值的记忆标记为
//! faded 不再注入 prompt。
//!
//! ## 数据模型
//! - `decay.json`：sidecar 元数据文件，与 `resident.md` 同目录。
//!   格式：`{ "<line_hash>": { "created_at": u64, "last_reinforced": u64, "importance": f64 } }`
//! - `faded.md`：衰减后被移出的条目（不注入 prompt，但保留可恢复）。
//!
//! ## 衰减公式
//! `weight = importance * exp(-decay_rate * days_since_last_reinforced)`
//! - `decay_rate`：默认 0.05（约 14 天半衰期），可通过 `AIRP_MEMORY_DECAY_RATE` 覆盖。
//! - `fade_threshold`：默认 0.15，可通过 `AIRP_MEMORY_FADE_THRESHOLD` 覆盖。
//!
//! ## 集成点
//! 在 `finalize.rs` 的 `run_memory_extraction` 中，压缩之前先执行 decay pass：
//! 1. 读 resident.md 所有条目
//! 2. 更新/创建 decay.json 元数据
//! 3. 计算每条 weight，低于阈值的移入 faded.md
//! 4. 若 decay 后仍超容量，再触发 LLM 压缩

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 默认衰减速率（约 14 天半衰期）。
pub const DEFAULT_DECAY_RATE: f64 = 0.05;
/// 默认淡出阈值。
pub const DEFAULT_FADE_THRESHOLD: f64 = 0.15;
/// 新条目默认重要度。
pub const DEFAULT_IMPORTANCE: f64 = 0.6;

/// 衰减配置。
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// 衰减速率 λ。weight = importance * exp(-λ * days)
    pub decay_rate: f64,
    /// 低于此阈值的条目将被移入 faded.md
    pub fade_threshold: f64,
}

impl Default for DecayConfig {
    fn default() -> Self {
        let decay_rate = std::env::var("AIRP_MEMORY_DECAY_RATE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|&r| r > 0.0)
            .unwrap_or(DEFAULT_DECAY_RATE);
        let fade_threshold = std::env::var("AIRP_MEMORY_FADE_THRESHOLD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|&t| t > 0.0 && t < 1.0)
            .unwrap_or(DEFAULT_FADE_THRESHOLD);
        Self {
            decay_rate,
            fade_threshold,
        }
    }
}

/// 单条记忆的衰减元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMeta {
    /// 创建时间（Unix 秒）。
    pub created_at: u64,
    /// 最后被强化时间（Unix 秒）。强化 = 内容被重新抽取或手动编辑。
    pub last_reinforced: u64,
    /// 重要度 [0.0, 1.0]。
    pub importance: f64,
}

/// decay.json 的完整结构。
type DecayStore = HashMap<String, EntryMeta>;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 计算条目的内容哈希（用于跨压缩追踪）。
/// 使用简单的 djb2 哈希，足够区分不同条目。
pub fn line_hash(line: &str) -> String {
    let trimmed = line.trim();
    let mut hash: u64 = 5381;
    for byte in trimmed.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    format!("{:016x}", hash)
}

/// 计算衰减权重。
pub fn decay_weight(meta: &EntryMeta, config: &DecayConfig, now: u64) -> f64 {
    let days = (now.saturating_sub(meta.last_reinforced)) as f64 / 86400.0;
    meta.importance * (-config.decay_rate * days).exp()
}

fn decay_json_path(session_dir: &Path) -> std::path::PathBuf {
    session_dir.join("decay.json")
}

fn faded_path(session_dir: &Path) -> std::path::PathBuf {
    session_dir.join("faded.md")
}

/// 读取 decay.json。不存在返回空 map。
fn load_decay_store(session_dir: &Path) -> DecayStore {
    let path = decay_json_path(session_dir);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 保存 decay.json。
fn save_decay_store(session_dir: &Path, store: &DecayStore) -> Result<(), AirpError> {
    let path = decay_json_path(session_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| AirpError::Internal(format!("decay.json serialize: {e}")))?;
    crate::data_dir::replace_file(&path, json.as_bytes())?;
    Ok(())
}

/// 衰减 pass 的结果。
#[derive(Debug)]
pub struct DecayResult {
    /// 衰减后保留在 resident.md 中的内容。
    pub retained: String,
    /// 被淡出的条目（追加到 faded.md）。
    pub faded: Vec<String>,
    /// 衰减前后的条目数。
    pub total_entries: usize,
    pub faded_count: usize,
}

/// 对 resident memory 执行衰减 pass。
///
/// 1. 解析 resident.md 为条目列表（以 "- " 开头的行为一条）
/// 2. 更新 decay.json（新条目创建元数据，已有条目保持）
/// 3. 计算每条 weight，低于阈值的移入 faded
/// 4. 返回保留内容和淡出条目
pub fn apply_decay(
    session_dir: &Path,
    content: &str,
    config: &DecayConfig,
) -> Result<DecayResult, AirpError> {
    let now = now_secs();
    let mut store = load_decay_store(session_dir);
    let mut retained_lines: Vec<String> = Vec::new();
    let mut faded_lines: Vec<String> = Vec::new();
    let mut total = 0usize;

    for line in content.lines() {
        let trimmed = line.trim();
        // 只处理 bullet 条目；空行和非 bullet 行跟随前一个条目
        if trimmed.starts_with("- ") {
            total += 1;
            let hash = line_hash(line);
            let meta = store.entry(hash).or_insert_with(|| EntryMeta {
                created_at: now,
                last_reinforced: now,
                importance: DEFAULT_IMPORTANCE,
            });
            let weight = decay_weight(meta, config, now);
            if weight < config.fade_threshold {
                faded_lines.push(line.to_string());
            } else {
                retained_lines.push(line.to_string());
            }
        } else if trimmed.is_empty() {
            // 空行：如果后面还有条目则保留为分隔
            retained_lines.push(String::new());
        } else {
            // 续行：跟随上一个条目的决策
            if faded_lines.len() > retained_lines.len() {
                faded_lines.push(line.to_string());
            } else {
                retained_lines.push(line.to_string());
            }
        }
    }

    // 清理尾部多余空行
    while retained_lines.last().is_some_and(|l| l.trim().is_empty()) {
        retained_lines.pop();
    }

    // 清理已不存在于 content 中的 stale 元数据
    let current_hashes: std::collections::HashSet<String> = content
        .lines()
        .filter(|l| l.trim().starts_with("- "))
        .map(line_hash)
        .collect();
    store.retain(|k, _| current_hashes.contains(k));

    save_decay_store(session_dir, &store)?;

    // 追加淡出条目到 faded.md
    if !faded_lines.is_empty() {
        let faded_file = faded_path(session_dir);
        let mut faded_content = fs::read_to_string(&faded_file).unwrap_or_default();
        if !faded_content.is_empty() && !faded_content.ends_with('\n') {
            faded_content.push('\n');
        }
        faded_content.push_str(&faded_lines.join("\n"));
        faded_content.push('\n');
        if let Some(parent) = faded_file.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::data_dir::replace_file(&faded_file, faded_content.as_bytes())?;
    }

    let faded_count = faded_lines
        .iter()
        .filter(|l| l.trim().starts_with("- "))
        .count();

    Ok(DecayResult {
        retained: retained_lines.join("\n"),
        faded: faded_lines,
        total_entries: total,
        faded_count,
    })
}

/// 强化条目：当记忆被重新抽取或手动编辑时，更新 last_reinforced。
pub fn reinforce_entry(session_dir: &Path, line: &str) -> Result<(), AirpError> {
    let mut store = load_decay_store(session_dir);
    let hash = line_hash(line);
    let now = now_secs();
    if let Some(meta) = store.get_mut(&hash) {
        meta.last_reinforced = now;
        // 被强化的条目重要度略微提升
        meta.importance = (meta.importance + 0.05).min(1.0);
    }
    save_decay_store(session_dir, &store)
}

/// 读取 faded.md 内容（供 API 查看/恢复）。
pub fn read_faded(session_dir: &Path) -> Result<String, AirpError> {
    let path = faded_path(session_dir);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AirpError::from(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn line_hash_is_stable() {
        let h1 = line_hash("- 用户喜欢猫");
        let h2 = line_hash("- 用户喜欢猫");
        assert_eq!(h1, h2);
        assert_ne!(h1, line_hash("- 用户喜欢狗"));
    }

    #[test]
    fn decay_weight_decreases_over_time() {
        let config = DecayConfig {
            decay_rate: 0.05,
            fade_threshold: 0.15,
        };
        let now = 1_700_000_000; // 足够大的时间戳
        let meta = EntryMeta {
            created_at: now - 86400 * 30,
            last_reinforced: now - 86400 * 30,
            importance: 0.6,
        };
        let weight_recent = decay_weight(
            &EntryMeta {
                last_reinforced: now,
                ..meta.clone()
            },
            &config,
            now,
        );
        let weight_old = decay_weight(&meta, &config, now);
        assert!(weight_recent > weight_old);
        assert!(weight_recent > 0.5); // 刚强化的应该很高
    }

    #[test]
    fn apply_decay_fades_old_low_importance_entries() {
        let tmp = tempdir().unwrap();
        let config = DecayConfig {
            decay_rate: 0.1, // 激进衰减
            fade_threshold: 0.2,
        };
        // 先写入 decay.json 模拟旧条目
        let old_time = now_secs() - 86400 * 60; // 60 天前
        let old_hash = line_hash("- 过时的信息");
        let mut store = HashMap::new();
        store.insert(
            old_hash,
            EntryMeta {
                created_at: old_time,
                last_reinforced: old_time,
                importance: 0.3,
            },
        );
        let json = serde_json::to_string(&store).unwrap();
        fs::write(tmp.path().join("decay.json"), json).unwrap();

        let content = "- 过时的信息\n- 最近的事实";
        let result = apply_decay(tmp.path(), content, &config).unwrap();

        assert!(result.retained.contains("最近的事实"));
        assert!(!result.retained.contains("过时的信息"));
        assert_eq!(result.faded_count, 1);
        // faded.md 应包含淡出条目
        let faded = read_faded(tmp.path()).unwrap();
        assert!(faded.contains("过时的信息"));
    }

    #[test]
    fn apply_decay_preserves_all_when_recent() {
        let tmp = tempdir().unwrap();
        let config = DecayConfig::default();
        let content = "- 新记忆一\n- 新记忆二\n- 新记忆三";
        let result = apply_decay(tmp.path(), content, &config).unwrap();
        assert_eq!(result.faded_count, 0);
        assert!(result.retained.contains("新记忆一"));
        assert!(result.retained.contains("新记忆二"));
        assert!(result.retained.contains("新记忆三"));
    }

    #[test]
    fn reinforce_updates_metadata() {
        let tmp = tempdir().unwrap();
        let content = "- 用户喜欢猫";
        // 先执行一次 decay 创建元数据
        let config = DecayConfig::default();
        apply_decay(tmp.path(), content, &config).unwrap();
        // 强化
        reinforce_entry(tmp.path(), "- 用户喜欢猫").unwrap();
        let store = load_decay_store(tmp.path());
        let hash = line_hash("- 用户喜欢猫");
        let meta = store.get(&hash).unwrap();
        assert!(meta.importance > DEFAULT_IMPORTANCE);
    }
}
