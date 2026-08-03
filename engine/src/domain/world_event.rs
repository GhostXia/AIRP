//! World event domain service: world event definitions, world clock, and
//! time-triggered event injection.
//!
//! Extracted from `agent/tools/world_event.rs` (E-P1-3 slice 1). Zero behavior
//! change. The agent tool layer now calls `WorldEventService` instead of
//! touching `replace_file` / `fs::write` directly, closing the domain write
//! path for world events and world clock.

use std::path::{Path, PathBuf};

use crate::error::AirpError;
use crate::revision::atomic::{
    commit_revision, next_content_revision, read_current_revision, CommitOptions, StagedRevision,
};
use crate::revision::manifest::{AssetKind, AssetSource};

/// 世界事件定义。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorldEvent {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 触发关键词（任一命中即可触发）。
    #[serde(default)]
    pub trigger_keywords: Vec<String>,
    /// 最小触发轮次。
    #[serde(default)]
    pub min_turn: Option<u32>,
    /// Phase 2.3: 时间触发条件——当 world_clock >= time_trigger 时自动触发。
    #[serde(default)]
    pub time_trigger: Option<u64>,
    /// 事件内容（注入到叙事上下文）。
    pub content: String,
    /// 是否已触发。
    #[serde(default)]
    pub triggered: bool,
}

/// 世界时钟配置与状态。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorldClock {
    /// 当前时间（抽象时间单位，从 0 开始）。
    pub current_time: u64,
    /// 每轮自动推进的时间单位。
    pub advance_per_turn: u64,
    /// 时间单位描述（用于显示，如 "hour", "day"）。
    pub time_unit: String,
    /// 可选的显示格式（如 "第{day}天 {hour}:00"）。
    #[serde(default)]
    pub display_format: Option<String>,
}

impl Default for WorldClock {
    fn default() -> Self {
        Self {
            current_time: 0,
            advance_per_turn: 1,
            time_unit: "hour".to_string(),
            display_format: None,
        }
    }
}

impl WorldClock {
    /// 生成人类可读的时间显示。
    pub fn display(&self) -> String {
        if let Some(ref fmt) = self.display_format {
            // 简单模板替换
            let hours_per_day = 24u64;
            let day = self.current_time / hours_per_day + 1;
            let hour = self.current_time % hours_per_day;
            fmt.replace("{day}", &day.to_string())
                .replace("{hour}", &format!("{:02}", hour))
                .replace("{time}", &self.current_time.to_string())
        } else {
            format!("T+{} {}", self.current_time, self.time_unit)
        }
    }
}

/// World event domain service: read/write world events and world clock with
/// revision contract support.
///
/// Extracted from `agent/tools/world_event.rs` (E-P1-3 slice 1). Zero behavior
/// change. All writes go through this service; agent tools no longer call
/// `replace_file` / `fs::write` directly for world event or world clock assets.
#[derive(Clone, Debug)]
pub struct WorldEventService {
    data_root: PathBuf,
}

impl WorldEventService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    fn world_events_path(&self, character_id: &str) -> PathBuf {
        self.data_root
            .join("characters")
            .join(character_id)
            .join("world_events.json")
    }

    /// #280: world_events 的 revision asset_dir，与工作副本 `world_events.json` 分离。
    fn world_events_asset_dir(&self, character_id: &str) -> PathBuf {
        self.data_root
            .join("characters")
            .join(character_id)
            .join("world_events")
    }

    fn world_clock_path(&self, character_id: &str) -> PathBuf {
        self.data_root
            .join("characters")
            .join(character_id)
            .join("world_clock.json")
    }

    /// #280: 将 world_events 写入 revision 快照目录。
    fn commit_world_events_revision(
        &self,
        character_id: &str,
        content: &[u8],
        source_kind: &str,
        parent_revision: Option<u64>,
    ) -> Result<u64, AirpError> {
        let asset_dir = self.world_events_asset_dir(character_id);
        let revision = next_content_revision(&asset_dir)?;
        let staged = StagedRevision {
            content_revision: revision,
            asset_kind: AssetKind::WorldEvents,
            asset_id: character_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source: AssetSource {
                source_kind: source_kind.to_string(),
                parent_revision,
                ..Default::default()
            },
            files: vec![("world_events.json".to_string(), content.to_vec())],
        };
        commit_revision(&staged, &CommitOptions::new(asset_dir))?;
        Ok(revision)
    }

    /// #280: legacy migration——若 `world_events.json` 存在但无 revision 快照，
    /// 首次 commit 为 revision 1。调用方已在 `state_lock` 临界区内，无需额外加锁。
    fn ensure_legacy_world_events_revision(
        &self,
        character_id: &str,
    ) -> Result<Option<u64>, AirpError> {
        let asset_dir = self.world_events_asset_dir(character_id);
        if let Some(existing) = read_current_revision(&asset_dir)? {
            return Ok(Some(existing));
        }
        let path = self.world_events_path(character_id);
        if !path.exists() {
            return Ok(None);
        }
        let legacy = std::fs::read(&path)?;
        let rev =
            self.commit_world_events_revision(character_id, &legacy, "legacy_migration", None)?;
        Ok(Some(rev))
    }

    /// 读取世界事件列表。不存在返回空 Vec。
    pub fn load_events(&self, character_id: &str) -> Result<Vec<WorldEvent>, AirpError> {
        let path = self.world_events_path(character_id);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let events: Vec<WorldEvent> = serde_json::from_str(&content)?;
                Ok(events)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(AirpError::from(e)),
        }
    }

    /// 保存世界事件列表（原子写 + revision 合同）。
    ///
    /// 顺序（CodeRabbit review 修正）：
    /// 1. legacy migration 必须在 replace_file 之前，确保读到的是真正的 legacy 内容
    /// 2. commit_revision 写不可变快照（在暴露工作副本之前）
    /// 3. replace_file 写工作副本（commit 成功后才暴露给并发 reader）
    pub fn save_events(&self, character_id: &str, events: &[WorldEvent]) -> Result<(), AirpError> {
        let path = self.world_events_path(character_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_vec_pretty(events)?;

        let parent_revision = self.ensure_legacy_world_events_revision(character_id)?;
        self.commit_world_events_revision(
            character_id,
            &content,
            "tool_triggered",
            parent_revision,
        )?;

        // 原子写工作副本：tmp + rename + fsync(parent)，避免半写状态被并发 reader 看到。
        crate::data_dir::replace_file(&path, &content)?;
        Ok(())
    }

    /// 读取世界时钟。不存在返回默认值。
    pub fn load_clock(&self, character_id: &str) -> Result<WorldClock, AirpError> {
        let path = self.world_clock_path(character_id);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let clock: WorldClock = serde_json::from_str(&content)?;
                Ok(clock)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WorldClock::default()),
            Err(e) => Err(AirpError::from(e)),
        }
    }

    /// 保存世界时钟（原子写）。
    pub fn save_clock(&self, character_id: &str, clock: &WorldClock) -> Result<(), AirpError> {
        let path = self.world_clock_path(character_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_vec_pretty(clock)?;
        crate::data_dir::replace_file(&path, &content)?;
        Ok(())
    }

    /// 推进时钟并检查时间触发事件。返回触发的事件列表与待追加到 `current.md`
    /// 的内容缓冲。调用方负责在 `session_lock` 临界区内执行 `append_to_current`。
    ///
    /// 不在此函数内执行 `append_to_current` 的原因：append 必须持有
    /// `session_lock(character_id, session_id)`，而时钟/事件状态变更必须持有
    /// `state_lock(character_id)`。若同一调用同时持有两把锁（state → session），
    /// 会与 `advance_plot`（session → state，经 `StateService::mutate`）形成
    /// 锁序倒置死锁。因此将状态变更（state_lock）与内容追加（session_lock）
    /// 拆分到两个临界区，由调用方分别加锁。
    pub fn advance_and_check_triggers(
        &self,
        character_id: &str,
        advance_by: Option<u64>,
    ) -> Result<(WorldClock, Vec<WorldEvent>, String), AirpError> {
        let mut clock = self.load_clock(character_id)?;
        let advance = advance_by.unwrap_or(clock.advance_per_turn);
        // 溢出检查：避免 unchecked addition 导致 u64 wrap-around
        clock.current_time = clock
            .current_time
            .checked_add(advance)
            .ok_or_else(|| AirpError::BadRequest("clock advance overflow".to_string()))?;
        self.save_clock(character_id, &clock)?;

        // 检查时间触发事件
        let mut events = self.load_events(character_id)?;
        let mut triggered_events = Vec::new();

        // 先收集所有到期事件，构造单次追加内容。
        // 新实现：批量收集 → 标记 + 持久化 → 由调用方在 session_lock 下单次 append。
        // 顺序权衡（save → append）：若 append 失败，事件已标记 triggered（不会
        // 重触发），内容未注入——失败对调用方可见（Err），不会静默累积重复内容。
        let mut due_indices: Vec<usize> = Vec::new();
        let mut content_buf = String::new();
        for (idx, event) in events.iter().enumerate() {
            if !event.triggered {
                if let Some(time_trigger) = event.time_trigger {
                    if clock.current_time >= time_trigger {
                        content_buf.push_str(&format!(
                            "\n[世界事件: {}]\n{}\n",
                            event.name, event.content
                        ));
                        due_indices.push(idx);
                    }
                }
            }
        }

        if !due_indices.is_empty() {
            for &idx in &due_indices {
                events[idx].triggered = true;
                triggered_events.push(events[idx].clone());
            }
            self.save_events(character_id, &events)?;
        }
        Ok((clock, triggered_events, content_buf))
    }
}
