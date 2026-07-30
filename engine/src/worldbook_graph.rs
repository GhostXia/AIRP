//! Phase 4.4: 世界书知识图谱（worldbook knowledge graph）。
//!
//! 端点：`GET /v1/characters/:id/lorebook/graph`
//!
//! 分析 lorebook 条目间的关系：
//! 1. **key 重叠**：两条 entry 共享某个 key（节点-节点 边）
//! 2. **content 引用**：entry A 的 content 中出现 entry B 的 key（A→B 有向边）
//! 3. **冲突检测**：多条 entry 的 content 中同时引用同一个 key（可能设定冲突）
//!
//! 输出 JSON 节点/边数组，供 WebUI 力导向图渲染（如 d3-force / vis-network）。
//!
//! ## 设计要点
//! - **只读分析**：不修改 lorebook 数据
//! - **AIRP 独立实现**：分析逻辑自行设计，不复用第三方代码
//! - **大小写敏感**：与 lorebook trigger 行为对齐（LeftmostLongest 默认大小写敏感）
//! - **去重**：同一对节点间的多条关系合并为一条带 weight 的边
//! - **常驻节点标记**：constant=true 的 entry 在节点上标记，便于 UI 高亮

use crate::error::AirpError;
use crate::orchestrator::lorebook::{Lorebook, LorebookEntry};
use serde::{Deserialize, Serialize};

/// 知识图谱节点（对应一个 lorebook entry）。
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    /// entry 索引（0-based）。
    pub id: usize,
    /// 节点标签（用 comment 或第一 key）。
    pub label: String,
    /// 主关键词列表。
    pub keys: Vec<String>,
    /// 次关键词列表。
    pub secondary_keys: Vec<String>,
    /// 是否常驻注入。
    pub constant: bool,
    /// 是否启用。
    pub enabled: bool,
    /// content 长度（字符数）。
    pub content_length: usize,
    /// 优先级。
    pub priority: i32,
}

/// 知识图谱边（节点间关系）。
#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    /// 起点节点 id。
    pub source: usize,
    /// 终点节点 id。
    pub target: usize,
    /// 关系类型：`key_overlap`（共享 key，无向）/ `reference`（content 引用 key，有向）。
    pub kind: EdgeKind,
    /// 权重：共享 key 数量或引用次数。
    pub weight: usize,
    /// 共享的具体 key 列表（便于 UI tooltip 展示）。
    pub shared_keys: Vec<String>,
}

/// 边类型。
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// 共享 key（无向）。
    KeyOverlap,
    /// content 引用 key（有向）。
    Reference,
}

/// 完整知识图谱响应。
#[derive(Debug, Clone, Serialize)]
pub struct WorldbookGraph {
    /// 角色 ID。
    pub character_id: String,
    /// 节点数。
    pub node_count: usize,
    /// 边数。
    pub edge_count: usize,
    /// 冲突警告（content 中引用了同一 key 的多条 entry）。
    pub conflicts: Vec<ConflictWarning>,
    /// 节点列表。
    pub nodes: Vec<GraphNode>,
    /// 边列表。
    pub edges: Vec<GraphEdge>,
}

/// 冲突警告：多条 entry 的 content 中引用了同一 key。
#[derive(Debug, Clone, Serialize)]
pub struct ConflictWarning {
    /// 被引用的 key。
    pub key: String,
    /// 引用该 key 的 entry 索引列表。
    pub entry_indices: Vec<usize>,
    /// 提示信息。
    pub message: String,
}

/// 请求参数（可选筛选）。
#[derive(Debug, Clone, Deserialize)]
pub struct GraphQuery {
    /// 是否包含 reference 边（默认 true）。
    #[serde(default = "default_true")]
    pub include_references: bool,
    /// 是否包含 key_overlap 边（默认 true）。
    #[serde(default = "default_true")]
    pub include_key_overlap: bool,
    /// 是否检测冲突（默认 true）。
    #[serde(default = "default_true")]
    pub detect_conflicts: bool,
    /// 最小权重阈值（默认 1，即至少 1 次共享/引用）。
    #[serde(default = "default_min_weight")]
    pub min_weight: usize,
    /// #324 N8: lorebook 条目数上限（默认 500）。
    /// 超过此上限的图谱分析请求返回 400 BadRequest，防止 O(n²) 图谱分析 DoS。
    /// 调用方可通过 query 参数 `max_entries=N` 调整（例如分析大 lorebook 时调高）。
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

impl Default for GraphQuery {
    fn default() -> Self {
        Self {
            include_references: true,
            include_key_overlap: true,
            detect_conflicts: true,
            min_weight: 1,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_min_weight() -> usize {
    1
}

/// #324 N8: 默认 lorebook 条目数上限。
const DEFAULT_MAX_ENTRIES: usize = 500;

fn default_max_entries() -> usize {
    DEFAULT_MAX_ENTRIES
}

/// 从 Lorebook 构建知识图谱。
pub fn build_graph(
    character_id: &str,
    lorebook: &Lorebook,
    query: &GraphQuery,
) -> Result<WorldbookGraph, AirpError> {
    let entries = &lorebook.entries;
    // #324 N8: 上限从 hardcoded 500 改为 query.max_entries（默认 500，可调）。
    if entries.len() > query.max_entries {
        return Err(AirpError::BadRequest(format!(
            "lorebook 条目数过多（{}），知识图谱分析上限 {}",
            entries.len(),
            query.max_entries
        )));
    }

    // 1. 构建节点
    let nodes: Vec<GraphNode> = entries
        .iter()
        .enumerate()
        .map(|(idx, e)| GraphNode {
            id: idx,
            label: derive_label(e, idx),
            keys: e.keys.clone(),
            secondary_keys: e.secondary_keys.clone(),
            constant: e.constant.unwrap_or(false),
            enabled: e.enabled.unwrap_or(true),
            content_length: e.content.chars().count(),
            priority: e.priority.unwrap_or(10),
        })
        .collect();

    // 2. 构建 key → entry indices 反查表（仅 enabled 条目）
    let mut key_to_entries: std::collections::HashMap<&str, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, e) in entries.iter().enumerate() {
        if !e.enabled.unwrap_or(true) {
            continue;
        }
        for k in &e.keys {
            if !k.is_empty() {
                key_to_entries.entry(k.as_str()).or_default().push(idx);
            }
        }
    }

    // 3. key_overlap 边：共享同一 key 的 entry 对
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_pairs: std::collections::HashSet<(usize, usize, EdgeKind)> =
        std::collections::HashSet::new();

    if query.include_key_overlap {
        // 遍历 (key, indices) 对：保留 key 用于正确填充 shared_keys。
        // 之前的 `for indices in key_to_entries.values()` 实现存在两个 bug：
        //   (1) 首次创建边时用 `entries[a].keys.find(|k| entries[b].keys.contains(k))`
        //       查找"某个"共享 key，但实际多个共享 key 的边只能记录第一个，
        //       后续重复 pair 仅 weight+1 不再 push。
        //   (2) weight=N 的边 shared_keys 长度始终为 1，与字段文档
        //       "共享的具体 key 列表" 矛盾，UI tooltip 会显示缺失的 key。
        // 改为按 (key, indices) 遍历，能正确把所有共享 key 累积到 shared_keys。
        for (shared_key, indices) in &key_to_entries {
            if indices.len() < 2 {
                continue;
            }
            // 两两组合（无向，用有序对去重）
            for i in 0..indices.len() {
                for j in (i + 1)..indices.len() {
                    let a = indices[i].min(indices[j]);
                    let b = indices[i].max(indices[j]);
                    let key = (a, b, EdgeKind::KeyOverlap);
                    if !seen_pairs.insert(key) {
                        // 同一对节点已建过边：weight+1 并把当前 key 追加到 shared_keys
                        if let Some(edge) = edges.iter_mut().find(|e| {
                            e.source == a && e.target == b && e.kind == EdgeKind::KeyOverlap
                        }) {
                            edge.weight += 1;
                            let k = (*shared_key).to_string();
                            if !edge.shared_keys.contains(&k) {
                                edge.shared_keys.push(k);
                            }
                        }
                    } else {
                        edges.push(GraphEdge {
                            source: a,
                            target: b,
                            kind: EdgeKind::KeyOverlap,
                            weight: 1,
                            shared_keys: vec![(*shared_key).to_string()],
                        });
                    }
                }
            }
        }
    }

    // 4. reference 边：entry A.content 中出现 entry B 的 key（A→B 有向）
    if query.include_references {
        for (a_idx, a) in entries.iter().enumerate() {
            if !a.enabled.unwrap_or(true) || a.content.is_empty() {
                continue;
            }
            let content = &a.content;
            for (b_idx, b) in entries.iter().enumerate() {
                if a_idx == b_idx {
                    continue;
                }
                if !b.enabled.unwrap_or(true) {
                    continue;
                }
                // 统计 b 的 keys 在 a.content 中出现的次数
                let mut hits = 0usize;
                let mut hit_keys = Vec::new();
                for k in &b.keys {
                    if k.is_empty() {
                        continue;
                    }
                    if content.contains(k.as_str()) {
                        hits += 1;
                        hit_keys.push(k.clone());
                    }
                }
                if hits == 0 {
                    continue;
                }
                let key = (a_idx, b_idx, EdgeKind::Reference);
                if !seen_pairs.insert(key) {
                    if let Some(edge) = edges.iter_mut().find(|e| {
                        e.source == a_idx && e.target == b_idx && e.kind == EdgeKind::Reference
                    }) {
                        edge.weight += hits;
                        for k in hit_keys {
                            if !edge.shared_keys.contains(&k) {
                                edge.shared_keys.push(k);
                            }
                        }
                    }
                } else {
                    edges.push(GraphEdge {
                        source: a_idx,
                        target: b_idx,
                        kind: EdgeKind::Reference,
                        weight: hits,
                        shared_keys: hit_keys,
                    });
                }
            }
        }
    }

    // 5. 应用 min_weight 过滤
    if query.min_weight > 1 {
        edges.retain(|e| e.weight >= query.min_weight);
    }

    // 6. 冲突检测：同一 key 被 ≥3 条 entry 的 content 引用
    let mut conflicts: Vec<ConflictWarning> = Vec::new();
    if query.detect_conflicts {
        let mut key_referrers: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::new();
        for (idx, e) in entries.iter().enumerate() {
            if !e.enabled.unwrap_or(true) || e.content.is_empty() {
                continue;
            }
            for k in key_to_entries.keys() {
                if e.content.contains(k) {
                    key_referrers.entry(k).or_default().push(idx);
                }
            }
        }
        for (k, referrers) in key_referrers {
            if referrers.len() >= 3 {
                let unique: Vec<usize> = {
                    let mut s = referrers.clone();
                    s.sort_unstable();
                    s.dedup();
                    s
                };
                conflicts.push(ConflictWarning {
                    key: k.to_string(),
                    entry_indices: unique,
                    message: format!(
                        "key '{}' 被 {} 条 entry 的 content 同时引用，可能存在设定冲突",
                        k,
                        referrers.len()
                    ),
                });
            }
        }
        conflicts.sort_by_key(|c| std::cmp::Reverse(c.entry_indices.len()));
    }

    let edge_count = edges.len();
    Ok(WorldbookGraph {
        character_id: character_id.to_string(),
        node_count: nodes.len(),
        edge_count,
        conflicts,
        nodes,
        edges,
    })
}

/// 推导节点标签：优先 comment，否则用第一个 key，否则用索引。
fn derive_label(entry: &LorebookEntry, idx: usize) -> String {
    if let Some(c) = &entry.comment {
        if !c.trim().is_empty() {
            return c.clone();
        }
    }
    if let Some(k) = entry.keys.first() {
        if !k.is_empty() {
            return k.clone();
        }
    }
    format!("#{}", idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(keys: Vec<&str>, content: &str, comment: Option<&str>) -> LorebookEntry {
        LorebookEntry {
            keys: keys.into_iter().map(String::from).collect(),
            content: content.to_string(),
            enabled: Some(true),
            priority: Some(10),
            constant: Some(false),
            comment: comment.map(String::from),
            secondary_keys: Vec::new(),
            selective: false,
            case_sensitive: None,
            extensions: None,
        }
    }

    fn make_entry_string_keys(
        keys: Vec<String>,
        content: &str,
        comment: Option<&str>,
    ) -> LorebookEntry {
        LorebookEntry {
            keys,
            content: content.to_string(),
            enabled: Some(true),
            priority: Some(10),
            constant: Some(false),
            comment: comment.map(String::from),
            secondary_keys: Vec::new(),
            selective: false,
            case_sensitive: None,
            extensions: None,
        }
    }

    #[test]
    fn empty_lorebook_produces_empty_graph() {
        let lb = Lorebook { entries: vec![] };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        assert_eq!(graph.node_count, 0);
        assert_eq!(graph.edge_count, 0);
        assert!(graph.conflicts.is_empty());
    }

    #[test]
    fn key_overlap_detected() {
        let lb = Lorebook {
            entries: vec![
                make_entry(vec!["龙", "城堡"], "entry A", Some("A")),
                make_entry(vec!["龙", "魔法"], "entry B", Some("B")),
            ],
        };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        assert_eq!(graph.node_count, 2);
        assert_eq!(graph.edge_count, 1);
        assert_eq!(graph.edges[0].kind, EdgeKind::KeyOverlap);
        assert_eq!(graph.edges[0].weight, 1);
        assert_eq!(graph.edges[0].shared_keys, vec!["龙".to_string()]);
    }

    #[test]
    fn reference_edge_detected() {
        let lb = Lorebook {
            entries: vec![
                make_entry(vec!["英雄"], "英雄前往城堡寻找宝藏", Some("A")),
                make_entry(vec!["城堡"], "城堡位于山谷中", Some("B")),
            ],
        };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        // A.content 引用了 B 的 key "城堡"
        let ref_edge = graph
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Reference)
            .expect("应有 reference 边");
        assert_eq!(ref_edge.source, 0);
        assert_eq!(ref_edge.target, 1);
        assert_eq!(ref_edge.shared_keys, vec!["城堡".to_string()]);
    }

    #[test]
    fn conflict_detected_when_three_entries_reference_same_key() {
        let lb = Lorebook {
            entries: vec![
                make_entry(vec!["龙"], "龙守护着宝藏", Some("A")),
                make_entry(vec!["剑"], "龙被剑击败", Some("B")),
                make_entry(vec!["魔法"], "龙拥有魔法力量", Some("C")),
            ],
        };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        assert_eq!(graph.conflicts.len(), 1);
        assert_eq!(graph.conflicts[0].key, "龙");
        assert_eq!(graph.conflicts[0].entry_indices.len(), 3);
    }

    #[test]
    fn min_weight_filters_low_weight_edges() {
        let lb = Lorebook {
            entries: vec![
                make_entry(vec!["龙", "城堡", "魔法"], "entry A", Some("A")),
                make_entry(vec!["龙"], "entry B", Some("B")),
            ],
        };
        let query = GraphQuery {
            min_weight: 2,
            ..Default::default()
        };
        let graph = build_graph("hero", &lb, &query).unwrap();
        // 只有 1 个共享 key，weight=1，被过滤
        assert_eq!(graph.edge_count, 0);
    }

    #[test]
    fn disabled_entries_excluded_from_edges() {
        let lb = Lorebook {
            entries: vec![make_entry(vec!["龙", "城堡"], "entry A", Some("A")), {
                let mut e = make_entry(vec!["龙", "魔法"], "entry B", Some("B"));
                e.enabled = Some(false);
                e
            }],
        };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        // B 被 disabled，不参与 key_overlap
        assert_eq!(graph.edge_count, 0);
    }

    #[test]
    fn constant_entries_marked_in_nodes() {
        let lb = Lorebook {
            entries: vec![
                {
                    let mut e = make_entry(vec!["世界观"], "全局设定", Some("world"));
                    e.constant = Some(true);
                    e
                },
                make_entry(vec!["英雄"], "英雄登场", Some("hero")),
            ],
        };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        assert!(graph.nodes[0].constant);
        assert!(!graph.nodes[1].constant);
    }

    #[test]
    fn label_falls_back_to_index_when_no_comment_no_key() {
        let lb = Lorebook {
            entries: vec![make_entry(vec![], "anonymous content", None)],
        };
        let graph = build_graph("hero", &lb, &GraphQuery::default()).unwrap();
        assert_eq!(graph.nodes[0].label, "#0");
    }

    #[test]
    fn too_many_entries_returns_error() {
        let entries: Vec<LorebookEntry> = (0..501)
            .map(|i| make_entry_string_keys(vec![format!("k{}", i)], "content", None))
            .collect();
        let lb = Lorebook { entries };
        let result = build_graph("hero", &lb, &GraphQuery::default());
        assert!(matches!(result, Err(AirpError::BadRequest(_))));
    }

    /// #324 N8: 默认上限 500，但调用方可通过 `max_entries` 调高。
    /// 501 条 lorebook 在默认配置下会被拒绝，但把 max_entries 调到 600 应当通过。
    #[test]
    fn max_entries_configurable_allows_larger_lorebook() {
        let entries: Vec<LorebookEntry> = (0..501)
            .map(|i| make_entry_string_keys(vec![format!("k{}", i)], "content", None))
            .collect();
        let lb = Lorebook { entries };

        // 默认 max_entries=500 → 拒绝
        let result_default = build_graph("hero", &lb, &GraphQuery::default());
        assert!(
            matches!(result_default, Err(AirpError::BadRequest(_))),
            "default max_entries=500 should reject 501 entries"
        );

        // 调高 max_entries=600 → 通过
        let query = GraphQuery {
            max_entries: 600,
            ..Default::default()
        };
        let graph = build_graph("hero", &lb, &query).unwrap();
        assert_eq!(graph.node_count, 501);
        // 501 个独立 key，无重叠、无引用，应无边
        assert_eq!(graph.edge_count, 0);
    }

    /// #324 N8: max_entries 也可调低，超过自定义下限的请求应被拒绝。
    #[test]
    fn max_entries_configurable_rejects_when_exceeded() {
        let entries: Vec<LorebookEntry> = (0..11)
            .map(|i| make_entry_string_keys(vec![format!("k{}", i)], "content", None))
            .collect();
        let lb = Lorebook { entries };

        // 调低 max_entries=10 → 11 条应被拒绝
        let query = GraphQuery {
            max_entries: 10,
            ..Default::default()
        };
        let result = build_graph("hero", &lb, &query);
        assert!(
            matches!(result, Err(AirpError::BadRequest(_))),
            "max_entries=10 should reject 11 entries"
        );
    }

    /// #324 N8: GraphQuery::default() 的 max_entries 应为 500（向后兼容）。
    #[test]
    fn default_max_entries_is_500() {
        let q = GraphQuery::default();
        assert_eq!(q.max_entries, 500);
    }

    #[test]
    fn query_defaults_all_true() {
        let q = GraphQuery::default();
        assert!(q.include_references);
        assert!(q.include_key_overlap);
        assert!(q.detect_conflicts);
        assert_eq!(q.min_weight, 1);
    }

    #[test]
    fn exclude_references_flag_respected() {
        let lb = Lorebook {
            entries: vec![
                make_entry(vec!["英雄"], "英雄前往城堡", Some("A")),
                make_entry(vec!["城堡"], "城堡", Some("B")),
            ],
        };
        let query = GraphQuery {
            include_references: false,
            ..Default::default()
        };
        let graph = build_graph("hero", &lb, &query).unwrap();
        // 不应有 reference 边，但也不应有 key_overlap（key 不同）
        assert_eq!(graph.edge_count, 0);
    }
}
