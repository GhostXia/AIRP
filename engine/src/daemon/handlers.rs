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
mod backups;
mod card_diff;
mod character_templates;
mod characters;
mod chat;
mod conversations;
mod dialogue_gen;
mod image_gen;
mod lorebook;
mod memory;
mod models;
mod personas;
mod plot;
mod plugin_tools;
mod presets;
mod provider_routing;
mod scenes;
mod search;
mod session_recovery;
mod sessions;
mod settings;
mod state;
mod style;
mod surfaces;
mod timeline_export;
mod ui_intents;
mod workspace;
mod worldbook_graph;

// #155 PR 4/5/6：re-export moved handlers 保持 `daemon/mod.rs` 的 `use handlers::{...}` 不变。
pub(super) use agent::{agent_run, list_agent_tools};
// #342 E-P2-1：backup / restore / 可恢复删除 闭环。
pub(super) use backups::{
    create_backup_endpoint, delete_backup_endpoint, get_backup_endpoint, list_backups_endpoint,
    restore_backup_endpoint, verify_backup_endpoint,
};
pub(super) use card_diff::{
    diff_character_revisions_endpoint, get_character_revision_endpoint,
    list_character_revisions_endpoint,
};
pub(super) use character_templates::{
    get_template_endpoint, instantiate_template_endpoint, list_templates_endpoint,
};
pub(super) use characters::{
    delete_character_endpoint, get_character_card, import_character, list_characters,
    reextract_character_assets, update_character_card,
};
pub(super) use chat::{
    cancel_chat_generation, chat_completion, continue_chat, delete_message, edit_message,
    get_chat_history, get_chat_session_state, preview_chat_assembly, regen_chat, rollback_chat,
    swipe_chat, switch_branch,
};
pub(super) use conversations::{
    append_conversation_event_endpoint, cancel_conversation_turn_endpoint,
    create_conversation_endpoint, create_scene_conversation_endpoint,
    execute_conversation_migration_endpoint, execute_conversation_turn_endpoint,
    get_conversation_capabilities_endpoint, get_conversation_endpoint,
    get_conversation_events_endpoint, get_conversation_migration_export_endpoint,
    get_conversation_turn_endpoint, get_conversation_turn_observability_endpoint,
    list_conversation_policies_endpoint, list_conversations_endpoint,
    plan_conversation_migration_endpoint, rollback_conversation_migration_endpoint,
};
pub(super) use dialogue_gen::generate_dialogue_examples_endpoint;
pub(super) use image_gen::{
    generate_image_endpoint, list_images_endpoint, serve_image_endpoint,
    serve_session_image_endpoint,
};
pub(super) use lorebook::{get_character_lorebook, update_character_lorebook};
pub(super) use memory::{
    get_resident_memory, get_user_model, update_resident_memory, update_user_model,
};
pub(super) use models::list_models;
pub(super) use personas::{
    bind_persona_endpoint, create_persona_endpoint, delete_persona_multi_endpoint,
    get_effective_persona_endpoint, get_persona_endpoint, get_persona_multi_endpoint,
    list_personas_endpoint, unbind_persona_endpoint, update_persona_endpoint,
    update_persona_multi_endpoint,
};
pub(super) use plot::{get_plot_arc, update_plot_arc};
pub(super) use plugin_tools::{
    delete_plugin_tool_endpoint, list_plugin_tools_endpoint, test_plugin_tool_endpoint,
    upsert_plugin_tool_endpoint,
};
pub(super) use presets::{get_preset_endpoint, import_preset_endpoint, list_presets_endpoint};
pub(super) use provider_routing::{
    get_routing_endpoint, list_providers_endpoint, resolve_provider_endpoint,
    update_providers_endpoint, update_routing_endpoint,
};
pub(super) use scenes::{
    add_scene_character_endpoint, create_scene_endpoint, get_scene_endpoint, list_scenes_endpoint,
};
pub(super) use search::chat_search;
// BUG-2 缓解切片：会话锁死的用户侧恢复端点（additive，不改既有 handler）。
pub(super) use session_recovery::recover_chat_session;
pub(super) use sessions::{
    create_session_endpoint, delete_session_endpoint, list_sessions_endpoint,
};
pub(super) use settings::{get_settings, update_settings};
pub(super) use state::{
    get_character_avatar, get_character_state, get_character_state_history,
    get_character_state_schema, get_world_events,
};
pub(super) use style::{
    get_drift, get_style_profile, list_style_profiles, rollback_drift, style_learn, style_review,
    update_drift,
};
pub(super) use surfaces::{get_surface_events, get_surface_snapshot};
pub(super) use timeline_export::{export_session_timeline_endpoint, get_session_timeline_endpoint};
pub(super) use ui_intents::dispatch_ui_intent;
pub(super) use workspace::{
    apply_workspace_migration_endpoint, dry_run_workspace_migration_endpoint,
    export_workspace_endpoint, get_workspace_endpoint, get_workspace_history_endpoint,
    rollback_workspace_endpoint, rollback_workspace_migration_endpoint, workspace_command_endpoint,
    WORKSPACE_HTTP_MAX_BODY_BYTES, WORKSPACE_MIGRATION_HTTP_MAX_BODY_BYTES,
};
pub(super) use worldbook_graph::get_lorebook_graph_endpoint;

// M_MCP MCP-2：角色卡导入的 `pub(crate)` 共享实现，供未来 daemon HTTP handler 与
// MCP tool 复用。facade 转发符号路径，保持 `crate::daemon::handlers::import_card_to_disk`
// 调用入口不变。当前 crate 内尚无外部调用方（grep 确认），re-export 仅为保留契约，
// 故本地允许 unused_imports；待 MCP tool 接入后移除 allow。
#[allow(unused_imports)]
pub(crate) use characters::{extract_card_assets, import_card_to_disk};
