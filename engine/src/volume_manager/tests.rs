// Tests for volume_manager — declared as `#[cfg(test)] mod tests;` in volume_manager.rs.
// `use super::*;` imports all accessible items from the volume_manager module.
use super::*;
use tempfile::tempdir;

#[test]
fn test_parse_seal_signal_basic() {
    let raw = r#"剧情正文。<卷评估 封存="true" 原因="晋级决赛"/>结尾。"#;
    let (cleaned, signal) = parse_seal_signal(raw);
    assert_eq!(cleaned, "剧情正文。结尾。");
    let sig = signal.expect("signal present");
    assert!(sig.should_seal);
    assert_eq!(sig.reason, "晋级决赛");
}

#[test]
fn test_parse_seal_signal_false() {
    let raw = r#"剧情。<卷评估 封存="false"/>"#;
    let (cleaned, signal) = parse_seal_signal(raw);
    assert_eq!(cleaned, "剧情。");
    let sig = signal.expect("signal present");
    assert!(!sig.should_seal);
}

#[test]
fn test_parse_seal_signal_no_attrs() {
    // 无属性自闭合：信号存在，但 should_seal = false（无显式 封存="true"）
    let raw = r#"剧情<卷评估/>结尾"#;
    let (cleaned, signal) = parse_seal_signal(raw);
    assert_eq!(cleaned, "剧情结尾");
    let sig = signal.expect("signal present even without attrs");
    assert!(!sig.should_seal);
}

#[test]
fn test_parse_seal_signal_missing() {
    let raw = "完全没有任何标签的剧情正文";
    let (cleaned, signal) = parse_seal_signal(raw);
    assert_eq!(cleaned, raw);
    assert!(signal.is_none());
}

#[test]
fn test_parse_seal_signal_attr_order_and_quote_styles() {
    // 单引号
    let raw1 = r#"x<卷评估 原因='重大转折' 封存='true'/>y"#;
    let (cleaned1, s1) = parse_seal_signal(raw1);
    assert_eq!(cleaned1, "xy");
    assert!(s1.unwrap().should_seal);

    // 反序
    let raw2 = r#"a<卷评估 原因="揭露" 封存="true"/>b"#;
    let (cleaned2, s2) = parse_seal_signal(raw2);
    assert_eq!(cleaned2, "ab");
    let sig2 = s2.unwrap();
    assert!(sig2.should_seal);
    assert_eq!(sig2.reason, "揭露");
}

#[test]
fn test_parse_sealing_output_complete() {
    let raw = r#"前置
<卷索引>
- 卷标题: 初遇
- 登场: 玩家, 艾莉娅
</卷索引>

<卷内容>
艾莉娅在森林入口出现。
</卷内容>

<全局index更新>
[人物] 提升: 艾莉娅(本卷·初登场)
</全局index更新>
尾后"#;
    let parsed = parse_sealing_output(raw).unwrap();
    assert!(parsed.header.contains("初遇"));
    assert!(parsed.content.contains("艾莉娅在森林"));
    assert!(parsed.diff.contains("艾莉娅(本卷"));
}

#[test]
fn test_parse_sealing_output_missing_blocks() {
    let raw = "<卷索引>头</卷索引>";
    let result = parse_sealing_output(raw);
    assert!(result.is_err());
}

#[test]
fn test_substitute_volume_placeholder() {
    let diff = "[人物] 提升: 艾莉娅(本卷·登场)\n[线索] X → 本卷(已解)";
    let out = substitute_volume_placeholder(diff, 5);
    assert!(out.contains("卷5·登场"));
    assert!(out.contains("卷5(已解)"));
}

#[test]
fn test_soft_pressure_hint() {
    let tmp = tempdir().unwrap();
    let session_dir = tmp.path().join("s1");
    volume_store::ensure_session_dirs(&session_dir).unwrap();

    // 空 current.md：不应有提示
    assert!(soft_pressure_hint(&session_dir, 100, 200).is_none());

    // 写入足够长内容到触发软压力
    let long_text = "你好世界 ".repeat(200);
    volume_store::append_to_current(&session_dir, &long_text).unwrap();

    let hint = soft_pressure_hint(&session_dir, 100, 5000);
    assert!(hint.is_some());
    assert!(hint.unwrap().contains("[系统提示]"));
}

#[test]
fn test_should_force_seal() {
    let tmp = tempdir().unwrap();
    let session_dir = tmp.path().join("s1");
    volume_store::ensure_session_dirs(&session_dir).unwrap();
    assert!(!should_force_seal(&session_dir, 100));

    let long = "测试文本 ".repeat(500);
    volume_store::append_to_current(&session_dir, &long).unwrap();
    assert!(should_force_seal(&session_dir, 100));
}

#[test]
fn test_parse_appearance_line_basic() {
    let header = r#"# 卷5：试炼

## [卷索引]
- 卷标题: 试炼
- 时间范围: D3
- 登场: 玩家, 艾莉娅(主角), 卡尔（剑士）, 路人甲
- 关键事件: 闯入古神殿
"#;
    let names = parse_appearance_line(header);
    assert_eq!(names, vec!["玩家", "艾莉娅", "卡尔", "路人甲"]);
}

#[test]
fn test_parse_appearance_line_missing() {
    let header = "# 卷1\n## [卷索引]\n- 卷标题: 起点\n";
    assert!(parse_appearance_line(header).is_empty());
}

#[test]
fn test_collect_cross_volume_appearances() {
    let tmp = tempdir().unwrap();
    let session_dir = tmp.path().join("s1");
    volume_store::ensure_session_dirs(&session_dir).unwrap();

    // 卷 1-3 都有艾莉娅，卷 1 还有卡尔
    let v1 = "# 卷1\n## [卷索引]\n- 登场: 艾莉娅, 卡尔\n\n---\n\n正文1";
    let v2 = "# 卷2\n## [卷索引]\n- 登场: 艾莉娅\n\n---\n\n正文2";
    let v3 = "# 卷3\n## [卷索引]\n- 登场: 艾莉娅, 露娜\n\n---\n\n正文3";
    volume_store::write_volume(&session_dir, 1, v1).unwrap();
    volume_store::write_volume(&session_dir, 2, v2).unwrap();
    volume_store::write_volume(&session_dir, 3, v3).unwrap();

    let map = collect_cross_volume_appearances(&session_dir);
    assert_eq!(map.get("艾莉娅").unwrap().len(), 3);
    assert_eq!(map.get("卡尔").unwrap().len(), 1);
    assert_eq!(map.get("露娜").unwrap().len(), 1);
}

#[test]
fn test_run_maintenance_promotes_cross_volume_entity() {
    let tmp = tempdir().unwrap();
    let session_dir = tmp.path().join("s1");
    volume_store::ensure_session_dirs(&session_dir).unwrap();

    // 三卷都出现艾莉娅 → 应被自动晋升
    for n in 1..=3 {
        let body = format!(
            "# 卷{}\n## [卷索引]\n- 登场: 艾莉娅, 仅本卷{}\n\n---\n\n正文",
            n, n
        );
        volume_store::write_volume(&session_dir, n, &body).unwrap();
    }

    run_maintenance(&session_dir).unwrap();
    let idx = volume_store::read_index(&session_dir).unwrap();

    // 艾莉娅出现 3 卷 ≥ threshold → 已晋升到人物段
    let characters_section = idx
        .split("## 人物")
        .nth(1)
        .unwrap()
        .split("##")
        .next()
        .unwrap();
    assert!(characters_section.contains("艾莉娅"));
    assert!(characters_section.contains("跨卷"));

    // 单卷实体不应晋升
    assert!(!idx.contains("仅本卷1"));
}

#[test]
fn test_run_maintenance_does_not_duplicate_existing() {
    let tmp = tempdir().unwrap();
    let session_dir = tmp.path().join("s1");
    volume_store::ensure_session_dirs(&session_dir).unwrap();

    // 预先在 index 写入艾莉娅
    let initial = "# 全局索引\n\n## 人物\n- 艾莉娅: 卷1(初登场)\n\n## 物品\n\n## 悬挂线索\n\n## 地点\n\n## [已归档]\n";
    volume_store::write_index(&session_dir, initial).unwrap();

    for n in 1..=3 {
        let body = format!("# 卷{}\n## [卷索引]\n- 登场: 艾莉娅\n\n---\n\n正文", n);
        volume_store::write_volume(&session_dir, n, &body).unwrap();
    }

    run_maintenance(&session_dir).unwrap();
    let idx = volume_store::read_index(&session_dir).unwrap();

    // 应保留原有条目，不重复添加
    let count = idx.matches("艾莉娅").count();
    assert_eq!(count, 1, "应只出现一次，实际 = {}\n{}", count, idx);
}

#[test]
fn test_run_maintenance_archives_resolved() {
    let tmp = tempdir().unwrap();
    let session_dir = tmp.path().join("s1");
    volume_store::ensure_session_dirs(&session_dir).unwrap();

    let initial = r#"# 全局索引

## 人物

## 物品

## 悬挂线索
- 艾莉娅身份: 卷2(提出) → 卷6(已解)
- 失散妹妹: 卷2(提出) → [未解]

## 地点

## [已归档]
"#;
    volume_store::write_index(&session_dir, initial).unwrap();
    run_maintenance(&session_dir).unwrap();

    let updated = volume_store::read_index(&session_dir).unwrap();
    // 已解决的线索应当移入归档
    let archived_section = updated.split("## [已归档]").nth(1).unwrap();
    assert!(archived_section.contains("艾莉娅身份"));
    // 未解决的留在原段
    let threads_section = updated.split("## 悬挂线索").nth(1).unwrap();
    assert!(threads_section.contains("失散妹妹"));
    // 已解条目不应同时在悬挂段
    let only_threads = threads_section.split("##").next().unwrap();
    assert!(!only_threads.contains("艾莉娅身份"));
}

// ── M5.7：多 session 隔离 + 维护互不影响 ───────────────────────────────
//
// 设计契约：同一角色的两个 session 各自维护独立的 current.md / vol_NNN.md /
// index.md / turn_counter.txt。封卷与 maintenance 只能影响触发它的 session，
// 必须严格不写入兄弟 session 目录。

#[test]
fn test_multi_session_isolation_volumes_and_index() {
    let tmp = tempdir().unwrap();
    let s_a = tmp.path().join("session_a");
    let s_b = tmp.path().join("session_b");
    volume_store::ensure_session_dirs(&s_a).unwrap();
    volume_store::ensure_session_dirs(&s_b).unwrap();

    // session A 写三卷登场角色「甲」
    for n in 1..=3 {
        let body = format!("# 卷{}\n## [卷索引]\n- 登场: 甲\n\n---\n\n正文 A{}", n, n);
        volume_store::write_volume(&s_a, n, &body).unwrap();
    }
    // session B 写两卷登场角色「乙」（不到晋升阈值）
    for n in 1..=2 {
        let body = format!("# 卷{}\n## [卷索引]\n- 登场: 乙\n\n---\n\n正文 B{}", n, n);
        volume_store::write_volume(&s_b, n, &body).unwrap();
    }

    // 仅在 session A 跑 maintenance
    run_maintenance(&s_a).unwrap();
    let idx_a = volume_store::read_index(&s_a).unwrap();
    let idx_b = volume_store::read_index(&s_b).unwrap();

    // session A 索引：甲 已晋升
    assert!(idx_a.contains("甲"), "session A index 应含甲: {}", idx_a);
    // session B 索引：未跑 maintenance，且即便跑也不应跨边界
    assert!(
        !idx_b.contains("甲"),
        "session B index 不应出现甲: {}",
        idx_b
    );
    assert!(
        !idx_b.contains("乙"),
        "session B 未跑 maintenance，索引应为初始空白"
    );

    // session B 跑 maintenance 后 仍不影响 session A
    run_maintenance(&s_b).unwrap();
    let idx_a_after = volume_store::read_index(&s_a).unwrap();
    assert_eq!(idx_a, idx_a_after, "session B 维护不应改写 session A 索引");
    // 乙 只两卷 < PROMOTE_THRESHOLD(3) → session B 不晋升
    let idx_b_after = volume_store::read_index(&s_b).unwrap();
    assert!(
        !idx_b_after
            .split("## 人物")
            .nth(1)
            .unwrap_or("")
            .contains("乙"),
        "乙 跨卷 2 次未达阈值 3，不应晋升"
    );
}

#[test]
fn test_multi_session_turn_counter_isolation() {
    let tmp = tempdir().unwrap();
    let s_a = tmp.path().join("session_a");
    let s_b = tmp.path().join("session_b");
    volume_store::ensure_session_dirs(&s_a).unwrap();
    volume_store::ensure_session_dirs(&s_b).unwrap();

    // A 推进 5 轮
    for _ in 0..5 {
        volume_store::increment_turn_counter(&s_a).unwrap();
    }
    // B 推进 2 轮
    for _ in 0..2 {
        volume_store::increment_turn_counter(&s_b).unwrap();
    }

    // 各自计数互不影响
    let counter_a = std::fs::read_to_string(s_a.join("turn_counter.txt"))
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    let counter_b = std::fs::read_to_string(s_b.join("turn_counter.txt"))
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert_eq!(counter_a, 5);
    assert_eq!(counter_b, 2);
}

#[test]
fn test_multi_session_current_md_isolation() {
    // append_to_current 必须写到对应 session 的 current.md，不串扰
    let tmp = tempdir().unwrap();
    let s_a = tmp.path().join("session_a");
    let s_b = tmp.path().join("session_b");
    volume_store::ensure_session_dirs(&s_a).unwrap();
    volume_store::ensure_session_dirs(&s_b).unwrap();

    volume_store::append_to_current(&s_a, "Alice 在酒馆点了一杯酒\n").unwrap();
    volume_store::append_to_current(&s_b, "Bob 走进了图书馆\n").unwrap();

    let cur_a = volume_store::read_current(&s_a).unwrap();
    let cur_b = volume_store::read_current(&s_b).unwrap();
    assert!(cur_a.contains("Alice"));
    assert!(!cur_a.contains("Bob"));
    assert!(cur_b.contains("Bob"));
    assert!(!cur_b.contains("Alice"));
}
