//! Phase 4.6: 角色卡版本对比 (character card revision diff)。
//!
//! 基于已有 revision 系统读取两个 revision 的 `card.json`，生成结构化 diff
//! （字段级 added / removed / changed），同时支持 Markdown / HTML 渲染。
//!
//! ## 数据来源
//! - `{character_dir}/revisions/{rev_a}/card.json`
//! - `{character_dir}/revisions/{rev_b}/card.json`
//! - `{character_dir}/revisions/{rev}/manifest.json`（用于 created_at 元数据）
//!
//! ## 设计要点
//! - **只读**：不修改任何持久化数据
//! - **JSON 深度 diff**：递归比较 object / array / 标量；array 按索引逐元素对比
//!   （不做 LCS，避免大数组性能爆炸；元素级 added/removed/changed 已足够人工 review）
//! - **AIRP 独立实现**：所有 diff 算法自行编写，不引入第三方 diff 库
//! - **路径表达**：用 `$.field.subfield[index]` 的 JSON Pointer 风格定位变更
//! - **双向语义**：`added` 表示 rev_b 有而 rev_a 没有；`removed` 反之
//!
//! ## 安全
//! - revision 编号通过 `parse::<u64>()` 校验，拒绝非数字
//! - 路径校验：只允许在 `{character_dir}/revisions/{rev}/` 目录下读取 `card.json` 与
//!   `manifest.json`，不允许 `..` / 绝对路径
//! - manifest 复用 `RevisionManifest::from_json_bytes` 的不变量校验

use crate::error::AirpError;
use crate::revision::manifest::RevisionManifest;
use crate::types::CharacterId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 单条字段变更记录。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FieldChange {
    /// rev_a 没有，rev_b 有（added）。
    Added {
        /// JSON Pointer 路径（如 `$.data.name`）。
        path: String,
        /// rev_b 的值。
        value: Value,
    },
    /// rev_a 有，rev_b 没有（removed）。
    Removed {
        path: String,
        /// rev_a 的值。
        value: Value,
    },
    /// rev_a 与 rev_b 都有但值不同（changed）。
    Changed {
        path: String,
        /// rev_a 的旧值。
        old_value: Value,
        /// rev_b 的新值。
        new_value: Value,
    },
}

/// revision 元数据快照（来自 manifest）。
#[derive(Debug, Clone, Serialize)]
pub struct RevisionSnapshot {
    /// revision 编号。
    pub revision: u64,
    /// manifest 中记录的创建时间（ISO 8601）。
    pub created_at: String,
    /// tree_sha256（用于校验快照完整性）。
    pub tree_sha256: String,
    /// 该 revision 的 card.json 解析后的 JSON 对象。
    pub card: Value,
}

/// 完整 diff 结果。
#[derive(Debug, Clone, Serialize)]
pub struct CardDiff {
    /// 角色 ID。
    pub character_id: String,
    /// 较旧的 revision 编号（语义上的"基线"）。
    pub revision_a: u64,
    /// 较新的 revision 编号（语义上的"对照"）。
    pub revision_b: u64,
    /// revision_a 的元数据快照。
    pub snapshot_a: RevisionSnapshot,
    /// revision_b 的元数据快照。
    pub snapshot_b: RevisionSnapshot,
    /// 字段级变更列表（按 path 字典序排序，便于人工 review）。
    pub changes: Vec<FieldChange>,
    /// 统计：added 数量。
    pub added_count: usize,
    /// 统计：removed 数量。
    pub removed_count: usize,
    /// 统计：changed 数量。
    pub changed_count: usize,
}

/// 加载指定 revision 的 card.json + manifest.json。
///
/// `revision` 必须为 >= 1 的数字；函数会构造
/// `{character_dir}/revisions/{revision}/` 路径并读取 `card.json` 与
/// `manifest.json`。任一文件缺失或 manifest 校验失败返回 `Err`。
pub fn load_revision_snapshot(
    data_root: &Path,
    character_id: &str,
    revision: u64,
) -> Result<RevisionSnapshot, AirpError> {
    let cid = CharacterId::new(character_id)?;
    if revision == 0 {
        return Err(AirpError::BadRequest("revision 必须 >= 1".to_string()));
    }

    let char_dir = crate::data_dir::character_dir(data_root, cid.as_str())?;
    let revision_dir = char_dir.join("revisions").join(revision.to_string());

    // 拒绝 path traversal：revision_dir 必须在 char_dir/revisions/ 之下
    let revisions_dir = char_dir.join("revisions");
    let canonical_revision = revision_dir
        .canonicalize()
        .map_err(|_| AirpError::NotFound(format!("revision {} 目录不存在", revision)))?;
    let canonical_revisions = revisions_dir
        .canonicalize()
        .map_err(|_| AirpError::NotFound("revisions/ 目录不存在".to_string()))?;
    if !canonical_revision.starts_with(&canonical_revisions) {
        return Err(AirpError::BadRequest(format!(
            "revision {} 路径逃逸 revisions/ 目录",
            revision
        )));
    }

    // 读取 manifest.json 并校验
    let manifest_path = revision_dir.join("manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AirpError::NotFound(format!("revision {} 的 manifest.json 不存在", revision))
        } else {
            AirpError::from(e)
        }
    })?;
    let manifest = RevisionManifest::from_json_bytes(&manifest_bytes)?;

    // 读取 card.json
    let card_path = revision_dir.join("card.json");
    let card_bytes = std::fs::read(&card_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AirpError::NotFound(format!("revision {} 的 card.json 不存在", revision))
        } else {
            AirpError::from(e)
        }
    })?;
    let card: Value = serde_json::from_slice(&card_bytes)?;

    Ok(RevisionSnapshot {
        revision,
        created_at: manifest.created_at,
        tree_sha256: manifest.tree_sha256,
        card,
    })
}

/// 递归比较两个 JSON 值，把差异追加到 `changes`。
///
/// `path` 是当前节点的 JSON Pointer 路径（如 `$`、`$.data`、`$.data.name`）。
fn diff_values(a: &Value, b: &Value, path: String, changes: &mut Vec<FieldChange>) {
    match (a, b) {
        (Value::Object(obj_a), Value::Object(obj_b)) => {
            // 收集所有 key（按字典序处理，便于 changes 排序后稳定）
            let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for k in obj_a.keys() {
                keys.insert(k.clone());
            }
            for k in obj_b.keys() {
                keys.insert(k.clone());
            }
            for key in keys {
                let child_path = format!("{}.{}", path, key);
                match (obj_a.get(&key), obj_b.get(&key)) {
                    (Some(va), Some(vb)) => {
                        if va != vb {
                            diff_values(va, vb, child_path, changes);
                        }
                    }
                    (Some(va), None) => {
                        changes.push(FieldChange::Removed {
                            path: child_path,
                            value: va.clone(),
                        });
                    }
                    (None, Some(vb)) => {
                        changes.push(FieldChange::Added {
                            path: child_path,
                            value: vb.clone(),
                        });
                    }
                    (None, None) => { /* 不会发生：key 来自 a 或 b */ }
                }
            }
        }
        (Value::Array(arr_a), Value::Array(arr_b)) => {
            let len_a = arr_a.len();
            let len_b = arr_b.len();
            let common = len_a.min(len_b);
            for i in 0..common {
                let child_path = format!("{}[{}]", path, i);
                if arr_a[i] != arr_b[i] {
                    diff_values(&arr_a[i], &arr_b[i], child_path, changes);
                }
            }
            // a 比 b 长 → 多余元素 removed
            if len_a > common {
                for (i, va) in arr_a.iter().enumerate().take(len_a).skip(common) {
                    let child_path = format!("{}[{}]", path, i);
                    changes.push(FieldChange::Removed {
                        path: child_path,
                        value: va.clone(),
                    });
                }
            }
            // b 比 a 长 → 多余元素 added
            if len_b > common {
                for (i, vb) in arr_b.iter().enumerate().take(len_b).skip(common) {
                    let child_path = format!("{}[{}]", path, i);
                    changes.push(FieldChange::Added {
                        path: child_path,
                        value: vb.clone(),
                    });
                }
            }
        }
        // 标量（string/number/bool/null）或类型不同 → 直接比较
        (a, b) if a != b => {
            changes.push(FieldChange::Changed {
                path,
                old_value: a.clone(),
                new_value: b.clone(),
            });
        }
        _ => { /* 相等，无变更 */ }
    }
}

/// 计算两个 revision 的角色卡 diff。
///
/// `revision_a` 应小于 `revision_b`（语义上"旧 vs 新"）；若调用方传反，
/// 函数会自动交换并继续计算（语义不变，但 added/removed 方向会反转）。
pub fn build_card_diff(
    data_root: &Path,
    character_id: &str,
    revision_a: u64,
    revision_b: u64,
) -> Result<CardDiff, AirpError> {
    if revision_a == revision_b {
        return Err(AirpError::BadRequest(format!(
            "revision_a ({}) 不能等于 revision_b ({})",
            revision_a, revision_b
        )));
    }

    let (older, newer) = if revision_a < revision_b {
        (revision_a, revision_b)
    } else {
        (revision_b, revision_a)
    };

    let snapshot_a = load_revision_snapshot(data_root, character_id, older)?;
    let snapshot_b = load_revision_snapshot(data_root, character_id, newer)?;

    let mut changes: Vec<FieldChange> = Vec::new();
    diff_values(
        &snapshot_a.card,
        &snapshot_b.card,
        "$".to_string(),
        &mut changes,
    );
    changes.sort_by(|a, b| {
        let pa = change_path(a);
        let pb = change_path(b);
        pa.cmp(pb)
    });

    let added_count = changes
        .iter()
        .filter(|c| matches!(c, FieldChange::Added { .. }))
        .count();
    let removed_count = changes
        .iter()
        .filter(|c| matches!(c, FieldChange::Removed { .. }))
        .count();
    let changed_count = changes
        .iter()
        .filter(|c| matches!(c, FieldChange::Changed { .. }))
        .count();

    Ok(CardDiff {
        character_id: character_id.to_string(),
        revision_a: older,
        revision_b: newer,
        snapshot_a,
        snapshot_b,
        changes,
        added_count,
        removed_count,
        changed_count,
    })
}

/// 取 FieldChange 的 path（用于排序）。
fn change_path(c: &FieldChange) -> &str {
    match c {
        FieldChange::Added { path, .. } => path,
        FieldChange::Removed { path, .. } => path,
        FieldChange::Changed { path, .. } => path,
    }
}

/// 列出角色所有可用 revision 编号（已排序升序）。
///
/// 扫描 `{character_dir}/revisions/` 目录下的数字命名子目录。
/// 不存在的目录返回空 Vec。
pub fn list_revisions(data_root: &Path, character_id: &str) -> Result<Vec<u64>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let char_dir = crate::data_dir::character_dir(data_root, cid.as_str())?;
    let revisions_dir: PathBuf = char_dir.join("revisions");
    if !revisions_dir.exists() {
        return Ok(Vec::new());
    }
    let mut revisions: Vec<u64> = Vec::new();
    for entry in std::fs::read_dir(&revisions_dir)? {
        let entry = entry?;
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(n) => n,
            None => continue,
        };
        // 跳过 staging 目录与 dot-prefixed 入口
        if name.starts_with('.') {
            continue;
        }
        if let Ok(rev) = name.parse::<u64>() {
            if rev >= 1 {
                revisions.push(rev);
            }
        }
    }
    revisions.sort_unstable();
    Ok(revisions)
}

/// 把 CardDiff 序列化为 Markdown。
pub fn to_markdown(diff: &CardDiff) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(&format!(
        "# 角色 `{}` · 版本对比 (rev {} → rev {})\n\n",
        escape_md(&diff.character_id),
        diff.revision_a,
        diff.revision_b
    ));

    // 元数据
    out.push_str("## Revision 元数据\n\n");
    out.push_str(&format!(
        "- **rev {}**: created_at={}, tree_sha256=`{}`\n",
        diff.snapshot_a.revision,
        diff.snapshot_a.created_at,
        short_hash(&diff.snapshot_a.tree_sha256)
    ));
    out.push_str(&format!(
        "- **rev {}**: created_at={}, tree_sha256=`{}`\n\n",
        diff.snapshot_b.revision,
        diff.snapshot_b.created_at,
        short_hash(&diff.snapshot_b.tree_sha256)
    ));

    // 统计
    out.push_str("## 统计\n\n");
    out.push_str(&format!(
        "- Added: **{}**\n- Removed: **{}**\n- Changed: **{}**\n\n",
        diff.added_count, diff.removed_count, diff.changed_count
    ));

    // 变更明细
    out.push_str("## 变更明细\n\n");
    if diff.changes.is_empty() {
        out.push_str("_（两个 revision 的 card.json 内容完全相同）_\n");
    } else {
        for change in &diff.changes {
            match change {
                FieldChange::Added { path, value } => {
                    out.push_str(&format!(
                        "- 🟢 **Added** `{}`\n  ```json\n  {}\n  ```\n",
                        escape_md(path),
                        format_json(value)
                    ));
                }
                FieldChange::Removed { path, value } => {
                    out.push_str(&format!(
                        "- 🔴 **Removed** `{}`\n  ```json\n  {}\n  ```\n",
                        escape_md(path),
                        format_json(value)
                    ));
                }
                FieldChange::Changed {
                    path,
                    old_value,
                    new_value,
                } => {
                    out.push_str(&format!(
                        "- 🟡 **Changed** `{}`\n  - old:\n    ```json\n    {}\n    ```\n  - new:\n    ```json\n    {}\n    ```\n",
                        escape_md(path),
                        format_json(old_value),
                        format_json(new_value)
                    ));
                }
            }
        }
    }

    out
}

/// 把 CardDiff 序列化为单文件 HTML（内嵌 CSS）。
pub fn to_html(diff: &CardDiff) -> String {
    let mut out = String::with_capacity(8192);

    out.push_str("<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n");
    out.push_str("<meta charset=\"UTF-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    out.push_str(&format!(
        "<title>角色卡版本对比 rev {} → rev {}</title>\n",
        diff.revision_a, diff.revision_b
    ));
    out.push_str(HTML_STYLE);
    out.push_str("</head>\n<body>\n<main class=\"diff-doc\">\n");

    out.push_str(&format!(
        "<h1 class=\"doc-title\">角色 <code>{}</code> · 版本对比</h1>\n",
        escape_html(&diff.character_id)
    ));
    out.push_str(&format!(
        "<p class=\"rev-range\">rev <b>{}</b> → rev <b>{}</b></p>\n",
        diff.revision_a, diff.revision_b
    ));

    // 元数据
    out.push_str("<section class=\"meta-card\">\n<h2>Revision 元数据</h2>\n<table>\n<thead><tr><th></th><th>rev ");
    out.push_str(&diff.revision_a.to_string());
    out.push_str("</th><th>rev ");
    out.push_str(&diff.revision_b.to_string());
    out.push_str("</th></tr></thead><tbody>\n");
    out.push_str(&format!(
        "<tr><td>created_at</td><td>{}</td><td>{}</td></tr>\n",
        escape_html(&diff.snapshot_a.created_at),
        escape_html(&diff.snapshot_b.created_at)
    ));
    out.push_str(&format!(
        "<tr><td>tree_sha256</td><td><code>{}</code></td><td><code>{}</code></td></tr>\n",
        escape_html(&short_hash(&diff.snapshot_a.tree_sha256)),
        escape_html(&short_hash(&diff.snapshot_b.tree_sha256))
    ));
    out.push_str("</tbody></table>\n</section>\n");

    // 统计
    out.push_str("<section class=\"stats\">\n");
    out.push_str(&format!(
        "<span class=\"stat-pill stat-added\">🟢 Added <b>{}</b></span>\n",
        diff.added_count
    ));
    out.push_str(&format!(
        "<span class=\"stat-pill stat-removed\">🔴 Removed <b>{}</b></span>\n",
        diff.removed_count
    ));
    out.push_str(&format!(
        "<span class=\"stat-pill stat-changed\">🟡 Changed <b>{}</b></span>\n",
        diff.changed_count
    ));
    out.push_str("</section>\n");

    // 变更明细
    out.push_str("<section class=\"changes\">\n<h2>变更明细</h2>\n");
    if diff.changes.is_empty() {
        out.push_str("<p class=\"empty\">（两个 revision 的 card.json 内容完全相同）</p>\n");
    } else {
        out.push_str("<ul class=\"change-list\">\n");
        for change in &diff.changes {
            match change {
                FieldChange::Added { path, value } => {
                    out.push_str("<li class=\"change-item change-added\">\n");
                    out.push_str(&format!(
                        "<div class=\"change-meta\"><span class=\"op op-added\">🟢 Added</span><code class=\"path\">{}</code></div>\n",
                        escape_html(path)
                    ));
                    out.push_str(&format!(
                        "<pre class=\"value\">{}</pre>\n",
                        escape_html(&format_json(value))
                    ));
                    out.push_str("</li>\n");
                }
                FieldChange::Removed { path, value } => {
                    out.push_str("<li class=\"change-item change-removed\">\n");
                    out.push_str(&format!(
                        "<div class=\"change-meta\"><span class=\"op op-removed\">🔴 Removed</span><code class=\"path\">{}</code></div>\n",
                        escape_html(path)
                    ));
                    out.push_str(&format!(
                        "<pre class=\"value\">{}</pre>\n",
                        escape_html(&format_json(value))
                    ));
                    out.push_str("</li>\n");
                }
                FieldChange::Changed {
                    path,
                    old_value,
                    new_value,
                } => {
                    out.push_str("<li class=\"change-item change-changed\">\n");
                    out.push_str(&format!(
                        "<div class=\"change-meta\"><span class=\"op op-changed\">🟡 Changed</span><code class=\"path\">{}</code></div>\n",
                        escape_html(path)
                    ));
                    out.push_str("<div class=\"value-pair\">\n");
                    out.push_str(&format!(
                        "<div class=\"value-old\"><span class=\"value-label\">old</span><pre class=\"value\">{}</pre></div>\n",
                        escape_html(&format_json(old_value))
                    ));
                    out.push_str(&format!(
                        "<div class=\"value-new\"><span class=\"value-label\">new</span><pre class=\"value\">{}</pre></div>\n",
                        escape_html(&format_json(new_value))
                    ));
                    out.push_str("</div>\n</li>\n");
                }
            }
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");

    out.push_str("</main>\n</body>\n</html>\n");
    out
}

/// 缩略 SHA-256（前 12 位 + ...）。
fn short_hash(hash: &str) -> String {
    if hash.len() <= 16 {
        hash.to_string()
    } else {
        format!("{}...", &hash[..16])
    }
}

/// 美化 JSON 输出。失败时退回 Debug 输出。
fn format_json(value: &Value) -> String {
    match serde_json::to_string_pretty(value) {
        Ok(s) => s,
        Err(_) => format!("{:?}", value),
    }
}

/// HTML 转义：& < > " '
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Markdown 转义：# * _ ` [ ] \ < > |
fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '#' | '*' | '_' | '`' | '[' | ']' | '\\' | '<' | '>' | '|' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// 导出格式查询参数。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiffExportFormat {
    /// 默认：结构化 JSON
    #[default]
    Json,
    /// Markdown
    Markdown,
    /// HTML
    Html,
}

const HTML_STYLE: &str = r#"<style>
:root {
  --fg: #1f2328;
  --fg-muted: #57606a;
  --bg: #ffffff;
  --bg-subtle: #f6f8fa;
  --border: #d0d7de;
  --added: #1a7f37;
  --removed: #cf222e;
  --changed: #bf3989;
}
@media (prefers-color-scheme: dark) {
  :root {
    --fg: #e6edf3;
    --fg-muted: #8b949e;
    --bg: #0d1117;
    --bg-subtle: #161b22;
    --border: #30363d;
    --added: #3fb950;
    --removed: #f85149;
    --changed: #d2a8ff;
  }
}
* { box-sizing: border-box; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", Arial, "PingFang SC", "Microsoft YaHei", sans-serif;
  color: var(--fg); background: var(--bg);
  margin: 0; padding: 24px; line-height: 1.6;
}
.diff-doc { max-width: 980px; margin: 0 auto; }
.doc-title { font-size: 24px; margin: 0 0 8px; }
.doc-title code { background: var(--bg-subtle); padding: 2px 8px; border-radius: 4px; font-size: 20px; }
.rev-range { font-size: 16px; color: var(--fg-muted); margin: 0 0 16px; }
.meta-card {
  background: var(--bg-subtle); border: 1px solid var(--border);
  border-radius: 8px; padding: 12px 16px; margin: 16px 0;
}
.meta-card h2 { margin: 0 0 12px; font-size: 14px; color: var(--fg-muted); }
.meta-card table { width: 100%; border-collapse: collapse; font-size: 13px; }
.meta-card th, .meta-card td { padding: 6px 10px; border: 1px solid var(--border); text-align: left; }
.meta-card th { background: var(--bg); font-weight: 600; }
.meta-card code { background: var(--bg); padding: 1px 5px; border-radius: 3px; font-size: 12px; }
.stats { display: flex; gap: 8px; flex-wrap: wrap; margin: 16px 0; }
.stat-pill { display: inline-flex; align-items: center; gap: 6px; padding: 4px 12px; border-radius: 999px; border: 1px solid var(--border); font-size: 12px; }
.stat-pill b { font-weight: 600; }
.stat-added { color: var(--added); border-color: var(--added); }
.stat-removed { color: var(--removed); border-color: var(--removed); }
.stat-changed { color: var(--changed); border-color: var(--changed); }
.changes { margin: 24px 0; }
.changes h2 { font-size: 16px; margin: 16px 0 8px; }
.change-list { list-style: none; padding: 0; margin: 0; }
.change-item { padding: 8px 12px; margin: 8px 0; border-left: 3px solid var(--border); background: var(--bg-subtle); border-radius: 0 6px 6px 0; }
.change-added { border-left-color: var(--added); }
.change-removed { border-left-color: var(--removed); }
.change-changed { border-left-color: var(--changed); }
.change-meta { display: flex; gap: 8px; align-items: center; margin-bottom: 6px; font-size: 12px; flex-wrap: wrap; }
.op { font-weight: 600; padding: 1px 6px; border-radius: 3px; }
.op-added { color: var(--added); background: rgba(26, 127, 55, 0.1); }
.op-removed { color: var(--removed); background: rgba(207, 34, 46, 0.1); }
.op-changed { color: var(--changed); background: rgba(191, 57, 137, 0.1); }
.path { background: var(--bg); padding: 1px 6px; border-radius: 3px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; word-break: break-all; }
.value { padding: 8px; background: var(--bg); border: 1px solid var(--border); border-radius: 4px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; white-space: pre-wrap; word-break: break-word; margin: 0; max-height: 240px; overflow: auto; }
.value-pair { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.value-label { display: block; font-size: 11px; color: var(--fg-muted); margin-bottom: 4px; font-weight: 600; }
.value-old .value { border-left: 2px solid var(--removed); }
.value-new .value { border-left: 2px solid var(--added); }
.empty { color: var(--fg-muted); font-style: italic; }
@media print { body { padding: 0; } .change-item { break-inside: avoid; } }
</style>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revision::atomic::{commit_revision, CommitOptions, StagedRevision};
    use crate::revision::manifest::{AssetKind, AssetSource};
    use tempfile::tempdir;

    fn commit_card(
        data_root: &Path,
        character_id: &str,
        revision: u64,
        card_json: &str,
    ) -> Result<(), AirpError> {
        let cid = CharacterId::new(character_id)?;
        let char_dir = crate::data_dir::character_dir(data_root, cid.as_str())?;
        std::fs::create_dir_all(&char_dir)?;

        let staged = StagedRevision {
            content_revision: revision,
            asset_kind: AssetKind::Character,
            asset_id: character_id.to_string(),
            created_at: format!("2026-07-{:02}T00:00:00Z", revision),
            source: AssetSource {
                source_kind: "controlled_upload".to_string(),
                ..Default::default()
            },
            files: vec![("card.json".to_string(), card_json.as_bytes().to_vec())],
        };
        let opts = CommitOptions::new(&char_dir);
        commit_revision(&staged, &opts)?;
        Ok(())
    }

    #[test]
    fn diff_values_identical_objects_no_changes() {
        let a = serde_json::json!({"name": "alice", "age": 25});
        let b = serde_json::json!({"name": "alice", "age": 25});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert!(changes.is_empty());
    }

    #[test]
    fn diff_values_detects_added_field() {
        let a = serde_json::json!({"name": "alice"});
        let b = serde_json::json!({"name": "alice", "age": 25});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            FieldChange::Added { path, value } => {
                assert_eq!(path, "$.age");
                assert_eq!(value, &serde_json::json!(25));
            }
            _ => panic!("expected Added"),
        }
    }

    #[test]
    fn diff_values_detects_removed_field() {
        let a = serde_json::json!({"name": "alice", "age": 25});
        let b = serde_json::json!({"name": "alice"});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], FieldChange::Removed { .. }));
    }

    #[test]
    fn diff_values_detects_changed_scalar() {
        let a = serde_json::json!({"name": "alice"});
        let b = serde_json::json!({"name": "bob"});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            FieldChange::Changed {
                path,
                old_value,
                new_value,
            } => {
                assert_eq!(path, "$.name");
                assert_eq!(old_value, &serde_json::json!("alice"));
                assert_eq!(new_value, &serde_json::json!("bob"));
            }
            _ => panic!("expected Changed"),
        }
    }

    #[test]
    fn diff_values_recurses_into_nested_objects() {
        let a = serde_json::json!({"data": {"name": "alice", "tags": ["a"]}});
        let b = serde_json::json!({"data": {"name": "bob", "tags": ["a", "b"]}});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert_eq!(changes.len(), 2);
        let paths: Vec<&str> = changes.iter().map(change_path).collect();
        assert!(paths.contains(&"$.data.name"));
        assert!(paths.contains(&"$.data.tags[1]"));
    }

    #[test]
    fn diff_values_array_length_diff() {
        let a = serde_json::json!({"arr": [1, 2, 3]});
        let b = serde_json::json!({"arr": [1, 2]});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            FieldChange::Removed { path, value } => {
                assert_eq!(path, "$.arr[2]");
                assert_eq!(value, &serde_json::json!(3));
            }
            _ => panic!("expected Removed"),
        }
    }

    #[test]
    fn diff_values_type_change_is_changed() {
        // 类型从 string 变为 object
        let a = serde_json::json!({"x": "hello"});
        let b = serde_json::json!({"x": {"nested": true}});
        let mut changes = Vec::new();
        diff_values(&a, &b, "$".to_string(), &mut changes);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], FieldChange::Changed { .. }));
    }

    #[test]
    fn build_card_diff_rejects_equal_revisions() {
        let tmp = tempdir().unwrap();
        let result = build_card_diff(tmp.path(), "ghost", 1, 1);
        assert!(result.is_err());
    }

    #[test]
    fn build_card_diff_swaps_revisions_if_a_greater_than_b() {
        let tmp = tempdir().unwrap();
        commit_card(tmp.path(), "alice", 1, r#"{"name":"alice v1"}"#).unwrap();
        commit_card(tmp.path(), "alice", 2, r#"{"name":"alice v2"}"#).unwrap();

        // 故意传 (2, 1)，函数应自动交换为 (1, 2)
        let diff = build_card_diff(tmp.path(), "alice", 2, 1).unwrap();
        assert_eq!(diff.revision_a, 1);
        assert_eq!(diff.revision_b, 2);
    }

    #[test]
    fn build_card_diff_with_real_revisions() {
        let tmp = tempdir().unwrap();
        let card_v1 = r#"{"name":"alice","description":"v1 desc","data":{"tags":["friendly"]}}"#;
        let card_v2 =
            r#"{"name":"alice","description":"v2 desc","data":{"tags":["friendly","brave"]}}"#;
        commit_card(tmp.path(), "alice", 1, card_v1).unwrap();
        commit_card(tmp.path(), "alice", 2, card_v2).unwrap();

        let diff = build_card_diff(tmp.path(), "alice", 1, 2).unwrap();
        assert_eq!(diff.revision_a, 1);
        assert_eq!(diff.revision_b, 2);
        assert_eq!(diff.added_count, 1); // $.data.tags[1]
        assert_eq!(diff.removed_count, 0);
        assert_eq!(diff.changed_count, 1); // $.description
        assert_eq!(diff.changes.len(), 2);
    }

    #[test]
    fn build_card_diff_identical_cards_no_changes() {
        let tmp = tempdir().unwrap();
        let card = r#"{"name":"alice","age":25}"#;
        commit_card(tmp.path(), "alice", 1, card).unwrap();
        commit_card(tmp.path(), "alice", 2, card).unwrap();

        let diff = build_card_diff(tmp.path(), "alice", 1, 2).unwrap();
        assert!(diff.changes.is_empty());
        assert_eq!(diff.added_count, 0);
        assert_eq!(diff.removed_count, 0);
        assert_eq!(diff.changed_count, 0);
    }

    #[test]
    fn build_card_diff_missing_revision_returns_not_found() {
        let tmp = tempdir().unwrap();
        commit_card(tmp.path(), "alice", 1, r#"{"name":"alice"}"#).unwrap();
        // revision 99 不存在
        let result = build_card_diff(tmp.path(), "alice", 1, 99);
        assert!(result.is_err());
    }

    #[test]
    fn build_card_diff_revision_zero_rejected() {
        let tmp = tempdir().unwrap();
        let result = build_card_diff(tmp.path(), "alice", 0, 1);
        assert!(result.is_err());
    }

    #[test]
    fn list_revisions_returns_empty_when_no_revisions_dir() {
        let tmp = tempdir().unwrap();
        let result = list_revisions(tmp.path(), "ghost-char").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn list_revisions_returns_sorted_revisions() {
        let tmp = tempdir().unwrap();
        // 故意以反序 commit（虽然 commit_revision 不允许，但跳过中间 revision 是允许的）
        commit_card(tmp.path(), "alice", 1, r#"{"name":"alice"}"#).unwrap();
        commit_card(tmp.path(), "alice", 3, r#"{"name":"alice v3"}"#).unwrap();
        commit_card(tmp.path(), "alice", 5, r#"{"name":"alice v5"}"#).unwrap();

        let revisions = list_revisions(tmp.path(), "alice").unwrap();
        assert_eq!(revisions, vec![1, 3, 5]);
    }

    #[test]
    fn list_revisions_ignores_staging_and_non_numeric() {
        let tmp = tempdir().unwrap();
        commit_card(tmp.path(), "alice", 1, r#"{"name":"alice"}"#).unwrap();
        // 手动创建 staging 和非数字目录
        let char_dir = tmp.path().join("characters").join("alice");
        std::fs::create_dir_all(char_dir.join("revisions").join(".staging-99")).unwrap();
        std::fs::create_dir_all(char_dir.join("revisions").join("not-a-number")).unwrap();

        let revisions = list_revisions(tmp.path(), "alice").unwrap();
        assert_eq!(revisions, vec![1]);
    }

    #[test]
    fn to_markdown_renders_basic_diff() {
        let tmp = tempdir().unwrap();
        commit_card(tmp.path(), "alice", 1, r#"{"name":"alice","v":1}"#).unwrap();
        commit_card(tmp.path(), "alice", 2, r#"{"name":"alice","v":2}"#).unwrap();

        let diff = build_card_diff(tmp.path(), "alice", 1, 2).unwrap();
        let md = to_markdown(&diff);
        assert!(md.contains("版本对比"));
        assert!(md.contains("rev 1 → rev 2"));
        assert!(md.contains("Changed"));
        assert!(md.contains("$.v"));
    }

    #[test]
    fn to_markdown_empty_changes() {
        let tmp = tempdir().unwrap();
        let card = r#"{"name":"alice"}"#;
        commit_card(tmp.path(), "alice", 1, card).unwrap();
        commit_card(tmp.path(), "alice", 2, card).unwrap();
        let diff = build_card_diff(tmp.path(), "alice", 1, 2).unwrap();
        let md = to_markdown(&diff);
        assert!(md.contains("完全相同"));
    }

    #[test]
    fn to_html_renders_full_diff() {
        let tmp = tempdir().unwrap();
        commit_card(tmp.path(), "alice", 1, r#"{"name":"alice","v":1}"#).unwrap();
        commit_card(
            tmp.path(),
            "alice",
            2,
            r#"{"name":"alice","v":2,"new":"x"}"#,
        )
        .unwrap();
        let diff = build_card_diff(tmp.path(), "alice", 1, 2).unwrap();
        let html = to_html(&diff);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("rev 1"));
        assert!(html.contains("rev 2"));
        assert!(html.contains("op-changed"));
        assert!(html.contains("op-added"));
        assert!(html.contains("$.v"));
        assert!(html.contains("$.new"));
    }

    #[test]
    fn to_html_escapes_xss_attempt() {
        let tmp = tempdir().unwrap();
        commit_card(
            tmp.path(),
            "alice",
            1,
            r#"{"name":"<script>alert(1)</script>"}"#,
        )
        .unwrap();
        commit_card(tmp.path(), "alice", 2, r#"{"name":"alice"}"#).unwrap();
        let diff = build_card_diff(tmp.path(), "alice", 1, 2).unwrap();
        let html = to_html(&diff);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn escape_html_special_chars() {
        assert_eq!(
            escape_html("a<b>c&d\"e'f"),
            "a&lt;b&gt;c&amp;d&quot;e&#39;f"
        );
    }

    #[test]
    fn escape_md_special_chars() {
        assert_eq!(escape_md("a*b_c"), "a\\*b\\_c");
    }

    #[test]
    fn short_hash_truncates_long_hash() {
        let long = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let short = short_hash(long);
        assert!(short.ends_with("..."));
        assert_eq!(short.len(), 16 + 3);
    }

    #[test]
    fn short_hash_keeps_short_hash_unchanged() {
        let short = "abc123";
        assert_eq!(short_hash(short), "abc123");
    }

    #[test]
    fn format_json_pretty_prints() {
        let value = serde_json::json!({"a": 1, "b": [2, 3]});
        let formatted = format_json(&value);
        assert!(formatted.contains("\n"));
        assert!(formatted.contains("\"a\""));
    }

    #[test]
    fn diff_export_format_default_is_json() {
        let fmt = DiffExportFormat::default();
        assert!(matches!(fmt, DiffExportFormat::Json));
    }

    #[test]
    fn diff_export_format_deserializes_lowercase() {
        let md: DiffExportFormat = serde_json::from_str("\"markdown\"").unwrap();
        assert!(matches!(md, DiffExportFormat::Markdown));
        let html: DiffExportFormat = serde_json::from_str("\"html\"").unwrap();
        assert!(matches!(html, DiffExportFormat::Html));
    }

    #[test]
    fn load_revision_snapshot_returns_not_found_for_missing_dir() {
        let tmp = tempdir().unwrap();
        let result = load_revision_snapshot(tmp.path(), "ghost", 1);
        assert!(result.is_err());
    }

    #[test]
    fn load_revision_snapshot_rejects_zero_revision() {
        let tmp = tempdir().unwrap();
        let result = load_revision_snapshot(tmp.path(), "alice", 0);
        assert!(result.is_err());
    }
}
