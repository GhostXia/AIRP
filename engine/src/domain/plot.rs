//! Plot arc domain service: user-authored story arc (起承转合) definitions.
//!
//! Extracted from `agent/tools/plot.rs` (E-P1-3 slice 2). Zero behavior change.
//! The daemon HTTP layer now calls `PlotService` instead of touching
//! `replace_file` / `fs::write` directly, closing the domain write path for
//! `plot_arc.json`.
//!
//! Boundary: `PlotService` only owns the standalone `plot_arc.json` asset. The
//! `plot_history` field inside `live.json` is owned by `StateService::mutate`
//! (see `agent/tools/plot.rs::AdvancePlotTool`), not by `PlotService`.

use std::path::{Path, PathBuf};

use crate::error::AirpError;

/// 剧情弧阶段。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlotPhase {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 目标轮次数。
    #[serde(default = "default_target_turns")]
    pub target_turns: u32,
    /// 是否已完成。
    #[serde(default)]
    pub completed: bool,
}

fn default_target_turns() -> u32 {
    5
}

/// 剧情弧定义（用户预设的"起承转合"大纲）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlotArc {
    /// 故事标题。
    pub title: String,
    /// 阶段列表。
    pub phases: Vec<PlotPhase>,
    /// 当前阶段 ID。
    pub current_phase: String,
    /// 当前阶段内进度 (0.0-1.0)。
    #[serde(default)]
    pub progress: f64,
    /// 总轮次计数（累积）。
    #[serde(default)]
    pub turn_count: u32,
    /// 当前阶段内的轮次计数（进入新阶段时重置为 0）。
    #[serde(default)]
    pub phase_turn_count: u32,
}

impl Default for PlotArc {
    fn default() -> Self {
        Self {
            title: "未命名故事".to_string(),
            phases: vec![
                PlotPhase {
                    id: "ki".into(),
                    name: "起".into(),
                    description: "引入世界观和角色".into(),
                    target_turns: 5,
                    completed: false,
                },
                PlotPhase {
                    id: "sho".into(),
                    name: "承".into(),
                    description: "发展冲突与关系".into(),
                    target_turns: 10,
                    completed: false,
                },
                PlotPhase {
                    id: "ten".into(),
                    name: "转".into(),
                    description: "高潮与转折".into(),
                    target_turns: 5,
                    completed: false,
                },
                PlotPhase {
                    id: "ketsu".into(),
                    name: "合".into(),
                    description: "结局收束".into(),
                    target_turns: 3,
                    completed: false,
                },
            ],
            current_phase: "ki".to_string(),
            progress: 0.0,
            turn_count: 0,
            phase_turn_count: 0,
        }
    }
}

impl PlotArc {
    /// 获取当前阶段。
    pub fn current_phase_obj(&self) -> Option<&PlotPhase> {
        self.phases.iter().find(|p| p.id == self.current_phase)
    }

    /// 推进轮次并更新进度。使用 per-phase turn counter 计算进度，进入新阶段时重置。
    pub fn advance_turn(&mut self) -> bool {
        self.turn_count += 1;
        self.phase_turn_count += 1;
        let phase_changed =
            if let Some(phase) = self.phases.iter_mut().find(|p| p.id == self.current_phase) {
                // 使用 per-phase turn count 而非累积 turn_count 计算进度
                self.progress = (self.phase_turn_count as f64 / phase.target_turns as f64).min(1.0);
                if self.progress >= 1.0 {
                    phase.completed = true;
                    // 进入下一个未完成的阶段，重置 phase_turn_count
                    if let Some(next) = self.phases.iter().find(|p| !p.completed) {
                        self.current_phase = next.id.clone();
                        self.progress = 0.0;
                        self.phase_turn_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
        phase_changed
    }
}

/// Plot arc domain service: read/write the standalone `plot_arc.json` asset.
///
/// Extracted from `agent/tools/plot.rs` (E-P1-3 slice 2). Zero behavior change.
/// All writes go through this service; the daemon HTTP layer no longer calls
/// `replace_file` / `fs::write` directly for `plot_arc.json`.
///
/// Boundary: this service only owns `plot_arc.json`. The `plot_history` field
/// inside `live.json` is owned by `StateService::mutate`.
#[derive(Clone, Debug)]
pub struct PlotService {
    data_root: PathBuf,
}

impl PlotService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    fn plot_arc_path(&self, character_id: &str) -> PathBuf {
        self.data_root
            .join("characters")
            .join(character_id)
            .join("plot_arc.json")
    }

    /// 读取剧情弧。不存在返回默认值。
    pub fn load_arc(&self, character_id: &str) -> Result<PlotArc, AirpError> {
        let path = self.plot_arc_path(character_id);
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(serde_json::from_str(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PlotArc::default()),
            Err(e) => Err(AirpError::from(e)),
        }
    }

    /// 保存剧情弧（原子写）。
    pub fn save_arc(&self, character_id: &str, arc: &PlotArc) -> Result<(), AirpError> {
        let path = self.plot_arc_path(character_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_vec_pretty(arc)?;
        crate::data_dir::replace_file(&path, &content)?;
        Ok(())
    }
}
