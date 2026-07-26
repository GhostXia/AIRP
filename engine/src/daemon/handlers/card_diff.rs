//! Phase 4.6: 角色卡版本对比 HTTP handlers。
//!
//! 端点：
//! - `GET /v1/characters/:character_id/revisions` — 列出角色所有可用 revision
//! - `GET /v1/characters/:character_id/revisions/:revision_id` — 获取单个 revision 的快照
//! - `GET /v1/characters/:character_id/revisions/diff?rev_a=1&rev_b=2&format=json|markdown|html`
//!   返回对比结果（结构化 JSON 或文件下载）
//!
//! 复用 `crate::card_diff` 业务逻辑；handler 只做参数解析、DaemonState 读取
//! 与响应包装。

use crate::card_diff::{
    build_card_diff, list_revisions, load_revision_snapshot, CardDiff, DiffExportFormat,
    RevisionSnapshot,
};
use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::CharacterId;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// `GET /v1/characters/:character_id/revisions`
///
/// 返回角色所有可用 revision 编号（升序）。
pub(in crate::daemon) async fn list_character_revisions_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> impl IntoResponse {
    match run_list_revisions(&state, &character_id).await {
        Ok(revisions) => {
            let body = RevisionsListResponse { revisions };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// `GET /v1/characters/:character_id/revisions/:revision_id`
///
/// 返回指定 revision 的元数据 + card.json 内容。
pub(in crate::daemon) async fn get_character_revision_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, revision_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match run_get_revision(&state, &character_id, &revision_id).await {
        Ok(snapshot) => match serde_json::to_value(&snapshot) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => AirpError::from(e).into_response(),
        },
        Err(e) => e.into_response(),
    }
}

/// `GET /v1/characters/:character_id/revisions/diff?rev_a=1&rev_b=2&format=json|markdown|html`
///
/// 返回对比结果。`format` 缺省为 `json`，返回结构化 JSON；
/// `markdown` / `html` 返回文件下载（Content-Disposition: attachment）。
pub(in crate::daemon) async fn diff_character_revisions_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Query(query): Query<DiffQuery>,
) -> impl IntoResponse {
    let rev_a = query.rev_a;
    let rev_b = query.rev_b;
    let format = query.format.unwrap_or_default();

    if rev_a == 0 || rev_b == 0 {
        return AirpError::BadRequest(
            "rev_a 和 rev_b 必须 >= 1（revision 编号从 1 起）".to_string(),
        )
        .into_response();
    }
    if rev_a == rev_b {
        return AirpError::BadRequest(format!("rev_a ({}) 不能等于 rev_b ({})", rev_a, rev_b))
            .into_response();
    }

    match run_build_diff(&state, &character_id, rev_a, rev_b).await {
        Ok(diff) => render_diff(&diff, &format, &character_id),
        Err(e) => e.into_response(),
    }
}

/// 列出角色所有可用 revision。
async fn run_list_revisions(
    state: &DaemonState,
    character_id: &str,
) -> Result<Vec<u64>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    ensure_character_exists(&state.data_root, &cid)?;
    list_revisions(&state.data_root, cid.as_str())
}

/// 加载指定 revision 的快照。
async fn run_get_revision(
    state: &DaemonState,
    character_id: &str,
    revision_id: &str,
) -> Result<RevisionSnapshot, AirpError> {
    let cid = CharacterId::new(character_id)?;
    ensure_character_exists(&state.data_root, &cid)?;
    let revision: u64 = revision_id.parse().map_err(|_| {
        AirpError::BadRequest(format!("revision_id {:?} 不是合法数字", revision_id))
    })?;
    load_revision_snapshot(&state.data_root, cid.as_str(), revision)
}

/// 构建两个 revision 的 diff。
async fn run_build_diff(
    state: &DaemonState,
    character_id: &str,
    rev_a: u64,
    rev_b: u64,
) -> Result<CardDiff, AirpError> {
    let cid = CharacterId::new(character_id)?;
    ensure_character_exists(&state.data_root, &cid)?;
    build_card_diff(&state.data_root, cid.as_str(), rev_a, rev_b)
}

/// 校验角色存在；不存在返回 `NotFound`。
fn ensure_character_exists(
    data_root: &std::path::Path,
    cid: &CharacterId,
) -> Result<(), AirpError> {
    let exists = crate::data_dir::list_characters(data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }
    Ok(())
}

/// diff 端点查询参数。
#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    /// `rev_a=N`（必填，>=1）
    pub rev_a: u64,
    /// `rev_b=N`（必填，>=1）
    pub rev_b: u64,
    /// `format=json|markdown|html`（缺省 `json`）
    pub format: Option<DiffExportFormat>,
}

/// revisions 列表响应体。
#[derive(Debug, Serialize)]
pub struct RevisionsListResponse {
    pub revisions: Vec<u64>,
}

/// 根据 format 渲染响应：JSON 直接返回，Markdown/HTML 返回文件下载。
fn render_diff(
    diff: &CardDiff,
    format: &DiffExportFormat,
    character_id: &str,
) -> axum::response::Response {
    match format {
        DiffExportFormat::Json => match serde_json::to_vec_pretty(diff) {
            Ok(bytes) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                bytes,
            )
                .into_response(),
            Err(e) => AirpError::from(e).into_response(),
        },
        DiffExportFormat::Markdown => {
            let md = crate::card_diff::to_markdown(diff);
            build_download_response(
                md.into_bytes(),
                "text/markdown; charset=utf-8",
                "md",
                character_id,
                diff.revision_a,
                diff.revision_b,
            )
        }
        DiffExportFormat::Html => {
            let html = crate::card_diff::to_html(diff);
            build_download_response(
                html.into_bytes(),
                "text/html; charset=utf-8",
                "html",
                character_id,
                diff.revision_a,
                diff.revision_b,
            )
        }
    }
}

/// 构造文件下载响应（含 Content-Type + Content-Disposition）。
fn build_download_response(
    body: Vec<u8>,
    content_type: &str,
    ext: &str,
    character_id: &str,
    rev_a: u64,
    rev_b: u64,
) -> axum::response::Response {
    let filename = format!(
        "card-diff-{}-rev{}-to-rev{}.{}",
        sanitize_filename(character_id),
        rev_a,
        rev_b,
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
        assert_eq!(pct_encode_utf8("a b"), "a%20b");
        assert_eq!(pct_encode_utf8("abc123"), "abc123");
        let encoded = pct_encode_utf8("中");
        assert_eq!(encoded, "%E4%B8%AD");
    }

    #[test]
    fn diff_query_deserializes_basic() {
        let q: DiffQuery = serde_json::from_str(r#"{"rev_a":1,"rev_b":2}"#).unwrap();
        assert_eq!(q.rev_a, 1);
        assert_eq!(q.rev_b, 2);
        assert!(q.format.is_none());
    }

    #[test]
    fn diff_query_deserializes_with_format() {
        let q: DiffQuery =
            serde_json::from_str(r#"{"rev_a":1,"rev_b":2,"format":"markdown"}"#).unwrap();
        assert!(matches!(q.format, Some(DiffExportFormat::Markdown)));
    }

    #[test]
    fn diff_query_missing_rev_a_fails() {
        let result: Result<DiffQuery, _> = serde_json::from_str(r#"{"rev_b":2}"#);
        assert!(result.is_err());
    }

    #[test]
    fn revisions_list_response_serializes() {
        let resp = RevisionsListResponse {
            revisions: vec![1, 3, 5],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"revisions\":[1,3,5]"));
    }
}
