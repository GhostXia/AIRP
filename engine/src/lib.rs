//! # AIRP-Core: 纯 Agent 端 — 自调 LLM 的流式 RP 后端
//!
//! 角色扮演（RP）后端：在客户端与上游 OpenAI / Anthropic 兼容 API 之间插入一层
//! 守护进程，负责注入角色卡 / 世界书 / 预设 / 历史卷上下文，并对流式响应进行
//! FSM 过滤与 XML 解包（`immersive` / `<action>` 分离）。
//!
//! **乐高式定位：** Core 只做"自调 LLM 的流式 RP 后端"一件事，不耦合生态其他块。
//! - 纯 MCP 数据工具面 → 见 [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server)
//! - 协议桥 / AgentBus → 见 [AIRP-Gateway](https://github.com/GhostXia/AIRP-Gateway)
//! - UI + State Protocol 契约 → 见 [AIRP-State-Protocol](https://github.com/GhostXia/AIRP-State-Protocol)
//!
//! ## 公开模块（外部消费者可直接 use）
//! - [`config`] · [`daemon`] · [`chat_pipeline`] · [`data_dir`] · [`png_parser`]
//! - [`adapter`] · [`chat_store`] · [`error`] · [`orchestrator`] · [`scene`]
//!
//! ## 内部模块（pub(crate)：实现细节，不对外保证 API 稳定）
//! - `fsm` · `xml_unpacker` · `auto_converter`
//! - `volume_store` · `volume_manager` · `index_parser`
//!
//! 设计概览参见 `AGENTS.md`。

pub mod adapter;
pub mod agent;
pub mod chat_pipeline;
pub mod chat_store;
pub mod config;
mod context_limit;
pub mod daemon;
pub mod data_dir;
pub mod decompose;
pub mod domain;
pub mod error;
pub mod memory;
pub mod orchestrator;
pub mod outbound;
pub mod png_parser;
pub mod quota;
pub mod scene;
pub(crate) mod secret_store;
pub mod types;
pub mod ulid;

// M0 F-50 / 6.0n：实现细节模块收紧为 pub(crate)，仅 crate 内部互调。
// 这些模块不被 main.rs / 外部消费者直接引用，未来重构无 API 兼容包袱。
// 注：M4.5 完成后 `fsm` 不再被 main.rs 直接引用，已降为 pub(crate)。
pub(crate) mod auto_converter;
pub(crate) mod fsm;
pub(crate) mod index_parser;
pub(crate) mod preset_regex;
// #115 Phase 2a：统一 revision/provenance 底层模块。
// pub(crate)：实现细节，不对外保证 API 稳定；Phase 2b 起被各 asset service 引用。
// 暂未接入 asset service，dead_code 在 Phase 2b 接入后移除 allow。
#[allow(dead_code)]
pub(crate) mod revision;
pub(crate) mod volume_manager;
pub(crate) mod volume_store;
pub(crate) mod xml_unpacker;
