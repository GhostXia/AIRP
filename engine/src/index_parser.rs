//! 全局 index.md 差分解析与合并。
//!
//! LLM 在封卷时输出 `<全局index更新>` 块，本模块负责：
//! 1. 把这个块解析成结构化的 IndexDiff
//! 2. 将 IndexDiff 合并进现有的 index.md
//!
//! 输入格式（每行一条指令）：
//! ```text
//! <全局index更新>
//! [人物] 提升: 艾莉娅(卷6·身份揭晓)
//! [人物] 仅本卷: 路过的观众
//! [物品] 提升: 神殿地图(卷6·转交玩家)
//! [地点] 提升: 古神殿(卷6·首次进入)
//! [线索] 艾莉娅身份 → 卷6(已解)
//! [线索] 神殿内部机关 → [新增·未解]
//! </全局index更新>
//! ```

use once_cell::sync::Lazy;
use regex::Regex;

// M2.5：预编译 Regex。
static VOL_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"卷\s*(\d+)").expect("VOL_NUM_RE"));
static DIFF_BLOCK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<全局index更新>(.*?)</全局index更新>").expect("DIFF_BLOCK_RE"));
static LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*\[(\S+?)\]\s*(.+?)\s*$").expect("LINE_RE"));
static PROMOTE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(提升|仅本卷)\s*[:：]\s*(.+?)\s*$").expect("PROMOTE_RE"));
static THREAD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(.+?)\s*(?:→|->)\s*(.+?)\s*$").expect("THREAD_RE"));

/// 实体类别。index.md 用二级标题（## 人物 / ## 物品 / ## 地点）分段记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityCategory {
    /// 人物（角色）。
    Character,
    /// 关键物品 / 道具。
    Item,
    /// 地点。
    Location,
}

impl EntityCategory {
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "人物" => Some(EntityCategory::Character),
            "物品" => Some(EntityCategory::Item),
            "地点" => Some(EntityCategory::Location),
            _ => None,
        }
    }

    /// 返回该类别在 index.md 中的二级标题文字。
    /// 当前未使用（保留供未来 index 渲染调用）。
    #[allow(dead_code)]
    pub fn section_title(&self) -> &'static str {
        match self {
            EntityCategory::Character => "人物",
            EntityCategory::Item => "物品",
            EntityCategory::Location => "地点",
        }
    }
}

/// 实体晋升模式：决定从卷级条目能否升入全局 index.md。
#[derive(Debug, Clone, PartialEq)]
pub enum PromoteMode {
    /// 提升到全局 index
    Promote,
    /// 仅本卷记录，不入全局（解析后仅保留用于日志或调试）
    LocalOnly,
}

/// 一条实体（人物 / 物品 / 地点）的索引项。
#[derive(Debug, Clone, PartialEq)]
pub struct EntityEntry {
    /// 所属类别。
    pub category: EntityCategory,
    /// 是否需要晋升入全局 index。
    pub mode: PromoteMode,
    /// 实体名（已归一化）。
    pub name: String,
    /// 关联卷号；为 `None` 表示跨卷或未指定。
    pub volume: Option<u32>,
    /// 自由备注（如「初登场」「失踪」等）。
    pub note: String,
}

/// 悬挂线索的状态变更类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadStatus {
    /// 新增悬挂线索，标记为 [新增·未解]
    NewOpen,
    /// 已有线索推进/更新
    Progressed,
    /// 线索已解决
    Resolved,
}

/// 单条悬挂线索的更新记录。
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadUpdate {
    /// 线索名（与 index.md 中条目对齐）。
    pub name: String,
    /// 触发更新的卷号。
    pub volume: Option<u32>,
    /// 本次状态变更类型。
    pub status: ThreadStatus,
    /// 自由备注。
    pub note: String,
}

/// 一次封卷输出解析得到的索引增量。可直接通过 [`apply_diff`] 合并到 index.md。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexDiff {
    /// 本卷新增 / 更新的实体集合。
    pub entities: Vec<EntityEntry>,
    /// 本卷涉及的悬挂线索更新集合。
    pub threads: Vec<ThreadUpdate>,
}

/// 从一行 `卷6` 或 `卷012` 这样的字符串中提取数字。
fn extract_volume_number(s: &str) -> Option<u32> {
    VOL_NUM_RE
        .captures(s)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
}

/// `parse_named_with_paren` 的返回值。M0 F-30 / 6.0g：取代原 3-tuple，命名更可读。
#[derive(Debug, Clone, PartialEq)]
struct ParsedNamedRef {
    name: String,
    volume: Option<u32>,
    note: String,
}

/// 解析形如 `名字(卷6·备注)` 或 `名字` 的字符串。
fn parse_named_with_paren(input: &str) -> ParsedNamedRef {
    let trimmed = input.trim();
    if let Some(open) = trimmed.find('(') {
        if let Some(close) = trimmed.rfind(')') {
            if close > open {
                let name = trimmed[..open].trim().to_string();
                let inside = &trimmed[open + 1..close];
                let volume = extract_volume_number(inside);
                let note = inside
                    .split(&['·', '·'][..])
                    .filter(|s| extract_volume_number(s).is_none())
                    .collect::<Vec<&str>>()
                    .join("·")
                    .trim()
                    .to_string();
                return ParsedNamedRef { name, volume, note };
            }
        }
    }
    ParsedNamedRef {
        name: trimmed.to_string(),
        volume: None,
        note: String::new(),
    }
}

/// 从 LLM 完整输出中提取 `<全局index更新>` 块的内部内容。
pub fn extract_diff_block(raw: &str) -> Option<String> {
    DIFF_BLOCK_RE
        .captures(raw)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 解析 `<全局index更新>` 块的内容为 IndexDiff。
pub fn parse_index_diff(raw: &str) -> IndexDiff {
    let mut diff = IndexDiff::default();

    // 如果传入的是带标签的完整块，先提取内部
    let body = extract_diff_block(raw).unwrap_or_else(|| raw.to_string());

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some(caps) = LINE_RE.captures(line) else {
            continue;
        };
        let tag = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let rest = caps.get(2).map(|m| m.as_str()).unwrap_or("").trim();

        if tag == "线索" {
            // [线索] 艾莉娅身份 → 卷6(已解)
            // [线索] 神殿内部机关 → [新增·未解]
            if let Some(tcaps) = THREAD_RE.captures(rest) {
                let name = tcaps
                    .get(1)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                let status_part = tcaps
                    .get(2)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();

                let (status, volume, note) =
                    if status_part.contains("[新增") || status_part.contains("新增·") {
                        (
                            ThreadStatus::NewOpen,
                            extract_volume_number(&status_part),
                            status_part.clone(),
                        )
                    } else if status_part.contains("已解") {
                        let vol = extract_volume_number(&status_part);
                        (ThreadStatus::Resolved, vol, status_part.clone())
                    } else {
                        let vol = extract_volume_number(&status_part);
                        (ThreadStatus::Progressed, vol, status_part.clone())
                    };

                diff.threads.push(ThreadUpdate {
                    name,
                    volume,
                    status,
                    note,
                });
            }
        } else if let Some(cat) = EntityCategory::from_tag(tag) {
            // [人物] 提升: 艾莉娅(卷6·身份揭晓)
            if let Some(pcaps) = PROMOTE_RE.captures(rest) {
                let mode_str = pcaps.get(1).map(|m| m.as_str()).unwrap_or("");
                let payload = pcaps.get(2).map(|m| m.as_str()).unwrap_or("").trim();

                let mode = if mode_str == "提升" {
                    PromoteMode::Promote
                } else {
                    PromoteMode::LocalOnly
                };

                // 允许逗号分隔多个条目，宽松处理
                for piece in payload.split(&[',', '，'][..]) {
                    let piece = piece.trim();
                    if piece.is_empty() {
                        continue;
                    }
                    let parsed = parse_named_with_paren(piece);
                    if parsed.name.is_empty() {
                        continue;
                    }
                    diff.entities.push(EntityEntry {
                        category: cat.clone(),
                        mode: mode.clone(),
                        name: parsed.name,
                        volume: parsed.volume,
                        note: parsed.note,
                    });
                }
            }
        }
    }

    diff
}

/// index.md 的结构化表示，便于增量合并。
#[derive(Debug, Clone, Default)]
pub struct IndexSections {
    /// `## 人物` 段下的原文行。
    pub characters: Vec<String>,
    /// `## 物品` 段下的原文行。
    pub items: Vec<String>,
    /// `## 地点` 段下的原文行。
    pub locations: Vec<String>,
    /// `## 悬挂线索` 段下的原文行。
    pub threads: Vec<String>,
    /// `## [已归档]` 段下的原文行。
    pub archived: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum SectionKind {
    Characters,
    Items,
    Locations,
    Threads,
    Archived,
    Unknown,
}

/// 把 index.md 文本解析为按 section 分类的行集合（保留每行原文）。
pub fn parse_sections(md: &str) -> IndexSections {
    let mut s = IndexSections::default();
    let mut current = SectionKind::Unknown;

    for line in md.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            current = match rest.trim() {
                "人物" => SectionKind::Characters,
                "物品" => SectionKind::Items,
                "地点" => SectionKind::Locations,
                "悬挂线索" => SectionKind::Threads,
                "[已归档]" => SectionKind::Archived,
                _ => SectionKind::Unknown,
            };
            continue;
        }

        if !trimmed.starts_with("- ") {
            continue;
        }

        match current {
            SectionKind::Characters => s.characters.push(line.to_string()),
            SectionKind::Items => s.items.push(line.to_string()),
            SectionKind::Locations => s.locations.push(line.to_string()),
            SectionKind::Threads => s.threads.push(line.to_string()),
            SectionKind::Archived => s.archived.push(line.to_string()),
            SectionKind::Unknown => {}
        }
    }

    s
}

/// 将 IndexSections 序列化回 index.md 文本。
pub fn serialize_sections(s: &IndexSections) -> String {
    let mut out = String::from("# 全局索引\n\n");

    out.push_str("## 人物\n");
    for line in &s.characters {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("## 物品\n");
    for line in &s.items {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("## 悬挂线索\n");
    for line in &s.threads {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("## 地点\n");
    for line in &s.locations {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("## [已归档]\n");
    for line in &s.archived {
        out.push_str(line);
        out.push('\n');
    }

    out
}

/// 在指定段中按"实体名"查找已有行；返回索引（如有）。
/// 行格式约定：`- 实体名: ...`
fn find_entity_line(lines: &[String], name: &str) -> Option<usize> {
    let prefix = format!("- {}", name);
    let prefix_colon = format!("- {}:", name);
    for (i, line) in lines.iter().enumerate() {
        let l = line.trim_start();
        if l.starts_with(&prefix_colon) {
            return Some(i);
        }
        // 兜底：精确等于 "- 名字"（无 ":"）也算
        if l == prefix {
            return Some(i);
        }
    }
    None
}

/// 把一个新卷号片段追加到已有的实体行上。例如已有 "- 艾莉娅: 卷2, 卷4"，追加 "卷6(身份揭晓)"。
fn append_volume_to_line(line: &str, new_piece: &str) -> String {
    if new_piece.is_empty() {
        return line.to_string();
    }
    let trimmed = line.trim_end();
    if trimmed.ends_with(':') || trimmed.ends_with('：') {
        // 该行还没有任何卷号
        format!("{} {}", trimmed, new_piece)
    } else {
        // 该行已经有卷号，用逗号追加
        format!("{}, {}", trimmed, new_piece)
    }
}

/// 把 IndexDiff 合并进 index.md，返回新的 index.md 文本。
pub fn apply_diff(index_md: &str, diff: &IndexDiff) -> String {
    let mut sections = parse_sections(index_md);

    // 处理实体（人物/物品/地点）
    for entry in &diff.entities {
        if entry.mode != PromoteMode::Promote {
            continue; // 仅本卷的实体不入全局
        }

        let target: &mut Vec<String> = match entry.category {
            EntityCategory::Character => &mut sections.characters,
            EntityCategory::Item => &mut sections.items,
            EntityCategory::Location => &mut sections.locations,
        };

        // 构造卷号片段
        let piece = match (entry.volume, entry.note.is_empty()) {
            (Some(v), true) => format!("卷{}", v),
            (Some(v), false) => format!("卷{}({})", v, entry.note),
            (None, _) => continue, // 没有卷号的提升条目跳过
        };

        if let Some(idx) = find_entity_line(target, &entry.name) {
            // 已有，追加卷号
            target[idx] = append_volume_to_line(&target[idx], &piece);
        } else {
            // 新增一行
            target.push(format!("- {}: {}", entry.name, piece));
        }
    }

    // 处理悬挂线索
    for thread in &diff.threads {
        match thread.status {
            ThreadStatus::NewOpen => {
                // 检查是否已存在同名线索（避免重复）
                if find_entity_line(&sections.threads, &thread.name).is_some() {
                    continue;
                }
                let piece = match thread.volume {
                    Some(v) => format!("- {}: 卷{}(提出) → [未解]", thread.name, v),
                    None => format!("- {}: [新增·未解]", thread.name),
                };
                sections.threads.push(piece);
            }
            ThreadStatus::Progressed => {
                let piece = match thread.volume {
                    Some(v) => format!("卷{}({})", v, thread.note.replace('卷', "")),
                    None => thread.note.clone(),
                };
                if let Some(idx) = find_entity_line(&sections.threads, &thread.name) {
                    sections.threads[idx] = append_volume_to_line(&sections.threads[idx], &piece);
                } else {
                    sections
                        .threads
                        .push(format!("- {}: {}", thread.name, piece));
                }
            }
            ThreadStatus::Resolved => {
                let piece = match thread.volume {
                    Some(v) => format!("卷{}(已解)", v),
                    None => "(已解)".to_string(),
                };
                let final_line =
                    if let Some(idx) = find_entity_line(&sections.threads, &thread.name) {
                        let merged = append_volume_to_line(&sections.threads[idx], &piece);
                        sections.threads.remove(idx);
                        merged
                    } else {
                        format!("- {}: {}", thread.name, piece)
                    };
                // 已解决的线索移入归档区
                sections.archived.push(final_line);
            }
        }
    }

    serialize_sections(&sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_diff_block() {
        let raw = "前置文本\n<全局index更新>\n[人物] 提升: 艾莉娅(卷6)\n</全局index更新>\n后续文本";
        let body = extract_diff_block(raw).unwrap();
        assert!(body.contains("[人物] 提升: 艾莉娅"));
    }

    #[test]
    fn test_extract_volume_number() {
        assert_eq!(extract_volume_number("卷6"), Some(6));
        assert_eq!(extract_volume_number("卷12·身份揭晓"), Some(12));
        assert_eq!(extract_volume_number("没有卷号"), None);
    }

    #[test]
    fn test_parse_named_with_paren() {
        let p = parse_named_with_paren("艾莉娅(卷6·身份揭晓)");
        assert_eq!(p.name, "艾莉娅");
        assert_eq!(p.volume, Some(6));
        assert_eq!(p.note, "身份揭晓");

        let p = parse_named_with_paren("玩家");
        assert_eq!(p.name, "玩家");
        assert_eq!(p.volume, None);
        assert_eq!(p.note, "");
    }

    #[test]
    fn test_parse_index_diff() {
        let raw = r#"<全局index更新>
[人物] 提升: 艾莉娅(卷6·身份揭晓)
[人物] 仅本卷: 路过的观众
[物品] 提升: 神殿地图(卷6·转交玩家)
[地点] 提升: 古神殿(卷6·首次进入)
[线索] 艾莉娅身份 → 卷6(已解)
[线索] 神殿内部机关 → [新增·未解]
</全局index更新>"#;

        let diff = parse_index_diff(raw);

        assert_eq!(diff.entities.len(), 4);

        let ai = diff.entities.iter().find(|e| e.name == "艾莉娅").unwrap();
        assert_eq!(ai.category, EntityCategory::Character);
        assert_eq!(ai.mode, PromoteMode::Promote);
        assert_eq!(ai.volume, Some(6));
        assert_eq!(ai.note, "身份揭晓");

        let onlooker = diff
            .entities
            .iter()
            .find(|e| e.name == "路过的观众")
            .unwrap();
        assert_eq!(onlooker.mode, PromoteMode::LocalOnly);

        let map = diff.entities.iter().find(|e| e.name == "神殿地图").unwrap();
        assert_eq!(map.category, EntityCategory::Item);

        let temple = diff.entities.iter().find(|e| e.name == "古神殿").unwrap();
        assert_eq!(temple.category, EntityCategory::Location);

        assert_eq!(diff.threads.len(), 2);
        let resolved = diff
            .threads
            .iter()
            .find(|t| t.name == "艾莉娅身份")
            .unwrap();
        assert_eq!(resolved.status, ThreadStatus::Resolved);
        assert_eq!(resolved.volume, Some(6));

        let new_thread = diff
            .threads
            .iter()
            .find(|t| t.name == "神殿内部机关")
            .unwrap();
        assert_eq!(new_thread.status, ThreadStatus::NewOpen);
    }

    #[test]
    fn test_apply_diff_new_entries() {
        let initial = r#"# 全局索引

## 人物

## 物品

## 悬挂线索

## 地点

## [已归档]
"#;

        let diff = parse_index_diff(
            r#"<全局index更新>
[人物] 提升: 艾莉娅(卷2·初登场)
[物品] 提升: 神殿地图(卷2)
[线索] 艾莉娅身份 → [新增·未解]
</全局index更新>"#,
        );

        let updated = apply_diff(initial, &diff);
        assert!(updated.contains("- 艾莉娅: 卷2(初登场)"));
        assert!(updated.contains("- 神殿地图: 卷2"));
        // 新增线索没有具体卷号时也应记录
        assert!(updated.contains("艾莉娅身份"));
    }

    #[test]
    fn test_apply_diff_append_to_existing() {
        let initial = r#"# 全局索引

## 人物
- 艾莉娅: 卷2(初登场), 卷4(夜间独行)

## 物品

## 悬挂线索
- 艾莉娅身份: 卷2(提出) → [未解]

## 地点

## [已归档]
"#;

        let diff = parse_index_diff(
            r#"<全局index更新>
[人物] 提升: 艾莉娅(卷6·身份揭晓)
[线索] 艾莉娅身份 → 卷6(已解)
</全局index更新>"#,
        );

        let updated = apply_diff(initial, &diff);
        // 应该追加到已有艾莉娅条目
        assert!(updated.contains("- 艾莉娅: 卷2(初登场), 卷4(夜间独行), 卷6(身份揭晓)"));
        // 已解决的线索应移入归档区
        assert!(updated.contains("## [已归档]"));
        let archived_section = updated.split("## [已归档]").nth(1).unwrap();
        assert!(archived_section.contains("艾莉娅身份"));
        assert!(archived_section.contains("已解"));
    }

    #[test]
    fn test_local_only_not_promoted() {
        let initial = "# 全局索引\n\n## 人物\n\n## 物品\n\n## 悬挂线索\n\n## 地点\n\n## [已归档]\n";
        let diff = parse_index_diff(
            r#"<全局index更新>
[人物] 仅本卷: 路过的观众, 录音棚工程师
</全局index更新>"#,
        );
        let updated = apply_diff(initial, &diff);
        // LocalOnly 的实体绝对不应该进入全局
        assert!(!updated.contains("路过的观众"));
        assert!(!updated.contains("录音棚工程师"));
    }
}
