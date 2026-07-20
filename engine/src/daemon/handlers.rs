//! HTTP handler functions for the daemon API.
//!
//! #155 PR 6 之后：本文件是 handler facade。sessions / personas / chat / agent
//! / settings / presets / scenes / models / characters / state / lorebook 十一个
//! family 已拆入 `handlers/` 子模块，facade 经 `pub(super) use` re-export 保持
//! `daemon/mod.rs` 的 `use handlers::{...}` 调用路径不变。
//!
//! 本文件不再持有 handler 实现；`pub(crate)` 共享函数（`import_card_to_disk` /
//! `extract_card_assets`）经 `pub(crate) use` 转发，供未来 MCP tool 复用。

mod agent;
mod characters;
mod chat;
mod lorebook;
mod models;
mod personas;
mod presets;
mod scenes;
mod sessions;
mod settings;
mod state;

// #155 PR 4/5/6：re-export moved handlers 保持 `daemon/mod.rs` 的 `use handlers::{...}` 不变。
pub(super) use agent::{agent_run, list_agent_tools};
pub(super) use characters::{
    delete_character_endpoint, get_character_card, import_character, list_characters,
    reextract_character_assets, update_character_card,
};
pub(super) use chat::{
    chat_completion, continue_chat, delete_message, get_chat_history, preview_chat_assembly,
    regen_chat, rollback_chat, swipe_chat,
};
pub(super) use lorebook::{get_character_lorebook, update_character_lorebook};
pub(super) use models::list_models;
pub(super) use personas::{
    bind_persona_endpoint, create_persona_endpoint, delete_persona_multi_endpoint,
    get_effective_persona_endpoint, get_persona_endpoint, get_persona_multi_endpoint,
    list_personas_endpoint, unbind_persona_endpoint, update_persona_endpoint,
    update_persona_multi_endpoint,
};
pub(super) use presets::{get_preset_endpoint, import_preset_endpoint, list_presets_endpoint};
pub(super) use scenes::{
    add_scene_character_endpoint, create_scene_endpoint, get_scene_endpoint, list_scenes_endpoint,
};
pub(super) use sessions::{
    create_session_endpoint, delete_session_endpoint, list_sessions_endpoint,
};
pub(super) use settings::{get_settings, update_settings};
pub(super) use state::{
    get_character_avatar, get_character_state, get_character_state_history,
    get_character_state_schema,
};

// M_MCP MCP-2：角色卡导入的 `pub(crate)` 共享实现，供未来 daemon HTTP handler 与
// MCP tool 复用。facade 转发符号路径，保持 `crate::daemon::handlers::import_card_to_disk`
// 调用入口不变。当前 crate 内尚无外部调用方（grep 确认），re-export 仅为保留契约，
// 故本地允许 unused_imports；待 MCP tool 接入后移除 allow。
#[allow(unused_imports)]
pub(crate) use characters::{extract_card_assets, import_card_to_disk};
