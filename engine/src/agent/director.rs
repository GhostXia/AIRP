//! Phase 2.1: 导演 Agent 编排。
//!
//! DirectorAgent 持有 plot_status + npc_registry + world_clock；
//! 每轮用户消息后决定是否介入（引入冲突/切换场景/推进时间线/NPC 行动）。
//! 角色 Agent 只负责扮演，导演负责叙事编排。
//!
//! ## 设计
//! - 导演不直接生成叙事文本，而是产出"指令"注入到下一轮角色 Agent 的上下文中
//! - 指令通过 `director_directive.md` 文件传递给 prepare 阶段
//! - 导演决策使用控制平面 LLM 调用（与 style review 同模式）
//!
//! ## 集成点
//! - finalize 后异步触发导演评估
//! - prepare 阶段读取 director_directive.md 注入到 system prompt

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 导演配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorConfig {
    /// 是否启用导演 Agent。
    pub enabled: bool,
    /// 每 N 轮评估一次（避免每轮都调 LLM）。
    pub evaluate_interval: u32,
    /// 导演介入的最低叙事张力阈值 (0.0-1.0)。
    pub tension_threshold: f64,
    /// 是否允许导演自动推进时间线。
    pub auto_advance_clock: bool,
    /// 是否允许导演引入 NPC 行动。
    pub allow_npc_actions: bool,
}

impl Default for DirectorConfig {
    fn default() -> Self {
        Self {
            enabled: std::env::var("AIRP_DIRECTOR_ENABLED")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            evaluate_interval: std::env::var("AIRP_DIRECTOR_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            tension_threshold: 0.4,
            auto_advance_clock: true,
            allow_npc_actions: true,
        }
    }
}

/// 导演决策。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision_type", rename_all = "snake_case")]
pub enum DirectorDecision {
    /// 不介入，继续观察。
    Observe,
    /// 引入冲突或转折。
    IntroduceConflict {
        /// 冲突描述（注入到角色上下文）。
        description: String,
        /// 建议的叙事方向。
        suggestion: String,
    },
    /// 切换场景。
    SwitchScene {
        /// 新场景描述。
        scene_description: String,
        /// 切换原因。
        reason: String,
    },
    /// 推进时间线。
    AdvanceTimeline {
        /// 推进的时间单位数。
        units: u64,
        /// 时间推进带来的变化描述。
        changes: String,
    },
    /// NPC 自主行动。
    NpcAction {
        /// NPC 名称。
        npc_name: String,
        /// 行动描述。
        npc_action: String,
    },
    /// 推进剧情弧。
    AdvancePlot {
        /// 剧情推进方向。
        direction: String,
        /// 当前弧进度评估。
        progress_note: String,
    },
}

/// 导演状态（持久化）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DirectorState {
    /// 当前轮次计数。
    pub turn_count: u32,
    /// 当前叙事张力评估 (0.0-1.0)。
    pub tension_level: f64,
    /// 当前剧情阶段（起/承/转/合）。
    pub plot_phase: String,
    /// 最近的导演决策历史。
    pub recent_decisions: Vec<DirectorDecisionRecord>,
    /// 导演备注（累积的叙事线索）。
    pub notes: Vec<String>,
}

/// 导演决策记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorDecisionRecord {
    pub turn: u32,
    pub decision: DirectorDecision,
    pub timestamp: u64,
}

fn director_state_path(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("director_state.json")
}

fn director_directive_path(session_dir: &Path) -> PathBuf {
    session_dir.join("director_directive.md")
}

/// 读取导演状态。
pub fn load_director_state(
    data_root: &Path,
    character_id: &str,
) -> Result<DirectorState, AirpError> {
    let path = director_state_path(data_root, character_id);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DirectorState::default()),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 保存导演状态。
pub fn save_director_state(
    data_root: &Path,
    character_id: &str,
    state: &DirectorState,
) -> Result<(), AirpError> {
    let path = director_state_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| AirpError::Internal(format!("director state serialize: {e}")))?;
    crate::data_dir::replace_file(&path, json.as_bytes())?;
    Ok(())
}

/// 写入导演指令到 session 目录（供 prepare 阶段注入）。
pub fn write_directive(session_dir: &Path, decision: &DirectorDecision) -> Result<(), AirpError> {
    let path = director_directive_path(session_dir);
    let content = match decision {
        DirectorDecision::Observe => String::new(),
        DirectorDecision::IntroduceConflict {
            description,
            suggestion,
        } => {
            format!(
                "[导演指令：引入冲突]\n{}\n建议方向：{}\n",
                description, suggestion
            )
        }
        DirectorDecision::SwitchScene {
            scene_description,
            reason,
        } => {
            format!(
                "[导演指令：场景切换]\n新场景：{}\n原因：{}\n",
                scene_description, reason
            )
        }
        DirectorDecision::AdvanceTimeline { units, changes } => {
            format!(
                "[导演指令：时间推进]\n推进 {} 个时间单位。\n变化：{}\n",
                units, changes
            )
        }
        DirectorDecision::NpcAction {
            npc_name,
            npc_action,
        } => {
            format!("[导演指令：NPC 行动]\n{}：{}\n", npc_name, npc_action)
        }
        DirectorDecision::AdvancePlot {
            direction,
            progress_note,
        } => {
            format!(
                "[导演指令：剧情推进]\n方向：{}\n进度：{}\n",
                direction, progress_note
            )
        }
    };
    if content.is_empty() {
        // 清除旧指令
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::data_dir::replace_file(&path, content.as_bytes())?;
    Ok(())
}

/// 读取导演指令（供 prepare 阶段注入到 system prompt）。
pub fn read_directive(session_dir: &Path) -> String {
    let path = director_directive_path(session_dir);
    fs::read_to_string(&path).unwrap_or_default()
}

/// 导演评估 prompt 模板。
pub const DIRECTOR_EVALUATE_PROMPT: &str = r#"你是一个 RP 叙事导演。根据当前对话状态，决定下一步的叙事编排。

你的职责：
1. 评估当前叙事张力（0.0-1.0）
2. 决定是否需要介入（引入冲突/切换场景/推进时间/NPC行动/推进剧情）
3. 如果对话自然流畅且张力足够，选择不介入（observe）

输出 JSON：
{
  "tension_level": 0.0-1.0,
  "plot_phase": "起|承|转|合",
  "decision": {
    "action": "observe|introduce_conflict|switch_scene|advance_timeline|npc_action|advance_plot",
    ...action-specific fields
  },
  "note": "可选的导演备注"
}

注意：
- 不要过于频繁介入，让角色自然互动
- 冲突引入要符合已有设定
- 时间推进要有叙事意义
- NPC 行动要推动剧情而非干扰"#;

/// 判断当前轮次是否需要导演评估。
pub fn should_evaluate(config: &DirectorConfig, state: &DirectorState) -> bool {
    if !config.enabled {
        return false;
    }
    state.turn_count.is_multiple_of(config.evaluate_interval)
}

/// 更新导演状态（轮次计数 + 决策记录）。
pub fn record_turn(state: &mut DirectorState, decision: Option<DirectorDecision>) {
    state.turn_count += 1;
    if let Some(d) = decision {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // 只保留最近 20 条决策记录
        if state.recent_decisions.len() >= 20 {
            state.recent_decisions.remove(0);
        }
        state.recent_decisions.push(DirectorDecisionRecord {
            turn: state.turn_count,
            decision: d,
            timestamp: now,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn director_state_roundtrip() {
        let tmp = tempdir().unwrap();
        let state = DirectorState {
            turn_count: 5,
            tension_level: 0.7,
            plot_phase: "承".to_string(),
            ..Default::default()
        };
        save_director_state(tmp.path(), "hero", &state).unwrap();
        let loaded = load_director_state(tmp.path(), "hero").unwrap();
        assert_eq!(loaded.turn_count, 5);
        assert_eq!(loaded.plot_phase, "承");
    }

    #[test]
    fn directive_write_and_read() {
        let tmp = tempdir().unwrap();
        let decision = DirectorDecision::IntroduceConflict {
            description: "突然暴风雨来袭".to_string(),
            suggestion: "角色需要寻找避难所".to_string(),
        };
        write_directive(tmp.path(), &decision).unwrap();
        let content = read_directive(tmp.path());
        assert!(content.contains("引入冲突"));
        assert!(content.contains("暴风雨"));
    }

    #[test]
    fn observe_clears_directive() {
        let tmp = tempdir().unwrap();
        // 先写一个指令
        let decision = DirectorDecision::NpcAction {
            npc_name: "商人".to_string(),
            npc_action: "带来远方的消息".to_string(),
        };
        write_directive(tmp.path(), &decision).unwrap();
        assert!(!read_directive(tmp.path()).is_empty());
        // Observe 清除
        write_directive(tmp.path(), &DirectorDecision::Observe).unwrap();
        assert!(read_directive(tmp.path()).is_empty());
    }

    #[test]
    fn should_evaluate_respects_config() {
        let config = DirectorConfig {
            enabled: true,
            evaluate_interval: 3,
            ..Default::default()
        };
        let state = DirectorState {
            turn_count: 0,
            ..Default::default()
        };
        assert!(should_evaluate(&config, &state)); // 0 % 3 == 0
        let state = DirectorState {
            turn_count: 1,
            ..Default::default()
        };
        assert!(!should_evaluate(&config, &state));
        let state = DirectorState {
            turn_count: 3,
            ..Default::default()
        };
        assert!(should_evaluate(&config, &state));
    }

    #[test]
    fn disabled_config_never_evaluates() {
        let config = DirectorConfig {
            enabled: false,
            ..Default::default()
        };
        let state = DirectorState::default();
        assert!(!should_evaluate(&config, &state));
    }

    #[test]
    fn record_turn_trims_history() {
        let mut state = DirectorState::default();
        for _ in 0..25 {
            record_turn(&mut state, Some(DirectorDecision::Observe));
        }
        assert_eq!(state.turn_count, 25);
        assert!(state.recent_decisions.len() <= 20);
    }
}
