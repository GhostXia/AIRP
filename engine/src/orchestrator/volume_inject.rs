use aho_corasick::{AhoCorasick, MatchKind};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

use crate::{index_parser, volume_store};

static VOL_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"卷\s*(\d+)").expect("VOL_NUM_RE"));

/// 别名段：`(别名: x, y)` 或 `(aliases: x, y)`，紧跟在 canonical name 之后、冒号之前。
static ALIAS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\(\s*(?:别名|aliases?)\s*[:：]\s*([^)]*)\)").expect("ALIAS_RE"));

/// 解析出的实体记录。
#[derive(Debug, Clone, PartialEq)]
struct EntityRecord {
    /// 规范名（用于显示）。
    name: String,
    /// 别名列表（与 name 等价的关键词）。
    aliases: Vec<String>,
    /// 关联的卷号。
    vols: Vec<u32>,
}

/// 把 current.md 追加到 System Prompt 的 [Recent Context] 段。
pub fn inject_current_context(session_dir: &Path, prompt: &mut String) {
    let Ok(content) = volume_store::read_current(session_dir) else {
        return;
    };
    if content.trim().is_empty() {
        return;
    }
    prompt.push_str("\n[Recent Context]\n");
    prompt.push_str(&content);
    if !content.ends_with('\n') {
        prompt.push('\n');
    }
}

/// 根据 user_message 中出现的关键词，扫描全局 index 找到相关卷，
/// 注入对应卷的 \[卷索引\] 头部到 System Prompt 的 \[Related History\] 段。
///
/// **M5.3 关键词匹配增强**：
///   - 关键词按字符长度倒序匹配，避免 "艾" 抢走 "艾莉娅" 的命中范围。
///   - 支持别名：`- 艾莉娅 (别名: 莉娅, 小艾): 卷2(初登场)` 中的别名也参与匹配，命中后归属规范名。
///   - 不重叠去重：一段文本一旦被某个关键词命中，被该命中区间覆盖的其它关键词不再触发。
pub fn inject_volume_context(session_dir: &Path, user_message: &str, prompt: &mut String) {
    let Ok(index_md) = volume_store::read_index(session_dir) else {
        return;
    };
    let sections = index_parser::parse_sections(&index_md);

    let mut records: Vec<EntityRecord> = Vec::new();
    for line in sections
        .characters
        .iter()
        .chain(sections.items.iter())
        .chain(sections.locations.iter())
    {
        if let Some(r) = parse_entity_line(line) {
            records.push(r);
        }
    }

    let matched = collect_matched_vols(user_message, &records);
    if matched.is_empty() {
        return;
    }

    let mut sorted: Vec<u32> = matched.into_iter().collect();
    sorted.sort();

    prompt.push_str("\n[Related History]\n");
    for v in sorted {
        match volume_store::read_volume_header(session_dir, v) {
            Ok(header) => {
                prompt.push_str(&format!("--- 卷{} ---\n", v));
                prompt.push_str(header.trim_end());
                prompt.push('\n');
            }
            Err(_) => continue,
        }
    }
}

/// 解析 index.md 的一行，例如：
///   `- 艾莉娅 (别名: 莉娅, 小艾): 卷2(初登场), 卷4`
/// → name="艾莉娅", aliases=["莉娅","小艾"], vols=[2,4]
fn parse_entity_line(line: &str) -> Option<EntityRecord> {
    let l = line.trim().trim_start_matches("- ").trim();

    // 在 paren 深度为 0 处寻找冒号；否则 `(别名: ...)` 内部冒号会被误判为分隔符。
    let colon_pos = find_top_level_colon(l)?;

    let left = &l[..colon_pos];
    // colon_pos 指向 ':' 或 '：' 的字节起点；按字符跳过
    let colon_len = l[colon_pos..]
        .chars()
        .next()
        .map(|c| c.len_utf8())
        .unwrap_or(1);
    let rest = &l[colon_pos + colon_len..];

    // 别名解析（如有）
    let aliases: Vec<String> = ALIAS_RE
        .captures(left)
        .and_then(|c| c.get(1))
        .map(|m| {
            m.as_str()
                .split(',')
                .flat_map(|p| p.split('，'))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // 规范名 = left 去掉别名 paren 后再 trim
    let name_stripped = ALIAS_RE.replace_all(left, "");
    let name = name_stripped.trim().to_string();
    if name.is_empty() {
        return None;
    }

    let vols: Vec<u32> = VOL_NUM_RE
        .captures_iter(rest)
        .filter_map(|c| c.get(1).and_then(|m| m.as_str().parse::<u32>().ok()))
        .collect();

    if vols.is_empty() {
        return None;
    }

    Some(EntityRecord {
        name,
        aliases,
        vols,
    })
}

/// 找到 paren 深度为 0 时的第一个冒号（ASCII `:` 或全角 `：`）位置。
/// 返回字节偏移；找不到返回 None。
fn find_top_level_colon(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '（' => depth += 1,
            ')' | '）' if depth > 0 => depth -= 1,
            ':' | '：' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// 实现非重叠的长度倒序匹配。
///
/// **6.0p Aho-Corasick 优化**：旧实现 N 个 key × text.find 复杂度 O(N × |text|)；
/// 现把所有 key 喂给 `MatchKind::LeftmostLongest` 自动机单次扫描即得正确语义：
/// 同位置长 key 自动遮蔽短 key（"艾莉娅" > "艾"），不同位置独立命中。
fn collect_matched_vols(user_message: &str, records: &[EntityRecord]) -> HashSet<u32> {
    // 展开 (key, record_idx)
    let mut patterns: Vec<&str> = Vec::new();
    let mut pattern_to_record: Vec<usize> = Vec::new();
    for (i, r) in records.iter().enumerate() {
        if !r.name.is_empty() {
            patterns.push(r.name.as_str());
            pattern_to_record.push(i);
        }
        for a in &r.aliases {
            if !a.is_empty() {
                patterns.push(a.as_str());
                pattern_to_record.push(i);
            }
        }
    }

    if patterns.is_empty() {
        return HashSet::new();
    }

    // `LeftmostLongest`：同位置选最长 pattern，命中后自动跳过其范围 → 等价于
    // 旧实现「长度倒序 + 非重叠」语义但 O(|text|) 单次扫描。
    let ac = match AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostLongest)
        .build(&patterns)
    {
        Ok(a) => a,
        // 极端情况（pattern 集已剥空键，理论不到达）→ 退回空集，主流程跳过 Related History
        Err(_) => return HashSet::new(),
    };

    let mut matched: HashSet<u32> = HashSet::new();
    for mat in ac.find_iter(user_message) {
        let rec_idx = pattern_to_record[mat.pattern().as_usize()];
        for &v in &records[rec_idx].vols {
            matched.insert(v);
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entity_line_basic() {
        let r = parse_entity_line("- 艾莉娅: 卷2(初登场), 卷4, 卷6(身份揭晓)").unwrap();
        assert_eq!(r.name, "艾莉娅");
        assert!(r.aliases.is_empty());
        assert_eq!(r.vols, vec![2, 4, 6]);

        let r = parse_entity_line("- 神殿地图: 卷3").unwrap();
        assert_eq!(r.name, "神殿地图");
        assert_eq!(r.vols, vec![3]);

        assert!(parse_entity_line("- 不规则行").is_none());
        assert!(parse_entity_line("- 名字: 无卷号").is_none());
    }

    #[test]
    fn test_parse_entity_line_with_aliases() {
        let r = parse_entity_line("- 艾莉娅 (别名: 莉娅, 小艾): 卷2(初登场)").unwrap();
        assert_eq!(r.name, "艾莉娅");
        assert_eq!(r.aliases, vec!["莉娅".to_string(), "小艾".to_string()]);
        assert_eq!(r.vols, vec![2]);

        // 中文逗号也接受
        let r = parse_entity_line("- 莱昂 (别名: Leo，老莱): 卷5").unwrap();
        assert_eq!(r.aliases, vec!["Leo".to_string(), "老莱".to_string()]);

        // 英文 aliases 关键字
        let r = parse_entity_line("- Elias (aliases: Eli): 卷7").unwrap();
        assert_eq!(r.name, "Elias");
        assert_eq!(r.aliases, vec!["Eli".to_string()]);
    }

    #[test]
    fn test_collect_length_desc_priority() {
        // 短名 "艾" 不应抢走 "艾莉娅" 的命中范围
        let records = vec![
            EntityRecord {
                name: "艾莉娅".to_string(),
                aliases: vec![],
                vols: vec![10],
            },
            EntityRecord {
                name: "艾".to_string(),
                aliases: vec![],
                vols: vec![20],
            },
        ];
        let matched = collect_matched_vols("我去找艾莉娅", &records);
        // 长名命中 → 卷 10 ；短名被遮蔽 → 卷 20 不命中
        assert!(matched.contains(&10));
        assert!(!matched.contains(&20));
    }

    #[test]
    fn test_collect_alias_resolves_to_canonical() {
        let records = vec![EntityRecord {
            name: "艾莉娅".to_string(),
            aliases: vec!["莉娅".to_string()],
            vols: vec![3, 5],
        }];
        let matched = collect_matched_vols("莉娅最近怎么样？", &records);
        assert!(matched.contains(&3));
        assert!(matched.contains(&5));
    }

    #[test]
    fn test_collect_non_overlapping_independent_matches() {
        // 两个不同实体在文本中出现于不同位置 → 都命中
        let records = vec![
            EntityRecord {
                name: "艾莉娅".to_string(),
                aliases: vec![],
                vols: vec![1],
            },
            EntityRecord {
                name: "莱昂".to_string(),
                aliases: vec![],
                vols: vec![2],
            },
        ];
        let matched = collect_matched_vols("艾莉娅和莱昂在小镇相遇", &records);
        assert!(matched.contains(&1));
        assert!(matched.contains(&2));
    }

    #[test]
    fn test_inject_current_context() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("s1");
        volume_store::ensure_session_dirs(&session_dir).unwrap();

        let mut prompt = String::from("Base prompt.");

        inject_current_context(&session_dir, &mut prompt);
        assert_eq!(prompt, "Base prompt.");

        volume_store::append_to_current(&session_dir, "玩家走进了森林。").unwrap();
        inject_current_context(&session_dir, &mut prompt);
        assert!(prompt.contains("[Recent Context]"));
        assert!(prompt.contains("玩家走进了森林"));
    }

    #[test]
    fn test_inject_volume_context_matches_keyword() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("s1");
        volume_store::ensure_session_dirs(&session_dir).unwrap();

        let idx = r#"# 全局索引

## 人物
- 艾莉娅: 卷2(初登场), 卷4(夜间独行)

## 物品

## 悬挂线索

## 地点

## [已归档]
"#;
        volume_store::write_index(&session_dir, idx).unwrap();

        volume_store::write_volume(
            &session_dir,
            2,
            "# 卷2\n\n## [卷索引]\n- 登场: 艾莉娅\n- 事件: 初遇\n\n---\n\n正文略\n",
        )
        .unwrap();
        volume_store::write_volume(
            &session_dir,
            4,
            "# 卷4\n\n## [卷索引]\n- 登场: 艾莉娅\n- 事件: 夜间独行\n\n---\n\n正文略\n",
        )
        .unwrap();

        let mut prompt = String::new();
        inject_volume_context(&session_dir, "我去找艾莉娅聊聊", &mut prompt);
        assert!(prompt.contains("[Related History]"));
        assert!(prompt.contains("--- 卷2 ---"));
        assert!(prompt.contains("--- 卷4 ---"));
        assert!(prompt.contains("初遇"));
        assert!(!prompt.contains("正文略"));
    }

    #[test]
    fn test_inject_volume_context_alias_end_to_end() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("s1");
        volume_store::ensure_session_dirs(&session_dir).unwrap();

        let idx = "# 全局索引\n\n## 人物\n- 艾莉娅 (别名: 莉娅): 卷3\n\n## 物品\n\n## 悬挂线索\n\n## 地点\n\n## [已归档]\n";
        volume_store::write_index(&session_dir, idx).unwrap();
        volume_store::write_volume(
            &session_dir,
            3,
            "# 卷3\n\n## [卷索引]\n- 登场: 艾莉娅\n\n---\n\n正文\n",
        )
        .unwrap();

        let mut prompt = String::new();
        // 用别名提到角色 → 仍应命中卷3
        inject_volume_context(&session_dir, "莉娅今天来访", &mut prompt);
        assert!(prompt.contains("--- 卷3 ---"));
    }

    #[test]
    fn test_inject_volume_context_no_match() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let session_dir = tmp.path().join("s1");
        volume_store::ensure_session_dirs(&session_dir).unwrap();

        let idx = "# 全局索引\n\n## 人物\n- 艾莉娅: 卷2\n\n## 物品\n\n## 悬挂线索\n\n## 地点\n\n## [已归档]\n";
        volume_store::write_index(&session_dir, idx).unwrap();

        let mut prompt = String::new();
        inject_volume_context(&session_dir, "今天天气真好", &mut prompt);
        assert!(!prompt.contains("[Related History]"));
        assert!(prompt.is_empty());
    }
}
