// Tests for `agent::tools` — declared as `#[cfg(test)] mod tests;` in `tools.rs`.
//
// 本文件是 `tools::tests` 子模块的 hub：
// - `use super::*` 从 facade（`tools`）拉入 `default_registry` / `ToolRegistry` /
//   `EchoTool` / `AirpError` / `DaemonState` 等 public + private 项。
// - `make_state` fixture 为 `pub(super)`，对所有测试子模块可见，绝不外泄到 production。
// - `MAX_RECENT_CONTEXT` 从 `tools::session`（production）re-import，
//   供 `tests::session` 的边界断言使用。
// - PR 3（#155）后 state/lorebook/volume/context/analysis 工具已拆入各自 family
//   模块；对应测试按 family 分入 `tests/state_lorebook` / `tests/volume_context`
//   / `tests/analysis`。

use super::*;
use crate::adapter::{BackendEngine, Provider};
use crate::config::VolumeConfig;
use crate::daemon::MutableConfig;
use std::path::PathBuf;
use std::sync::Arc;

// 从 production `tools::session` re-import，供 `tests::session` 边界断言。
use super::session::MAX_RECENT_CONTEXT;

mod analysis;
mod character;
mod registry;
mod session;
mod state_lorebook;
mod state_preset;
mod volume_context;

/// 最小可运行 DaemonState，data_root 指向临时目录（照 chat_pipeline/tests 模板）。
pub(super) fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
    Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        settings_update: Default::default(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint: "https://example.test/v1/chat/completions".to_string(),
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: crate::quota::QuotaConfig::default(),
            deployment_mode: Default::default(),
            public_origin: None,
        }),
    })
}
