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
}

impl CouncilSession {
    /// 创建新会议。
    pub fn new(config: CouncilConfig) -> Self {
        Self {
            config,
            current_round: 1,
            current_speaker_idx: 0,
            turns: Vec::new(),
            finished: false,
            end_reason: None,
        }
    }

    /// 获取当前发言者。
    pub fn current_speaker(&self) -> Option<&str> {
        if self.finished || self.config.participants.is_empty() {
            return None;
        }
        self.config.participants.get(self.current_speaker_idx).map(|s| s.as_str())
    }

    /// 推进到下一个发言者。返回是否进入了新轮次。
    pub fn advance_speaker(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.current_speaker_idx += 1;
        if self.current_speaker_idx >= self.config.participants.len() {
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

    /// 添加发言。
    pub fn add_turn(&mut self, speaker: &str, content: &str, is_intervention: bool) {
        self.turns.push(CouncilTurn {
            speaker: speaker.to_string(),
            content: content.to_string(),
            round: self.current_round,
            is_intervention,
        });
    }

    /// 结束会议。
    pub fn finish(&mut self, reason: &str) {
        self.finished = true;
        self.end_reason = Some(reason.to_string());
    }

    /// 生成会议摘要（用于注入到叙事上下文）。
    pub fn summary(&self) -> String {
        let mut s = format!("[会议记录] 主题: {}\n", self.config.topic);
        s.push_str(&format!("参与者: {}\n", self.config.participants.join(", ")));
        s.push_str(&format!("轮次: {}/{}\n", self.current_round.min(self.config.max_rounds), self.config.max_rounds));
        s.push_str("---\n");
        for turn in &self.turns {
            let prefix = if turn.is_intervention { "[用户介入] " } else { "" };
            s.push_str(&format!("{}【{}】: {}\n", prefix, turn.speaker, turn.content));
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
pub fn load_council(data_root: &Path, character_id: &str) -> Result<Option<CouncilSession>, AirpError> {
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
        for turn in prior_turns.iter().rev().take(10) {
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
        session.add_turn("alice", "我认为应该去", false);
        session.advance_speaker();
        assert_eq!(session.current_speaker(), Some("bob"));
        session.add_turn("bob", "太危险了", false);
        session.advance_speaker();
        assert_eq!(session.current_speaker(), Some("carol"));
        session.add_turn("carol", "我同意 alice", false);
        let new_round = session.advance_speaker();
        assert!(new_round); // 进入第二轮
        assert_eq!(session.current_round, 2);
        assert_eq!(session.current_speaker(), Some("alice"));
    }

    #[test]
    fn council_finishes_after_max_rounds() {
        let mut session = CouncilSession::new(make_config());
        // Round 1
        for _ in 0..3 { session.advance_speaker(); }
        assert_eq!(session.current_round, 2);
        // Round 2
        for _ in 0..3 { session.advance_speaker(); }
        assert!(session.finished);
        assert_eq!(session.end_reason, Some("达到最大轮次".to_string()));
    }

    #[test]
    fn council_summary_format() {
        let mut session = CouncilSession::new(make_config());
        session.add_turn("alice", "应该去", false);
        session.add_turn("User", "我支持探索", true);
        let summary = session.summary();
        assert!(summary.contains("会议记录"));
        assert!(summary.contains("是否应该探索北方废墟"));
        assert!(summary.contains("【alice】: 应该去"));
        assert!(summary.contains("[用户介入] 【User】"));
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
