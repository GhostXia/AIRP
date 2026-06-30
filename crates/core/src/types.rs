//! 类型安全的标识符 newtype。M5.0a 引入，目的是把"经过 `validate_id_segment`
//! 校验"从运行时惯例提升为编译期保证：一旦你拿到 `CharacterId`，它就一定是合法
//! 的角色 ID，不会包含路径遍历字符或空字节。
//!
//! 反序列化路径会自动调用校验：API 边界（`ChatCompletionRequest` 等）从客户端
//! 接收原始字符串，serde 在 deserialize 时把它包装为 newtype，若校验失败则直接
//! 返回 400，无需在 handler 里重复手写 `validate_id_segment`。

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::data_dir::validate_id_segment;
use crate::error::AirpError;

/// `data/characters/{id}/` 下的角色 ID。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterId(String);

impl CharacterId {
    /// 构造时校验。校验失败返回 `AirpError::BadRequest`。
    pub fn new(s: impl Into<String>) -> Result<Self, AirpError> {
        let s = s.into();
        validate_id_segment(&s)?;
        Ok(Self(s))
    }

    /// 返回内部字符串的不可变引用。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 消耗 newtype，取回内部字符串。
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for CharacterId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CharacterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for CharacterId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for CharacterId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        CharacterId::new(s).map_err(serde::de::Error::custom)
    }
}

/// `data/presets/{id}.json` 下的预设 ID。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresetId(String);

impl PresetId {
    /// 构造时校验。校验失败返回 `AirpError::BadRequest`。
    pub fn new(s: impl Into<String>) -> Result<Self, AirpError> {
        let s = s.into();
        validate_id_segment(&s)?;
        Ok(Self(s))
    }

    /// 返回内部字符串的不可变引用。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 消耗 newtype，取回内部字符串。
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for PresetId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PresetId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for PresetId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        PresetId::new(s).map_err(serde::de::Error::custom)
    }
}

/// AUDIT-2: `data/scenes/{id}/` 下的场景 ID。M_MS 多角色场景使用。
///
/// 与 [`CharacterId`] / [`PresetId`] 同构：构造时调 `validate_id_segment`，
/// serde 反序列化路径自动校验，所以 axum / MCP 工具收到的 `SceneId` 一定合法。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SceneId(String);

impl SceneId {
    /// 构造时校验。校验失败返回 `AirpError::BadRequest`。
    pub fn new(s: impl Into<String>) -> Result<Self, AirpError> {
        let s = s.into();
        validate_id_segment(&s)?;
        Ok(Self(s))
    }

    /// 返回内部字符串的不可变引用。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 消耗 newtype，取回内部字符串。
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for SceneId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SceneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for SceneId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for SceneId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        SceneId::new(s).map_err(serde::de::Error::custom)
    }
}

/// M_UP / P1: `data/users/{user_id}/` 下的用户 ID。
///
/// 与 `CharacterId` / `PresetId` / `SceneId` 同构：构造时 `validate_id_segment`，
/// serde 反序列化路径自动校验。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(String);

impl UserId {
    pub fn new(s: impl Into<String>) -> Result<Self, AirpError> {
        let s = s.into();
        validate_id_segment(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for UserId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for UserId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for UserId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        UserId::new(s).map_err(serde::de::Error::custom)
    }
}

/// 会话 ID（M5.1 多 session 时使用）。基于 UUID v4，无需路径段校验。
///
/// 默认构造一个全新的 v4 UUID；`parse` 接受标准 UUID 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(uuid::Uuid);

impl SessionId {
    /// 生成一个新 v4 UUID。
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// 从字符串解析（标准 UUID 格式）。
    pub fn parse(s: &str) -> Result<Self, AirpError> {
        uuid::Uuid::parse_str(s)
            .map(Self)
            .map_err(|e| AirpError::BadRequest(format!("非法 SessionId: {}", e)))
    }

    /// 取出内部 UUID 副本。
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for SessionId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // 序列化为标准 UUID 字符串
        self.0.to_string().serialize(s)
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        SessionId::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_id_valid() {
        let id = CharacterId::new("alice").unwrap();
        assert_eq!(id.as_str(), "alice");
        let id = CharacterId::new("艾米丽").unwrap();
        assert_eq!(id.as_str(), "艾米丽");
    }

    #[test]
    fn character_id_rejects_traversal() {
        assert!(CharacterId::new("..").is_err());
        assert!(CharacterId::new("a/b").is_err());
        assert!(CharacterId::new("").is_err());
        assert!(CharacterId::new(".hidden").is_err());
    }

    #[test]
    fn character_id_serde_roundtrip() {
        let id = CharacterId::new("bob").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"bob\"");
        let back: CharacterId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "bob");
    }

    #[test]
    fn character_id_serde_rejects_invalid() {
        // 反序列化路径校验：非法字符串应在 deserialize 时直接拒绝
        let res: Result<CharacterId, _> = serde_json::from_str("\"../bad\"");
        assert!(res.is_err());
    }

    #[test]
    fn preset_id_basic() {
        assert!(PresetId::new("default").is_ok());
        assert!(PresetId::new("../etc").is_err());
        let p = PresetId::new("test_preset").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: PresetId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "test_preset");
    }

    #[test]
    fn session_id_uniqueness() {
        let a = SessionId::new();
        let b = SessionId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn session_id_parse_valid() {
        let s = "550e8400-e29b-41d4-a716-446655440000";
        let sid = SessionId::parse(s).unwrap();
        assert_eq!(sid.to_string(), s);
    }

    #[test]
    fn session_id_parse_invalid() {
        assert!(SessionId::parse("not-a-uuid").is_err());
    }

    #[test]
    fn session_id_serde_roundtrip() {
        let sid = SessionId::new();
        let json = serde_json::to_string(&sid).unwrap();
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(sid, back);
    }

    // AUDIT-2: SceneId newtype tests
    #[test]
    fn scene_id_valid() {
        let id = SceneId::new("dawn_tavern").unwrap();
        assert_eq!(id.as_str(), "dawn_tavern");
    }

    #[test]
    fn scene_id_rejects_traversal() {
        assert!(SceneId::new("../etc").is_err());
        assert!(SceneId::new("a/b").is_err());
        assert!(SceneId::new("").is_err());
        assert!(SceneId::new(".hidden").is_err());
    }

    #[test]
    fn scene_id_serde_roundtrip() {
        let id = SceneId::new("scene_01").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"scene_01\"");
        let back: SceneId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "scene_01");
    }

    #[test]
    fn scene_id_serde_rejects_invalid() {
        let res: Result<SceneId, _> = serde_json::from_str("\"../bad\"");
        assert!(res.is_err());
    }

    // M_UP / P1: UserId newtype tests
    #[test]
    fn user_id_valid() {
        let id = UserId::new("alice").unwrap();
        assert_eq!(id.as_str(), "alice");
    }

    #[test]
    fn user_id_rejects_traversal() {
        assert!(UserId::new("../etc").is_err());
        assert!(UserId::new("a/b").is_err());
        assert!(UserId::new("").is_err());
        assert!(UserId::new(".hidden").is_err());
    }

    #[test]
    fn user_id_serde_roundtrip() {
        let id = UserId::new("alice").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"alice\"");
        let back: UserId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "alice");
    }

    #[test]
    fn user_id_serde_rejects_invalid() {
        let res: Result<UserId, _> = serde_json::from_str("\"../bad\"");
        assert!(res.is_err());
    }
}
