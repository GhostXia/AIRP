//! Phase 4.5: 剧情时间线导出 HTTP handlers。
//!
//! 端点：
//! - `GET /v1/sessions/:character_id/:session_id/timeline` — 返回结构化 JSON 时间线
//! - `GET /v1/sessions/:character_id/:session_id/timeline/export?format=markdown|html|json`
//!   返回文件下载（Content-Disposition: attachment）
//!
//! 复用 `crate::timeline_export` 业务逻辑；handler 只做参数解析、DaemonState 读取
//! 与响应包装。读取 character card 的 name 字段作为角色名快照。

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::timeline_export::{build_timeline, to_html, to_markdown, ExportFormat, TimelineExport};
use crate::types::{CharacterId, SessionId};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// `GET /v1/sessions/:character_id/:session_id/timeline`
///
/// 返回结构化 JSON 时间线数据（含元数据、entries、pending_events）。
pub(in crate::daemon) async fn get_session_timeline_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // R1: 把同步 fs + 解析 + 排序放到 spawn_blocking，避免阻塞 axum runtime。
    //     timeline export 会读 chat_log.jsonl 全量 + world_events.json + world_clock.json，
    //     长会话可达到几 MB，反序列化 + 排序在繁忙 daemon 上会阻塞其它请求。
    // R2: TimelineExport 已实现 Serialize，直接交给 Json，不再绕一层 to_value。
    let state = state.clone();
    match tokio::task::spawn_blocking(move || {
        run_build_timeline_sync(&state, &character_id, &session_id)
    })
    .await
    {
        Ok(Ok(timeline)) => (StatusCode::OK, Json(timeline)).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(join_err) => {
            AirpError::Internal(format!("timeline build task join failed: {}", join_err))
                .into_response()
        }
    }
}

/// `GET /v1/sessions/:character_id/:session_id/timeline/export?format=markdown|html|json`
///
/// 返回文件下载。`format` 缺省为 `json`。
pub(in crate::daemon) async fn export_session_timeline_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, session_id)): Path<(String, String)>,
    Query(query): Query<ExportQuery>,
) -> impl IntoResponse {
    let format = query.format.unwrap_or_default();
    // 与 get_session_timeline_endpoint 同样把同步工作放到 spawn_blocking。
    let state = state.clone();
    match tokio::task::spawn_blocking(move || {
        run_build_timeline_sync(&state, &character_id, &session_id)
    })
    .await
    {
        Ok(Ok(timeline)) => render_export(&timeline, &format),
        Ok(Err(e)) => e.into_response(),
        Err(join_err) => {
            AirpError::Internal(format!("timeline export task join failed: {}", join_err))
                .into_response()
        }
    }
}

/// 导出格式查询参数。
#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    /// `format=markdown|html|json`，缺省 `json`。
    pub format: Option<ExportFormat>,
}

fn run_build_timeline_sync(
    state: &DaemonState,
    character_id: &str,
    session_id: &str,
) -> Result<TimelineExport, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sid = SessionId::parse(session_id)?;

    // 校验角色存在
    let exists = crate::data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }

    // 尝试读取 character card 的 name 字段（失败不阻塞导出，仅省略 name 快照）
    let character_name = read_character_name(&state.data_root, &cid).ok();

    build_timeline(&state.data_root, cid.as_str(), Some(&sid), character_name)
}

/// 读取 character card 的 `data.name` 或顶层 `name` 字段（兼容 v1/v2 卡）。
fn read_character_name(
    data_root: &std::path::Path,
    cid: &CharacterId,
) -> Result<String, AirpError> {
    let card = crate::data_dir::get_character_card(data_root, cid)?;
    // 兼容 v2 (data.name) 与 v1 (name)
    if let Some(name) = card
        .get("data")
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
    {
        return Ok(name.to_string());
    }
    if let Some(name) = card.get("name").and_then(|v| v.as_str()) {
        return Ok(name.to_string());
    }
    Err(AirpError::NotFound(format!(
        "character {} card has no name field",
        cid
    )))
}

/// 根据导出格式渲染响应（含 Content-Type + Content-Disposition）。
fn render_export(timeline: &TimelineExport, format: &ExportFormat) -> axum::response::Response {
    let (body, content_type, ext) = match format {
        ExportFormat::Json => match serde_json::to_vec_pretty(timeline) {
            Ok(bytes) => (bytes, "application/json; charset=utf-8", "json"),
            Err(e) => return AirpError::from(e).into_response(),
        },
        ExportFormat::Markdown => {
            let md = to_markdown(timeline);
            (md.into_bytes(), "text/markdown; charset=utf-8", "md")
        }
        ExportFormat::Html => {
            let html = to_html(timeline);
            (html.into_bytes(), "text/html; charset=utf-8", "html")
        }
    };

    let filename = format!(
        "timeline-{}-{}.{}",
        sanitize_filename(&timeline.character_id),
        sanitize_filename(&timeline.session_id),
        ext
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).expect("valid content type"),
    );
    // RFC 6266 + RFC 5987：非 ASCII 文件名用 filename*=UTF-8''<pct-encoded>
    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        sanitize_filename_ascii(&filename),
        pct_encode_utf8(&filename)
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }

    (StatusCode::OK, headers, body).into_response()
}

/// 把文件名中的非法字符替换为 `_`（保留中文等 Unicode 字符）。
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// 仅保留 ASCII 字母数字与少数安全字符，用于 `filename="..."` 字段（RFC 6266 token）。
fn sanitize_filename_ascii(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// RFC 5987 percent-encoding for `filename*=UTF-8''<encoded>`。
/// 仅对非 ALPHA / DIGIT / 安全字符做 %-encoding。
fn pct_encode_utf8(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_replaces_path_separators() {
        assert_eq!(sanitize_filename("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_filename("star*star"), "star_star");
        // 中文字符保留
        assert_eq!(sanitize_filename("爱丽丝"), "爱丽丝");
    }

    #[test]
    fn sanitize_filename_ascii_strips_non_ascii() {
        assert_eq!(sanitize_filename_ascii("alice-123.json"), "alice-123.json");
        // 中文字符应被剔除
        assert_eq!(sanitize_filename_ascii("爱丽丝"), "");
    }

    #[test]
    fn pct_encode_utf8_encodes_non_safe_bytes() {
        // 空格 -> %20
        assert_eq!(pct_encode_utf8("a b"), "a%20b");
        // ASCII alnum 不编码
        assert_eq!(pct_encode_utf8("abc123"), "abc123");
        // 中文字符 UTF-8 字节均编码
        let encoded = pct_encode_utf8("中");
        assert!(encoded.starts_with("%"));
        // "中" UTF-8 = E4 B8 AD
        assert_eq!(encoded, "%E4%B8%AD");
    }

    #[test]
    fn export_query_default_format_is_none() {
        // 空 query → format=None
        let q: ExportQuery = serde_json::from_str("{}").unwrap();
        assert!(q.format.is_none());
    }

    #[test]
    fn export_query_deserializes_format() {
        let q: ExportQuery = serde_json::from_str(r#"{"format":"markdown"}"#).unwrap();
        assert!(matches!(q.format, Some(ExportFormat::Markdown)));
    }
}
