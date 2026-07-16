use aho_corasick::{AhoCorasick, MatchKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const DEFAULT_PRIORITY: i32 = 10;

/// 世界书（Lorebook）条目。
///
/// v4 schema：`selective` 从 ST-only extensions 提升为 canonical bool 字段。
/// `selective=true` 时，primary key 命中后还需 `secondary_keys` 任一命中
/// 扫描文本才最终激活（[`Lorebook::trigger`] 消费）。`selective=false`（默认）
/// 保持旧行为：primary key 命中即激活。
///
/// v3 schema 新增 `secondary_keys` / `case_sensitive` / `extensions` 三个
/// advisory metadata 字段：它们由 [`super::worldbook_normalizer`] 从
/// SillyTavern 字段（`keysecondary` / `caseSensitive` /
/// `position` / `probability` / …）归一化而来并保留在条目里。
/// v4 后 `secondary_keys` 在 `selective=true` 时进入运行时注入管线；
/// `case_sensitive` 与 `extensions` 仍为 advisory metadata。新增字段全部带
/// `#[serde(default)]`，旧 v1/v2/v3 数据反序列化不破。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LorebookEntry {
    /// 触发关键词列表（OR 关系，任一命中即激活）。
    pub keys: Vec<String>,
    /// 命中后追加到 system prompt 的文本。
    pub content: String,
    /// 是否启用；为 `None` 视为 `true`。
    pub enabled: Option<bool>,
    /// 优先级，越大越靠前；为 `None` 默认 10。
    pub priority: Option<i32>,
    /// 是否常驻注入。`constant=true` 且 `enabled!=false` 时，无论关键词是否命中都会注入。
    /// 为 `None` 或 `false` 时走关键词触发路径。
    #[serde(default)]
    pub constant: Option<bool>,
    /// 自由注释字段。
    pub comment: Option<String>,
    /// v3：SillyTavern `keysecondary` 归一化结果。v4：`selective=true` 时
    /// 参与 trigger 二次匹配（任一命中扫描文本才激活）。
    #[serde(default)]
    pub secondary_keys: Vec<String>,
    /// v4：SillyTavern `selective` 归一化结果。`true` 时 primary key 命中后
    /// 还需 `secondary_keys` 任一命中扫描文本才激活。`false`（默认）= primary
    /// 命中即激活，与 v3 行为一致。
    #[serde(default)]
    pub selective: bool,
    /// v3：SillyTavern `caseSensitive` 归一化结果。当前 trigger 走大小写敏感
    /// 的 `AhoCorasick::LeftmostLongest` 默认；此字段仅作 advisory metadata。
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    /// v3：未在 canonical schema 内的 SillyTavern 字段（`position`
    /// / `depth` / `probability` / `sticky` / `cooldown` / `delay` / `group`
    /// / `use_regex` / `match_whole_words` / `recursion` 等）原样保留在此。
    /// v4 后 `selective` 已提升为 canonical，不再出现在 extensions。
    /// 不得把这里的字段当作已支持语义；新增运行时语义须先扩 schema 并写合同。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, serde_json::Value>>,
}

/// 世界书（Lorebook）整体结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lorebook {
    /// 全部 Lore 条目。
    pub entries: Vec<LorebookEntry>,
}

impl Lorebook {
    /// 扫描文本，触发匹配关键词的条目，返回拼接的 lore 字符串。
    ///
    /// **6.0p Aho-Corasick 优化**：旧实现对每个 entry × 每个 key 走
    /// `text.contains(k)` 双层循环，复杂度 O(entries × keys × |text|)。
    /// 现把所有 enabled entries 的 keys 扁平化为单一 DFA，文本一次扫描即
    /// 标记所有命中 entry，复杂度降为 O(build + |text|)。对于含数十
    /// entries × 数十 keys 的真实世界书，加速比一个数量级以上。
    pub fn trigger(&self, text: &str) -> String {
        let triggered = triggered_entry_indices(&self.entries, text);
        if triggered.is_empty() {
            return String::new();
        }

        let mut out = String::from("\n[World Info/Lorebook Information]:\n");
        for index in triggered {
            out.push_str(&self.entries[index].content);
            out.push('\n');
        }
        out
    }
}

fn triggered_entry_indices(entries: &[LorebookEntry], text: &str) -> Vec<usize> {
    // 1. 收集需要参与扫描的 enabled entries 的 (key, entry_idx) 扁平表。
    //    constant=true 的 entry 不需要关键词命中，跳过 pattern 收集，
    //    在第 3 步直接加入 triggered 集合。
    let mut patterns: Vec<&str> = Vec::new();
    let mut pattern_to_entry: Vec<usize> = Vec::new();
    let mut triggered_idx: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (idx, e) in entries.iter().enumerate() {
        if !e.enabled.unwrap_or(true) {
            continue;
        }
        // constant=true 且 enabled 的条目直接标记为命中，不依赖关键词扫描。
        if e.constant.unwrap_or(false) {
            triggered_idx.insert(idx);
            continue;
        }
        for k in &e.keys {
            if k.is_empty() {
                continue;
            }
            patterns.push(k.as_str());
            pattern_to_entry.push(idx);
        }
    }

    // 2. 构造 Aho-Corasick 自动机，单次扫描收集命中 entry idx。
    //    用 `LeftmostLongest` 避免「人物4」抢走「人物42」的命中范围
    //    （Standard 默认 leftmost-shortest，与世界书前缀重叠语义相悖）。
    //    空 pattern 集时跳过 build（constant 条目可能已填充 triggered_idx）。
    if !patterns.is_empty() {
        let ac = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
            .expect("AhoCorasick build patterns");
        for mat in ac.find_iter(text) {
            triggered_idx.insert(pattern_to_entry[mat.pattern().as_usize()]);
        }
    }

    // v4：selective 二次匹配。primary key 命中且 `selective=true` 的 entry
    // 还需 `secondary_keys` 任一命中扫描文本才最终激活。constant entry 不受
    // selective 影响（常驻注入不依赖关键词，也不依赖 secondary）。
    // secondary 匹配用 case-sensitive `text.contains`，与 primary DFA 的
    // 实际运行时行为一致；空 secondary key 先过滤。
    triggered_idx.retain(|&idx| {
        let e = &entries[idx];
        if !e.selective || e.constant.unwrap_or(false) {
            return true;
        }
        let has_effective_secondary = e.secondary_keys.iter().any(|k| !k.is_empty());
        if !has_effective_secondary {
            // selective=true 但无有效 secondary key：保持 primary 命中
            // （与 ST 行为一致：selective 但 keysecondary 为空 = primary 命中即激活）。
            return true;
        }
        e.secondary_keys
            .iter()
            .filter(|k| !k.is_empty())
            .any(|k| text.contains(k.as_str()))
    });

    if triggered_idx.is_empty() {
        return Vec::new();
    }

    // 3. 按 priority 从高到低排序，拼接 content。
    //    constant 与关键词命中的 entry 共用同一排序规则，去重由 HashSet 保证。
    let mut triggered: Vec<(usize, &LorebookEntry)> = triggered_idx
        .iter()
        .map(|&index| (index, &entries[index]))
        .collect();
    triggered.sort_by_key(|(index, entry)| {
        (
            std::cmp::Reverse(entry.priority.unwrap_or(DEFAULT_PRIORITY)),
            *index,
        )
    });

    let mut emitted_content = std::collections::HashSet::new();
    triggered
        .into_iter()
        .filter_map(|(index, entry)| {
            emitted_content
                .insert(entry.content.as_str())
                .then_some(index)
        })
        .collect()
}

/// One lorebook plus the logical identity of the asset that supplied its entries.
#[derive(Debug, Clone)]
pub(crate) struct SourcedLorebook {
    pub(crate) source_id: String,
    pub(crate) lorebook: Lorebook,
}

/// One activated entry after merge, priority ordering, and content deduplication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TriggeredLorebookEntry {
    pub(crate) source_id: String,
    pub(crate) item_id: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone)]
struct LorebookEntryProvenance {
    source_id: String,
    item_id: String,
}

/// A merged runtime lorebook that retains the first exact duplicate's provenance.
#[derive(Debug, Clone)]
pub(crate) struct MergedLorebook {
    lorebook: Lorebook,
    provenance: Vec<LorebookEntryProvenance>,
}

impl MergedLorebook {
    pub(crate) fn trigger(&self, text: &str) -> Vec<TriggeredLorebookEntry> {
        triggered_entry_indices(&self.lorebook.entries, text)
            .into_iter()
            .map(|index| {
                let provenance = &self.provenance[index];
                TriggeredLorebookEntry {
                    source_id: provenance.source_id.clone(),
                    item_id: provenance.item_id.clone(),
                    content: self.lorebook.entries[index].content.clone(),
                }
            })
            .collect()
    }
}

/// Merge lorebooks while retaining the logical source and original entry index.
pub(crate) fn merge_sourced_lorebooks(lorebooks: &[SourcedLorebook]) -> MergedLorebook {
    let mut merged: Vec<(LorebookEntry, LorebookEntryProvenance)> = Vec::new();

    for sourced in lorebooks {
        for (index, entry) in sourced.lorebook.entries.iter().enumerate() {
            if !merged
                .iter()
                .any(|(existing, _)| entries_are_semantic_duplicates(existing, entry))
            {
                merged.push((
                    entry.clone(),
                    LorebookEntryProvenance {
                        source_id: sourced.source_id.clone(),
                        item_id: index.to_string(),
                    },
                ));
            }
        }
    }

    merged.sort_by(|a, b| {
        let pa = a.0.priority.unwrap_or(DEFAULT_PRIORITY);
        let pb = b.0.priority.unwrap_or(DEFAULT_PRIORITY);
        pb.cmp(&pa)
    });

    let (entries, provenance) = merged.into_iter().unzip();
    MergedLorebook {
        lorebook: Lorebook { entries },
        provenance,
    }
}

fn entries_are_semantic_duplicates(a: &LorebookEntry, b: &LorebookEntry) -> bool {
    a.content == b.content
        && a.keys == b.keys
        && a.enabled.unwrap_or(true) == b.enabled.unwrap_or(true)
        && a.priority.unwrap_or(DEFAULT_PRIORITY) == b.priority.unwrap_or(DEFAULT_PRIORITY)
        && a.constant.unwrap_or(false) == b.constant.unwrap_or(false)
        && a.comment == b.comment
        && a.secondary_keys == b.secondary_keys
        && a.selective == b.selective
        && a.case_sensitive == b.case_sensitive
        && a.extensions == b.extensions
}

/// MS-5: Merge multiple lorebooks without discarding distinct activation semantics.
/// Exact semantic duplicates are removed; output content is deduplicated after trigger evaluation.
pub fn merge_lorebooks(lorebooks: &[Lorebook]) -> Lorebook {
    let mut merged: Vec<LorebookEntry> = Vec::new();

    for lb in lorebooks {
        for entry in &lb.entries {
            if !merged
                .iter()
                .any(|existing| entries_are_semantic_duplicates(existing, entry))
            {
                merged.push(entry.clone());
            }
        }
    }

    // Sort by priority descending (None = DEFAULT_PRIORITY)
    merged.sort_by(|a, b| {
        let pa = a.priority.unwrap_or(DEFAULT_PRIORITY);
        let pb = b.priority.unwrap_or(DEFAULT_PRIORITY);
        pb.cmp(&pa)
    });

    Lorebook { entries: merged }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(keys: &[&str], content: &str, priority: Option<i32>) -> LorebookEntry {
        LorebookEntry {
            keys: keys.iter().map(|s| s.to_string()).collect(),
            content: content.to_string(),
            enabled: None,
            priority,
            constant: None,
            comment: None,
            secondary_keys: Vec::new(),
            selective: false,
            case_sensitive: None,
            extensions: None,
        }
    }

    #[test]
    fn trigger_single_match() {
        let lb = Lorebook {
            entries: vec![entry(&["艾莉娅"], "艾莉娅是冒险者", None)],
        };
        let out = lb.trigger("我去找艾莉娅");
        assert!(out.contains("艾莉娅是冒险者"));
    }

    #[test]
    fn trigger_no_match_returns_empty() {
        let lb = Lorebook {
            entries: vec![entry(&["龙王"], "...", None)],
        };
        assert_eq!(lb.trigger("今天天气不错"), "");
    }

    #[test]
    fn trigger_disabled_entry_ignored() {
        let lb = Lorebook {
            entries: vec![LorebookEntry {
                keys: vec!["艾莉娅".to_string()],
                content: "should not appear".to_string(),
                enabled: Some(false),
                priority: None,
                constant: None,
                comment: None,
                secondary_keys: Vec::new(),
                selective: false,
                case_sensitive: None,
                extensions: None,
            }],
        };
        assert_eq!(lb.trigger("艾莉娅来了"), "");
    }

    #[test]
    fn trigger_priority_sort_desc() {
        let lb = Lorebook {
            entries: vec![
                entry(&["A"], "low-prio-A", Some(1)),
                entry(&["B"], "high-prio-B", Some(100)),
            ],
        };
        let out = lb.trigger("A 和 B");
        // 高优先级 B 必须出现在 A 之前
        let pos_a = out.find("low-prio-A").unwrap();
        let pos_b = out.find("high-prio-B").unwrap();
        assert!(pos_b < pos_a, "priority 100 应排在 1 之前: {}", out);
    }

    #[test]
    fn trigger_multiple_keys_or_semantic() {
        // 一个 entry 多 key，OR 关系
        let lb = Lorebook {
            entries: vec![entry(&["龙王", "Dragon Lord"], "传说中的龙王", None)],
        };
        assert!(lb.trigger("Dragon Lord 现身").contains("传说中的龙王"));
        assert!(lb.trigger("龙王降临").contains("传说中的龙王"));
        assert_eq!(lb.trigger("普通的一天"), "");
    }

    #[test]
    fn trigger_skips_empty_keys() {
        // 空 key 不应误命中（旧实现已守护）
        let lb = Lorebook {
            entries: vec![entry(&["", "实key"], "命中", None)],
        };
        assert!(lb.trigger("含实key 的文本").contains("命中"));
        assert_eq!(lb.trigger("无关文本"), "");
    }

    #[test]
    fn airp_v1_contract_fixture_has_deterministic_output() {
        let fixture = include_str!("../../tests/fixtures/worldbook/airp-v1-basic.json");
        let lorebook: Lorebook = serde_json::from_str(fixture).unwrap();
        let output = lorebook
            .trigger("The moon gate opens near the old observatory; disabled is mentioned.");
        assert_eq!(
            output,
            "\n[World Info/Lorebook Information]:\nThe observatory predates the city.\nThe moon gate opens only at night.\n"
        );
        assert!(!output.contains("This must never be injected."));
    }

    // ── v2 constant semantic tests ────────────────────────────────────────

    fn entry_with_constant(
        keys: &[&str],
        content: &str,
        priority: Option<i32>,
        constant: Option<bool>,
    ) -> LorebookEntry {
        LorebookEntry {
            keys: keys.iter().map(|s| s.to_string()).collect(),
            content: content.to_string(),
            enabled: None,
            priority,
            constant,
            comment: None,
            secondary_keys: Vec::new(),
            selective: false,
            case_sensitive: None,
            extensions: None,
        }
    }

    #[test]
    fn constant_entry_injects_without_keyword_match() {
        // constant=true 且 enabled 的条目，即使 keys 为空也应注入。
        let lb = Lorebook {
            entries: vec![entry_with_constant(&[], "always-on lore", None, Some(true))],
        };
        let out = lb.trigger("一段完全无关的文本");
        assert!(
            out.contains("always-on lore"),
            "constant entry must inject without keyword match: {}",
            out
        );
    }

    #[test]
    fn disabled_constant_entry_is_skipped() {
        // enabled=false 的 constant 条目不得注入。
        let lb = Lorebook {
            entries: vec![LorebookEntry {
                keys: vec![],
                content: "must not appear".to_string(),
                enabled: Some(false),
                priority: None,
                constant: Some(true),
                comment: None,
                secondary_keys: Vec::new(),
                selective: false,
                case_sensitive: None,
                extensions: None,
            }],
        };
        assert_eq!(lb.trigger("任何文本"), "");
    }

    #[test]
    fn constant_and_keyword_entries_coexist() {
        // constant 条目与关键词命中条目共存，各自按 priority 排序。
        let lb = Lorebook {
            entries: vec![
                entry_with_constant(&[], "constant-lore-low", Some(5), Some(true)),
                entry_with_constant(&["keyword"], "keyword-lore-high", Some(20), None),
            ],
        };
        let out = lb.trigger("含 keyword 的文本");
        assert!(out.contains("constant-lore-low"));
        assert!(out.contains("keyword-lore-high"));
        // priority 20 的 keyword 条目应排在 priority 5 的 constant 条目之前
        let pos_high = out.find("keyword-lore-high").unwrap();
        let pos_low = out.find("constant-lore-low").unwrap();
        assert!(pos_high < pos_low, "priority order: {}", out);
    }

    #[test]
    fn constant_entry_with_keys_injects_once() {
        // constant=true 且有 keys 的条目，即使 keys 命中也只注入一次。
        let lb = Lorebook {
            entries: vec![entry_with_constant(
                &["dragon"],
                "dragon lore",
                None,
                Some(true),
            )],
        };
        let out = lb.trigger("the dragon appears");
        let count = out.matches("dragon lore").count();
        assert_eq!(count, 1, "constant entry must inject exactly once: {}", out);
    }

    #[test]
    fn constant_false_falls_back_to_keyword_trigger() {
        // constant=false 的条目必须依赖关键词命中。
        let lb = Lorebook {
            entries: vec![entry_with_constant(
                &["missing-keyword"],
                "should not appear",
                None,
                Some(false),
            )],
        };
        assert_eq!(lb.trigger("无关文本"), "");
    }

    #[test]
    fn airp_v2_constant_fixture_has_deterministic_output() {
        let fixture = include_str!("../../tests/fixtures/worldbook/airp-v2-constant.json");
        let lorebook: Lorebook = serde_json::from_str(fixture).unwrap();
        // 扫描文本命中 "moon gate" 和 "observatory"，但不命中 "marketplace"
        let output = lorebook.trigger("The moon gate opens near the old observatory.");
        // 期望：2 个 constant 条目（dragon compact + marketplace）+ 2 个关键词命中
        // 按 priority 降序：dragon compact(30) > observatory(20) > marketplace(5) == moon gate(10)
        // 优先级：30, 20, 10, 5
        assert_eq!(
            output,
            "\n[World Info/Lorebook Information]:\n\
The world is shaped by an ancient compact between dragons and mortals.\n\
The observatory predates the city.\n\
The moon gate opens only at night.\n\
The marketplace bustles at dawn.\n"
        );
        // disabled constant 条目不得出现
        assert!(!output.contains("This disabled constant must never appear."));
    }

    #[test]
    fn empty_lorebook_returns_empty_string() {
        // 无任何条目时返回空串
        let lb = Lorebook { entries: vec![] };
        assert_eq!(lb.trigger("任何文本"), "");
    }

    #[test]
    fn only_constant_entries_inject_on_unrelated_text() {
        // 只有 constant 条目、文本完全不相关时仍应注入
        let lb = Lorebook {
            entries: vec![entry_with_constant(&[], "always-on", None, Some(true))],
        };
        assert!(lb.trigger("").contains("always-on"));
    }

    /// 6.0p 基准对比：朴素 substring 双层循环 vs Aho-Corasick。
    /// `cargo test -- --ignored bench_aho_corasick_vs_naive` 触发；
    /// 不进 CI 跑路径以免抖动失败。
    #[test]
    #[ignore]
    fn bench_aho_corasick_vs_naive() {
        use std::time::Instant;
        // 构造 SillyTavern 级世界书：500 entries × 平均 3 keys = 1500 patterns
        let mut entries = Vec::with_capacity(500);
        for i in 0..500 {
            entries.push(entry(
                &[
                    &format!("人物{}", i),
                    &format!("alias{}_a", i),
                    &format!("alias{}_b", i),
                ],
                &format!("人物{}的设定", i),
                None,
            ));
        }
        let lb = Lorebook { entries };

        // 一段 4KB 的对话文本（模拟实际 user message + history 注入后的扫描目标）
        let mut text = String::with_capacity(4096);
        for _ in 0..200 {
            text.push_str("user沿着山道走了很久，看到远方的灯火，心中升起一丝期待。");
        }
        text.push_str(" 人物42 与 alias88_b 出现 ");

        // Aho-Corasick 实现（已在 trigger 中）
        let iter = 50;
        let t0 = Instant::now();
        for _ in 0..iter {
            let _ = lb.trigger(&text);
        }
        let ac_elapsed = t0.elapsed();

        // 朴素实现对照（旧 text.contains 双层循环逻辑）
        let t0 = Instant::now();
        for _ in 0..iter {
            let mut count = 0;
            for e in &lb.entries {
                if !e.enabled.unwrap_or(true) {
                    continue;
                }
                if e.keys
                    .iter()
                    .any(|k| !k.is_empty() && text.contains(k.as_str()))
                {
                    count += 1;
                }
            }
            std::hint::black_box(count);
        }
        let naive_elapsed = t0.elapsed();

        eprintln!(
            "[6.0p bench] {} iters / 500 entries × 3 keys / 4 KiB text\n  \
             Aho-Corasick : {:?}\n  Naive loop   : {:?}\n  Speedup      : {:.2}x",
            iter,
            ac_elapsed,
            naive_elapsed,
            naive_elapsed.as_secs_f64() / ac_elapsed.as_secs_f64()
        );
        // 不硬性断言加速比 — debug 构建 Aho-Corasick 自身开销大；release 才显著
        // 仅断言两者都跑完 + 命中正确
        assert!(naive_elapsed.as_micros() > 0);
        assert!(ac_elapsed.as_micros() > 0);
    }

    #[test]
    fn trigger_stress_many_entries() {
        // 6.0p 性能压力测试：200 entries × 3 keys，单次扫描应远快于双层循环
        // 此测试主要验证正确性 + 编译期保证（criterion 基准独立）
        let mut entries = Vec::new();
        for i in 0..200 {
            entries.push(entry(
                &[
                    &format!("人物{}", i),
                    &format!("alias{}_a", i),
                    &format!("alias{}_b", i),
                ],
                &format!("人物{}的设定", i),
                None,
            ));
        }
        let lb = Lorebook { entries };
        let text = "人物42 与 alias88_b 在 alias199_a 处相遇";
        let out = lb.trigger(text);
        assert!(out.contains("人物42的设定"));
        assert!(out.contains("人物88的设定"));
        assert!(out.contains("人物199的设定"));
        // 未提及 entry 不应触发
        assert!(!out.contains("人物0的设定"));
    }

    // MS-5 tests

    #[test]
    fn test_ms5_merge_lorebooks_preserves_distinct_trigger_semantics() {
        let lb1 = Lorebook {
            entries: vec![entry(&["A"], "shared content", Some(10))],
        };
        let lb2 = Lorebook {
            entries: vec![entry(&["B"], "shared content", Some(20))],
        };
        let merged = super::merge_lorebooks(&[lb1, lb2]);
        assert_eq!(merged.entries.len(), 2);
        assert_eq!(
            merged.trigger("A and B").matches("shared content").count(),
            1
        );
    }

    #[test]
    fn test_ms5_merge_lorebooks_deduplicates_exact_semantic_duplicates() {
        let duplicate = entry(&["A"], "shared content", Some(10));
        let merged = super::merge_lorebooks(&[
            Lorebook {
                entries: vec![duplicate.clone()],
            },
            Lorebook {
                entries: vec![duplicate],
            },
        ]);
        assert_eq!(merged.entries.len(), 1);
    }

    #[test]
    fn merge_lorebooks_preserves_distinct_advisory_metadata() {
        let first = entry(&["A"], "shared content", Some(10));
        let mut second = first.clone();
        second.secondary_keys = vec!["secondary".to_string()];

        let merged = super::merge_lorebooks(&[
            Lorebook {
                entries: vec![first],
            },
            Lorebook {
                entries: vec![second],
            },
        ]);

        assert_eq!(merged.entries.len(), 2);
        assert_eq!(merged.trigger("A").matches("shared content").count(), 1);
    }

    #[test]
    fn merged_disabled_keyword_does_not_suppress_enabled_constant() {
        let mut disabled = entry(&["keyword"], "shared content", Some(20));
        disabled.enabled = Some(false);
        let mut constant = entry(&[], "shared content", Some(10));
        constant.constant = Some(true);

        let merged = super::merge_lorebooks(&[
            Lorebook {
                entries: vec![disabled],
            },
            Lorebook {
                entries: vec![constant],
            },
        ]);

        assert_eq!(
            merged
                .trigger("unrelated text")
                .matches("shared content")
                .count(),
            1
        );
    }

    #[test]
    fn test_ms5_merge_lorebooks_preserves_all_unique() {
        let lb1 = Lorebook {
            entries: vec![entry(&["A"], "content A", Some(5))],
        };
        let lb2 = Lorebook {
            entries: vec![entry(&["B"], "content B", Some(50))],
        };
        let merged = super::merge_lorebooks(&[lb1, lb2]);
        assert_eq!(merged.entries.len(), 2);
        // Higher priority B should come first
        assert_eq!(merged.entries[0].content, "content B");
        assert_eq!(merged.entries[1].content, "content A");
    }

    #[test]
    fn test_ms5_merge_lorebooks_empty() {
        let merged = super::merge_lorebooks(&[]);
        assert!(merged.entries.is_empty());
    }

    // ── v4 selective semantic tests ─────────────────────────────────────

    fn entry_selective(
        keys: &[&str],
        content: &str,
        priority: Option<i32>,
        secondary_keys: &[&str],
        selective: bool,
    ) -> LorebookEntry {
        LorebookEntry {
            keys: keys.iter().map(|s| s.to_string()).collect(),
            content: content.to_string(),
            enabled: None,
            priority,
            constant: None,
            comment: None,
            secondary_keys: secondary_keys.iter().map(|s| s.to_string()).collect(),
            selective,
            case_sensitive: None,
            extensions: None,
        }
    }

    #[test]
    fn selective_entry_activates_when_secondary_matches() {
        let lb = Lorebook {
            entries: vec![entry_selective(
                &["dragon"],
                "dragon lore",
                None,
                &["wyrm", "scaled"],
                true,
            )],
        };
        // primary + secondary 都命中 → 激活
        assert!(lb
            .trigger("the dragon is a scaled wyrm")
            .contains("dragon lore"));
    }

    #[test]
    fn selective_entry_suppressed_when_secondary_not_matching() {
        let lb = Lorebook {
            entries: vec![entry_selective(
                &["dragon"],
                "dragon lore",
                None,
                &["wyrm"],
                true,
            )],
        };
        // primary 命中但 secondary 未命中 → 抑制
        assert_eq!(lb.trigger("the dragon flies away"), "");
    }

    #[test]
    fn selective_entry_with_empty_secondary_activates_on_primary() {
        // selective=true 但 secondary_keys 为空（或全空串）→ 保持 primary 命中
        let lb = Lorebook {
            entries: vec![entry_selective(
                &["moon gate"],
                "moon gate lore",
                None,
                &[],
                true,
            )],
        };
        assert!(lb.trigger("the moon gate opens").contains("moon gate lore"));
    }

    #[test]
    fn selective_entry_with_all_empty_secondary_keys_activates_on_primary() {
        // secondary_keys 全是空串 → 等效于无有效 secondary → 保持 primary 命中
        let lb = Lorebook {
            entries: vec![entry_selective(
                &["moon gate"],
                "moon gate lore",
                None,
                &["", ""],
                true,
            )],
        };
        assert!(lb.trigger("the moon gate opens").contains("moon gate lore"));
    }

    #[test]
    fn non_selective_entry_ignores_secondary_keys() {
        // selective=false → secondary_keys 不参与匹配，primary 命中即激活
        let lb = Lorebook {
            entries: vec![entry_selective(
                &["observatory"],
                "observatory lore",
                None,
                &["never_matches"],
                false,
            )],
        };
        assert!(lb
            .trigger("the observatory stands tall")
            .contains("observatory lore"));
    }

    #[test]
    fn constant_entry_ignores_selective() {
        // constant=true + selective=true → 常驻注入，selective 不影响
        let lb = Lorebook {
            entries: vec![LorebookEntry {
                keys: vec![],
                content: "always-on lore".to_string(),
                enabled: None,
                priority: None,
                constant: Some(true),
                comment: None,
                secondary_keys: vec!["never_matches".to_string()],
                selective: true,
                case_sensitive: None,
                extensions: None,
            }],
        };
        assert!(lb.trigger("unrelated text").contains("always-on lore"));
    }

    #[test]
    fn selective_secondary_matches_any_key_or_semantic() {
        // 多个 secondary key，任一命中即激活（OR 关系）
        let lb = Lorebook {
            entries: vec![entry_selective(
                &["dragon"],
                "dragon lore",
                None,
                &["wyrm", "scaled", "ancient"],
                true,
            )],
        };
        assert!(lb.trigger("an ancient dragon").contains("dragon lore"));
        assert!(lb.trigger("a scaled dragon").contains("dragon lore"));
        assert!(lb.trigger("a wyrm dragon").contains("dragon lore"));
        // primary 命中但无任一 secondary → 抑制
        assert_eq!(lb.trigger("a dragon flies"), "");
    }

    #[test]
    fn airp_v4_selective_fixture_has_deterministic_output() {
        let fixture = include_str!("../../tests/fixtures/worldbook/airp-v4-selective.json");
        let lorebook: Lorebook = serde_json::from_str(fixture).unwrap();
        // 扫描文本命中 "dragon"（primary）+ "scaled"（secondary）→ selective 激活
        // 命中 "moon gate"（primary，selective 但无 secondary）→ 激活
        // 命中 "observatory"（primary，non-selective）→ 激活
        // 命中 "marketplace"（primary，selective 但 secondary "midnight" 未命中）→ 抑制
        // constant 条目（dragons compact, selective=true）→ 常驻激活
        let output = lorebook.trigger("The dragon is a scaled beast near the moon gate and observatory, plus the marketplace.");
        // priority 降序：dragon(30) > compact(25) > moon gate(20) > observatory(10)
        // marketplace(5) 被抑制
        assert_eq!(
            output,
            "\n[World Info/Lorebook Information]:\n\
Dragons are ancient beings of fire.\n\
The world is shaped by an ancient compact between dragons and mortals.\n\
The moon gate opens only at night.\n\
The observatory predates the city.\n"
        );
        // marketplace 被抑制
        assert!(!output.contains("The marketplace bustles at dawn."));
    }

    #[test]
    fn merge_lorebooks_preserves_distinct_selective() {
        let first = entry(&["A"], "shared content", Some(10));
        let mut second = first.clone();
        second.selective = true;

        let merged = super::merge_lorebooks(&[
            Lorebook {
                entries: vec![first],
            },
            Lorebook {
                entries: vec![second],
            },
        ]);

        // selective 不同 → 不去重，保留两条
        assert_eq!(merged.entries.len(), 2);
    }

    #[test]
    fn sourced_merge_preserves_origin_and_original_entry_index() {
        let merged = merge_sourced_lorebooks(&[
            SourcedLorebook {
                source_id: "character:alice".to_string(),
                lorebook: Lorebook {
                    entries: vec![
                        entry(&["low"], "alice low", Some(1)),
                        entry(&["high"], "alice high", Some(30)),
                    ],
                },
            },
            SourcedLorebook {
                source_id: "scene:forest".to_string(),
                lorebook: Lorebook {
                    entries: vec![entry(&["scene"], "scene middle", Some(20))],
                },
            },
        ]);

        let triggered = merged.trigger("low high scene");
        assert_eq!(
            triggered,
            vec![
                TriggeredLorebookEntry {
                    source_id: "character:alice".to_string(),
                    item_id: "1".to_string(),
                    content: "alice high".to_string(),
                },
                TriggeredLorebookEntry {
                    source_id: "scene:forest".to_string(),
                    item_id: "0".to_string(),
                    content: "scene middle".to_string(),
                },
                TriggeredLorebookEntry {
                    source_id: "character:alice".to_string(),
                    item_id: "0".to_string(),
                    content: "alice low".to_string(),
                },
            ]
        );
    }

    #[test]
    fn sourced_merge_assigns_exact_duplicate_to_first_source() {
        let duplicate = entry(&["shared"], "same", Some(10));
        let merged = merge_sourced_lorebooks(&[
            SourcedLorebook {
                source_id: "character:alice".to_string(),
                lorebook: Lorebook {
                    entries: vec![duplicate.clone()],
                },
            },
            SourcedLorebook {
                source_id: "scene:forest".to_string(),
                lorebook: Lorebook {
                    entries: vec![duplicate],
                },
            },
        ]);

        let triggered = merged.trigger("shared");
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].source_id, "character:alice");
        assert_eq!(triggered[0].item_id, "0");
    }
}
