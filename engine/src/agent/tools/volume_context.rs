//! Volume & context bundle family built-in Agent tools.
//!
//! 设计纪律（#155 PR 3）：
//! - 2 个 tool struct 保持私有；对 facade 只暴露 [`register`]。
//! - 不改任何 `ToolMeta` 文案、side_effect 或入参/出参形状。
//! - 共享 helper 走 [`super::params`]，不重复实现。
//!
//! 工具清单：
//! - `seal_volume`：把当前会话记忆摘要到下一个 volume 并清空 current.md
//!   （destructive，默认 dry-run）
//! - `export_context_bundle`：写一个 bounded generic-Markdown context bundle
//!   供 isolated subagent 使用（mutate）

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::data_dir;
use crate::domain::LorebookService;
use crate::error::AirpError;
use crate::types::PresetId;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// `seal_volume`：把当前会话记忆摘要到下一个 volume 并清空 current.md。
/// **destructive** → 默认 dry-run，未 confirm 只回 preview。
struct SealVolumeTool {
    state: Arc<DaemonState>,
}

impl Tool for SealVolumeTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "seal_volume",
            description: "Summarize current session memory into the next volume and clear current.md. Destructive — dry-run unless confirmed.",
            side_effect: ToolSideEffect::Destructive,
        }
    }

    fn call(
        &self,
        params: Value,
        confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            data_dir::get_character_card(&state.data_root, &character)?;
            let session_id = optional_session_id(&params)?;
            let session_dir = data_dir::resolve_session_dir(
                &state.data_root,
                character.as_str(),
                session_id.as_ref(),
            )?;
            let current = crate::volume_store::read_current(&session_dir)?;
            let next_volume = crate::volume_store::next_volume_number(&session_dir);
            let preview = serde_json::json!({
                "character_id": character.as_str(),
                "session_id": session_id.as_ref().map(ToString::to_string),
                "next_volume": next_volume,
                "current_bytes": current.len(),
                "current_tokens_estimated": crate::volume_store::estimate_tokens(&current),
            });
            if current.trim().is_empty() {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "sealed": false,
                        "reason": "current memory is empty",
                        "preview": preview,
                    }),
                    dry_run: true,
                });
            }
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "action": "seal_volume",
                        "preview": preview,
                        "requires": "confirm=true",
                    }),
                    dry_run: true,
                });
            }
            let (provider, params) = {
                let config = state
                    .config
                    .read()
                    .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?;
                let volume = &config.volume_config;
                (
                    Arc::new(crate::adapter::ProviderConfig {
                        provider: config.provider.clone(),
                        endpoint: config.endpoint.clone(),
                        api_key: config.api_key.clone(),
                    }),
                    crate::adapter::GenerationParams {
                        model: volume
                            .seal_model
                            .clone()
                            .unwrap_or_else(|| config.model.clone()),
                        temperature: Some(volume.seal_temperature),
                        max_tokens: None,
                    },
                )
            };
            let written_volume = crate::volume_manager::run_seal_flow(
                &state.http_client,
                &session_dir,
                provider,
                params,
            )
            .await?
            .ok_or_else(|| {
                AirpError::Volume("current memory became empty before sealing".into())
            })?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "sealed": true,
                    "character_id": character.as_str(),
                    "session_id": session_id.as_ref().map(ToString::to_string),
                    "volume": written_volume,
                    "file": format!("volumes/vol_{written_volume:03}.md"),
                }),
                dry_run: false,
            })
        })
    }
}

/// `export_context_bundle`：写一个 bounded generic-Markdown context bundle
/// 供 isolated subagent 使用。mutate。
struct ExportContextBundleTool {
    state: Arc<DaemonState>,
}

impl Tool for ExportContextBundleTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "export_context_bundle",
            description: "Write a bounded generic-Markdown context bundle for an isolated subagent under the AIRP data root.",
            side_effect: ToolSideEffect::Mutate,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let character = required_character_id(&params)?;
            let preset_id = params
                .get("preset_id")
                .and_then(Value::as_str)
                .map(PresetId::new)
                .transpose()?;
            let include_lorebook = params
                .get("include_lorebook")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let thinking_mode = params
                .get("thinking_mode_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty());

            let card = data_dir::get_character_card(&state.data_root, &character)?;
            let raw_card = data_dir::read_character_card_text(&state.data_root, &character)?;
            let normalized = crate::orchestrator::card::normalize_v1_to_v2(&raw_card);
            let orchestrator = crate::orchestrator::Orchestrator::new(Some(&normalized), None)?;
            let character_name = card
                .get("data")
                .and_then(|data| data.get("name"))
                .or_else(|| card.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(character.as_str());

            let stable_prompt =
                orchestrator.build_system_prompt("User", &std::collections::HashMap::new(), "");
            let mut context = format!(
                "# RP Context Bundle: {character_name}\n\n> Feed this to an ISOLATED subagent as its system context. A fresh context lets the persona dominate instead of competing with the orchestrator's coding register. Generic Markdown only; wrap it in the host's own skill shape when needed.\n\n---\n\n"
            );
            if let Some(text) = thinking_mode {
                context.push_str("## Thinking mode (verbatim; AIRP does not interpret)\n");
                context.push_str(text);
                context.push_str("\n\n");
            }
            context.push_str("## Stable character context\n");
            context.push_str(&stable_prompt);

            if include_lorebook {
                let lorebook = LorebookService::new(&state.data_root).read(&character)?;
                let mut enabled: Vec<_> = lorebook
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.enabled.unwrap_or(true))
                    .collect();
                enabled.sort_by_key(|(index, entry)| {
                    (std::cmp::Reverse(entry.priority.unwrap_or(10)), *index)
                });
                if !enabled.is_empty() {
                    context.push_str("\n\n## World knowledge\n");
                    for (_, entry) in enabled {
                        context.push('\n');
                        context.push_str(&entry.content);
                        context.push('\n');
                    }
                }
            }

            let state_path =
                data_dir::char_state_dir(&state.data_root, character.as_str()).join("live.json");
            let live_state = match tokio::fs::read_to_string(&state_path).await {
                Ok(state) => Some(state),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };

            let bundle_dir =
                data_dir::ensure_context_bundle_dir(&state.data_root, character.as_str())?;
            for stale in ["preset_raw.json", "extensions.json"] {
                let path = bundle_dir.join(stale);
                if tokio::fs::try_exists(&path).await? {
                    tokio::fs::remove_file(path).await?;
                }
            }

            let mut files = vec!["context.md".to_string()];
            if let Some(preset) = preset_id.as_ref() {
                let raw_path = data_dir::preset_json_path(&state.data_root, preset.as_str());
                if !tokio::fs::try_exists(&raw_path).await? {
                    return Err(AirpError::NotFound(format!(
                        "preset {} has no preset.json",
                        preset
                    )));
                }
                tokio::fs::copy(raw_path, bundle_dir.join("preset_raw.json")).await?;
                files.push("preset_raw.json".to_string());
                context.push_str("\n> `preset_raw.json` is verbatim passthrough; AIRP does not interpret its prompts.\n");
            }

            let extensions = card
                .get("data")
                .and_then(|data| data.get("extensions"))
                .or_else(|| card.get("extensions"));
            if let Some(extensions) = extensions.filter(|value| {
                !value.is_null() && value.as_object().is_none_or(|object| !object.is_empty())
            }) {
                tokio::fs::write(
                    bundle_dir.join("extensions.json"),
                    serde_json::to_vec_pretty(extensions)?,
                )
                .await?;
                files.push("extensions.json".to_string());
                context.push_str("\n> `extensions.json` is raw bundled-card passthrough; AIRP does not interpret it.\n");
            }

            if let Some(live_state) = live_state {
                context.push_str(
                    "\n\n## Current state (volatile; keep after stable context)\n```json\n",
                );
                context.push_str(live_state.trim());
                context.push_str("\n```\n");
            }

            let context_bytes = context.len();
            let stored_context = crate::context_limit::truncate_for_context(&context);
            tokio::fs::write(bundle_dir.join("context.md"), &stored_context).await?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "character_id": character.as_str(),
                    "bundle_path": format!("exports/context-bundles/{}", character.as_str()),
                    "files": files,
                    "context_bytes": context_bytes,
                    "stored_bytes": stored_context.len(),
                    "truncated": context_bytes > crate::context_limit::max_read_bytes(),
                }),
                dry_run: false,
            })
        })
    }
}

/// 由 facade `default_registry` 集中调用，注册本 family 全部 2 个工具。
pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(SealVolumeTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ExportContextBundleTool { state }))
        .expect(COLLISION);
}
