//! Phase 2.6: 多 Agent 辩论/会议模式（Council）。
//!
//! scene 内多角色 Agent 就议题各自生成回复，用户可选择介入或旁听。
//! 需要并发调用 + 发言顺序调度。
//!
//! ## 设计
//! - `CouncilConfig`: 会议配置（参与者、议题、轮次、发言顺序）
//! - `CouncilSession`: 一次会议的状态
//! - `CouncilTurn`: 单轮发言结果
//! - 发言顺序：round-robin / 随机 / 按相关性
//! - 用户可在任意轮次介入发言
//!
//! ## 集成
//! - 通过 `POST /v1/council/start` 启动会议
//! - 通过 `POST /v1/council/round` 推进一轮
//! - 通过 `POST /v1/council/intervene` 用户介入
//! - 会议记录存入 session 的 current.md

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 发言顺序策略。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeakingOrder {
    /// 轮流发言。
    #[default]
    RoundRobin,
    /// 随机顺序。
    Random,
    /// 按与议题的相关性排序（需要 LLM 评估）。
    Relevance,
}

/// 会议配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilConfig {
    /// 会议主题/议题。
    pub topic: String,
    /// 参与者角色 ID 列表。
    pub participants: Vec<String>,
    /// 最大轮次。
    pub max_rounds: u32,
    /// 发言顺序策略。
    pub order: SpeakingOrder,
    /// 是否允许用户介入。
    pub allow_intervention: bool,
    /// 每轮每参与者最大 token 数。
    pub max_tokens_per_turn: u32,
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            topic: String::new(),
            participants: Vec::new(),
            max_rounds: 3,
            order: SpeakingOrder::RoundRobin,
            allow_intervention: true,
            max_tokens_per_turn: 500,
        }
    }
}

/// 单条发言。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilTurn {
    /// 发言者角色 ID。
    pub speaker: String,
    /// 发言内容。
    pub content: String,
    /// 轮次编号。
    pub round: u32,
    /// 是否是用户介入。
    pub is_intervention: bool,
}

/// 会议状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilSession {
    /// 配置。
    pub config: CouncilConfig,
    /// 当前轮次。
    pub current_round: u32,
    /// 当前发言者索引。
    pub current_speaker_idx: usize,
    /// 所有发言记录。
    pub turns: Vec<CouncilTurn>,
    /// 是否已结束。
    pub finished: bool,
    /// 结束原因。
    pub end_reason: Option<String>,
    /// 发言顺序调度表（Random/Relevance 策略时使用，空则回退到 participants 顺序）。
    #[serde(default)]
    pub speaker_schedule: Vec<String>,
}

impl CouncilSession {
    /// 创建新会议。验证 config：无参与者或 max_rounds==0 时直接标记为 finished。
    pub fn new(config: CouncilConfig) -> Self {
        if config.participants.is_empty() || config.max_rounds == 0 {
            return Self {
                config,
                current_round: 0,
                current_speaker_idx: 0,
                turns: Vec::new(),
                finished: true,
                end_reason: Some("无效配置：无参与者或最大轮次为零".to_string()),
                speaker_schedule: Vec::new(),
            };
        }
        let mut session = Self {
            config,
            current_round: 1,
            current_speaker_idx: 0,
            turns: Vec::new(),
            finished: false,
            end_reason: None,
            speaker_schedule: Vec::new(),
        };
        // 为 Random / Relevance 策略生成发言顺序表
        session.build_speaker_schedule();
        session
    }

    /// 根据发言顺序策略生成 participant 调度表。
    fn build_speaker_schedule(&mut self) {
        match self.config.order {
            SpeakingOrder::RoundRobin => {
                // RoundRobin 不需要额外调度表，直接按 participants 顺序
            }
            SpeakingOrder::Random => {
                // 简单 shuffle：使用确定性旋转（避免引入 rand 依赖）
                // 用 topic 长度作为种子做一次 rotation
                let n = self.config.participants.len();
                let offset = self.config.topic.len() % n.max(1);
                let mut schedule: Vec<String> = self.config.participants.clone();
                schedule.rotate_right(offset);
                self.speaker_schedule = schedule;
            }
            SpeakingOrder::Relevance => {
                // Relevance 需要 LLM 评估，此处降级为 RoundRobin 顺序
                // 实际相关性排序应在调用方用 LLM 评估后覆盖 speaker_schedule
                self.speaker_schedule = self.config.participants.clone();
            }
        }
    }

    /// 获取当前发言者。优先使用 speaker_schedule，空则回退到 participants。
    pub fn current_speaker(&self) -> Option<&str> {
        if self.finished || self.config.participants.is_empty() {
            return None;
        }
        if !self.speaker_schedule.is_empty() {
            self.speaker_schedule
                .get(self.current_speaker_idx)
                .map(|s| s.as_str())
        } else {
            self.config
                .participants
                .get(self.current_speaker_idx)
                .map(|s| s.as_str())
        }
    }

    /// 推进到下一个发言者。返回是否进入了新轮次。
    pub fn advance_speaker(&mut self) -> bool {
        if self.finished {
            return false;
        }
        let pool_len = if !self.speaker_schedule.is_empty() {
            self.speaker_schedule.len()
        } else {
            self.config.participants.len()
        };
        self.current_speaker_idx += 1;
        if self.current_speaker_idx >= pool_len {
            self.current_speaker_idx = 0;
            self.current_round += 1;
            if self.current_round > self.config.max_rounds {
                self.finished = true;
                self.end_reason = Some("达到最大轮次".to_string());
                return false;
            }
            return true;
        }
        false
    }

    /// 添加发言。验证 session 状态：
    /// - 已结束的 session 拒绝任何新发言
    /// - 非介入发言必须来自当前发言者
    /// - 介入发言在 allow_intervention=false 时被拒绝
    pub fn add_turn(
        &mut self,
        speaker: &str,
        content: &str,
        is_intervention: bool,
    ) -> Result<(), String> {
        if self.finished {
            return Err("会议已结束，无法添加发言".to_string());
        }
        if is_intervention && !self.config.allow_intervention {
            return Err("当前会议不允许用户介入".to_string());
        }
        if !is_intervention {
            // 非介入发言必须来自当前发言者
            if let Some(expected) = self.current_speaker() {
                if speaker != expected {
                    return Err(format!(
                        "非介入发言者不匹配：期望 {expected}，实际 {speaker}"
                    ));
                }
            }
        }
        self.turns.push(CouncilTurn {
            speaker: speaker.to_string(),
            content: content.to_string(),
            round: self.current_round,
            is_intervention,
        });
        Ok(())
    }

    /// 结束会议。
    pub fn finish(&mut self, reason: &str) {
        self.finished = true;
        self.end_reason = Some(reason.to_string());
    }

    /// 生成会议摘要（用于注入到叙事上下文）。
    pub fn summary(&self) -> String {
        let mut s = format!("[会议记录] 主题: {}\n", self.config.topic);
        s.push_str(&format!(
            "参与者: {}\n",
            self.config.participants.join(", ")
        ));
        s.push_str(&format!(
            "轮次: {}/{}\n",
            self.current_round.min(self.config.max_rounds),
            self.config.max_rounds
        ));
        s.push_str("---\n");
        for turn in &self.turns {
            let prefix = if turn.is_intervention {
                "[用户介入] "
            } else {
                ""
            };
            s.push_str(&format!(
                "{}【{}】: {}\n",
                prefix, turn.speaker, turn.content
            ));
        }
        if let Some(ref reason) = self.end_reason {
            s.push_str(&format!("---\n会议结束: {}\n", reason));
        }
        s
    }
}

fn council_path(data_root: &Path, character_id: &str) -> PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("council_session.json")
}

/// 读取会议状态。
pub fn load_council(
    data_root: &Path,
    character_id: &str,
) -> Result<Option<CouncilSession>, AirpError> {
    let path = council_path(data_root, character_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Some(serde_json::from_str(&content)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 保存会议状态。
pub fn save_council(
    data_root: &Path,
    character_id: &str,
    session: &CouncilSession,
) -> Result<(), AirpError> {
    let path = council_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(session)
        .map_err(|e| AirpError::Internal(format!("council serialize: {e}")))?;
    crate::data_dir::replace_file(&path, json.as_bytes())?;
    Ok(())
}

/// 清除会议状态。
pub fn clear_council(data_root: &Path, character_id: &str) -> Result<(), AirpError> {
    let path = council_path(data_root, character_id);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// 会议系统 prompt 模板。
pub fn council_speaker_prompt(topic: &str, speaker: &str, prior_turns: &[CouncilTurn]) -> String {
    let mut prompt = format!(
        "你正在参加一场会议/辩论。\n主题: {}\n你的身份: {}\n\n",
        topic, speaker
    );
    if !prior_turns.is_empty() {
        prompt.push_str("之前的发言：\n");
        // 取最近 10 条，但按时间顺序（oldest-to-newest）输出
        let recent: Vec<&CouncilTurn> = prior_turns.iter().rev().take(10).collect();
        for turn in recent.iter().rev() {
            prompt.push_str(&format!("【{}】: {}\n", turn.speaker, turn.content));
        }
        prompt.push('\n');
    }
    prompt.push_str("请就主题发表你的看法。保持角色特征，简洁有力（2-4句话）。");
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_config() -> CouncilConfig {
        CouncilConfig {
            topic: "是否应该探索北方废墟".to_string(),
            participants: vec!["alice".into(), "bob".into(), "carol".into()],
            max_rounds: 2,
            order: SpeakingOrder::RoundRobin,
            allow_intervention: true,
            max_tokens_per_turn: 200,
        }
    }

    #[test]
    fn council_session_round_robin() {
        let mut session = CouncilSession::new(make_config());
        assert_eq!(session.current_speaker(), Some("alice"));
        session.add_turn("alice", "我认为应该去", false).unwrap();
        session.advance_speaker();
        assert_eq!(session.current_speaker(), Some("bob"));
        session.add_turn("bob", "太危险了", false).unwrap();
        session.advance_speaker();
        assert_eq!(session.current_speaker(), Some("carol"));
        session.add_turn("carol", "我同意 alice", false).unwrap();
        let new_round = session.advance_speaker();
        assert!(new_round); // 进入第二轮
        assert_eq!(session.current_round, 2);
        assert_eq!(session.current_speaker(), Some("alice"));
    }

    #[test]
    fn council_finishes_after_max_rounds() {
        let mut session = CouncilSession::new(make_config());
        // Round 1
        for _ in 0..3 {
            session.advance_speaker();
        }
        assert_eq!(session.current_round, 2);
        // Round 2
        for _ in 0..3 {
            session.advance_speaker();
        }
        assert!(session.finished);
        assert_eq!(session.end_reason, Some("达到最大轮次".to_string()));
    }

    #[test]
    fn council_summary_format() {
        let mut session = CouncilSession::new(make_config());
        session.add_turn("alice", "应该去", false).unwrap();
        session.add_turn("User", "我支持探索", true).unwrap();
        let summary = session.summary();
        assert!(summary.contains("会议记录"));
        assert!(summary.contains("是否应该探索北方废墟"));
        assert!(summary.contains("【alice】: 应该去"));
        assert!(summary.contains("[用户介入] 【User】"));
    }

    #[test]
    fn council_rejects_turn_after_finish() {
        let mut session = CouncilSession::new(make_config());
        session.finish("测试结束");
        assert!(session.add_turn("alice", "test", false).is_err());
    }

    #[test]
    fn council_rejects_intervention_when_disabled() {
        let mut config = make_config();
        config.allow_intervention = false;
        let mut session = CouncilSession::new(config);
        assert!(session.add_turn("alice", "test", true).is_err());
    }

    #[test]
    fn council_rejects_wrong_speaker() {
        let mut session = CouncilSession::new(make_config());
        // 当前发言者是 alice，尝试用 bob 发非介入发言
        assert!(session.add_turn("bob", "test", false).is_err());
    }

    #[test]
    fn council_validates_config() {
        let config = CouncilConfig {
            topic: "test".to_string(),
            participants: vec![],
            max_rounds: 3,
            ..Default::default()
        };
        let session = CouncilSession::new(config);
        assert!(session.finished);
        assert!(session.end_reason.is_some());

        let config = CouncilConfig {
            topic: "test".to_string(),
            participants: vec!["a".into()],
            max_rounds: 0,
            ..Default::default()
        };
        let session = CouncilSession::new(config);
        assert!(session.finished);
    }

    #[test]
    fn council_random_order_generates_schedule() {
        let config = CouncilConfig {
            topic: "测试主题".to_string(),
            participants: vec!["a".into(), "b".into(), "c".into()],
            max_rounds: 2,
            order: SpeakingOrder::Random,
            allow_intervention: true,
            max_tokens_per_turn: 200,
        };
        let session = CouncilSession::new(config);
        assert!(!session.speaker_schedule.is_empty());
        // schedule 长度应等于 participants 长度
        assert_eq!(session.speaker_schedule.len(), 3);
    }

    #[test]
    fn council_persistence_roundtrip() {
        let tmp = tempdir().unwrap();
        let session = CouncilSession::new(make_config());
        save_council(tmp.path(), "hero", &session).unwrap();
        let loaded = load_council(tmp.path(), "hero").unwrap().unwrap();
        assert_eq!(loaded.config.topic, "是否应该探索北方废墟");
        assert_eq!(loaded.config.participants.len(), 3);
    }

    #[test]
    fn council_clear() {
        let tmp = tempdir().unwrap();
        let session = CouncilSession::new(make_config());
        save_council(tmp.path(), "hero", &session).unwrap();
        clear_council(tmp.path(), "hero").unwrap();
        assert!(load_council(tmp.path(), "hero").unwrap().is_none());
    }
}
