//! Search family: FTS5 历史检索（4.3）。
//!
//! 工具清单：
//! - `session_search`：全文搜索历史对话（readonly）

use super::params::required_character_id;
use super::*;
use crate::daemon::DaemonState;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// `session_search`：全文搜索历史对话。
struct SessionSearchTool {
    state: Arc<DaemonState>,
}

impl Tool for SessionSearchTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "session_search",
            description: "Search through all historical conversations using full-text search.",
            side_effect: ToolSideEffect::Readonly,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let cid = required_character_id(&params)?;
            let query = params
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("query is required".to_string()))?;
            let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            let query = query.to_string();

            let results = tokio::task::spawn_blocking(move || {
                state
                    .fts
                    .search_history(&state.data_root, &cid, &query, limit)
            })
            .await
            .map_err(|error| AirpError::Internal(format!("FTS search task failed: {error}")))??;

            let out: Vec<Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "session_id": r.session_id,
                        "role": r.role,
                        "content": r.content,
                        "timestamp": r.timestamp,
                        "rank": r.rank
                    })
                })
                .collect();

            Ok(ToolResult {
                output: Value::Array(out),
                dry_run: false,
            })
        })
    }
}

pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(SessionSearchTool { state }))
        .expect(COLLISION);
}
