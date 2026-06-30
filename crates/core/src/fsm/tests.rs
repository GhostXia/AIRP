// Tests for fsm — declared as `#[cfg(test)] mod tests;` in fsm.rs.
// `use super::*;` imports all accessible items from the fsm module.
use super::*;

#[test]
fn test_regex_filter_parser() {
    let f1 = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
    assert_eq!(f1.start, "<thought>");
    assert_eq!(f1.end, "</thought>");

    let f2 = RegexFilter::from_regex("\\[系统提示:[\\s\\S]*?\\]");
    assert_eq!(f2.start, "[系统提示:");
    assert_eq!(f2.end, "]");
}

#[test]
fn test_variable_replacement() {
    let mut vars = HashMap::new();
    vars.insert("user".to_string(), "小明".to_string());
    vars.insert("weapon".to_string(), "精钢长剑".to_string());

    let mut fsm = StreamingFsm::new(vec![], vars);

    let out1 = fsm.process_chunk("我手持 {{weapon}} 指向他。");
    let out2 = fsm.finish();
    assert_eq!(out1 + &out2, "我手持 精钢长剑 指向他。");
}

#[test]
fn test_filtering() {
    let f1 = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
    let mut fsm = StreamingFsm::new(vec![f1], HashMap::new());

    let out1 = fsm.process_chunk("你好，人类。<thought>他在看我</thought>我是AI。");
    let out2 = fsm.finish();
    assert_eq!(out1 + &out2, "你好，人类。我是AI。");
}

#[test]
fn test_chunk_boundaries() {
    let f1 = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
    let mut vars = HashMap::new();
    vars.insert("char".to_string(), "艾米丽".to_string());
    let mut fsm = StreamingFsm::new(vec![f1], vars);

    // 模拟碎 Token 吐出
    let chunks = vec![
        "我的名字叫 ",
        "{",
        "{ch",
        "ar}}",
        "。 <tho",
        "ught> 思考",
        "一下 </th",
        "ought> 很高兴见到你",
    ];

    let mut result = String::new();
    for chunk in chunks {
        result.push_str(&fsm.process_chunk(chunk));
    }
    result.push_str(&fsm.finish());

    assert_eq!(result, "我的名字叫 艾米丽。  很高兴见到你");
}

#[test]
fn test_rollback_on_false_positive() {
    let f1 = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
    let mut fsm = StreamingFsm::new(vec![f1], HashMap::new());

    // 输入 <though (不是完整标签，但在缓冲中)
    let out1 = fsm.process_chunk("这是一个 <though 这是一个测试");
    let out2 = fsm.finish();
    assert_eq!(out1 + &out2, "这是一个 <though 这是一个测试");
}

#[test]
fn test_self_closing_volume_seal_tag() {
    // <卷评估 封存="true" 原因="..."/>  自闭合标签，应被完全剥离
    let f = RegexFilter::from_regex("<卷评估[\\s\\S]*?\\/>");
    assert_eq!(f.start, "<卷评估");
    assert_eq!(f.end, "/>");

    let mut fsm = StreamingFsm::new(vec![f], HashMap::new());
    let out1 = fsm.process_chunk("剧情正文。<卷评估 封存=\"true\" 原因=\"晋级决赛\"/>结尾。");
    let out2 = fsm.finish();
    assert_eq!(out1 + &out2, "剧情正文。结尾。");
}

#[test]
fn test_state_tag_filter() {
    // M_LS-2: <state>…</state> stripped during streaming
    let f = RegexFilter::from_regex("<state>[\\s\\S]*?</state>");
    assert_eq!(f.start, "<state>");
    assert_eq!(f.end, "</state>");

    let mut fsm = StreamingFsm::new(vec![f], HashMap::new());
    let out1 = fsm.process_chunk("Turn end.<state>{\"hp\":80}</state>Next.");
    let out2 = fsm.finish();
    assert_eq!(out1 + &out2, "Turn end.Next.");
}

#[test]
fn test_state_tag_across_chunks() {
    // <state> tag split across streaming chunks
    let f = RegexFilter::from_regex("<state>[\\s\\S]*?</state>");
    let mut fsm = StreamingFsm::new(vec![f], HashMap::new());
    let chunks = ["text.<sta", "te>{\"x", "\":1}</sta", "te>end"];
    let mut out = String::new();
    for c in chunks {
        out.push_str(&fsm.process_chunk(c));
    }
    out.push_str(&fsm.finish());
    assert_eq!(out, "text.end");
}

#[test]
fn test_seal_tag_across_chunks() {
    // 标签跨多个流式块到达
    let f = RegexFilter::from_regex("<卷评估[\\s\\S]*?\\/>");
    let mut fsm = StreamingFsm::new(vec![f], HashMap::new());

    let chunks = vec![
        "今天结束了。<卷评",
        "估 封存=\"true\"",
        " 原因=\"重大转",
        "折\"/>",
        "明天继续。",
    ];

    let mut out = String::new();
    for chunk in chunks {
        out.push_str(&fsm.process_chunk(chunk));
    }
    out.push_str(&fsm.finish());
    assert_eq!(out, "今天结束了。明天继续。");
}

#[test]
fn test_multibyte_in_buffering_no_panic() {
    // 回归用例：缓冲态 "<th" 后接中文，旧实现会在 s.len() byte 切片处 panic。
    let f1 = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
    let mut fsm = StreamingFsm::new(vec![f1], HashMap::new());
    // 喂一个 buffering 触发 + 中文字符 fallback 路径
    let chunks = vec!["<th", "字"];
    let mut out = String::new();
    for c in chunks {
        out.push_str(&fsm.process_chunk(c));
    }
    out.push_str(&fsm.finish());
    assert_eq!(out, "<th字");
}

#[test]
fn test_nested_or_adjacent_patterns() {
    let f1 = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
    let mut vars = HashMap::new();
    vars.insert("user".to_string(), "小明".to_string());
    let mut fsm = StreamingFsm::new(vec![f1], vars);

    // <tho 是前缀，但后面接了 {{user}}
    let chunks = vec!["我看着 <tho", "{{user}}"];
    let mut result = String::new();
    for chunk in chunks {
        result.push_str(&fsm.process_chunk(chunk));
    }
    result.push_str(&fsm.finish());
    assert_eq!(result, "我看着 <tho小明");
}

// ── M6.3：proptest 性质测试 ───────────────────────────────────────────
//
// 验证两个核心不变量：
//   1. **chunk 切分独立性**：无论输入按什么边界切碎，逐 chunk 处理后
//      `feed_chunks(parts) + finish()` 必须 等于 `process_chunk(whole) + finish()`。
//      这是流式 FSM 的基本契约——网络 SSE 不能改变文本含义。
//   2. **任意 UTF-8 输入不 panic**：filter + variable 表 + 任意 UTF-8 文本
//      喂进 FSM 都不应崩溃（旧版本曾在 `s.len()` byte 切片处 panic，
//      见回归测试 test_multibyte_in_buffering_no_panic）。

mod proptest_fsm {
    use super::*;
    use proptest::prelude::*;

    fn run_fsm(
        filters: Vec<RegexFilter>,
        vars: HashMap<String, String>,
        chunks: &[&str],
    ) -> String {
        let mut fsm = StreamingFsm::new(filters, vars);
        let mut out = String::new();
        for c in chunks {
            out.push_str(&fsm.process_chunk(c));
        }
        out.push_str(&fsm.finish());
        out
    }

    /// 将完整字符串按给定 split 点切成片，所有 split 点裁剪到 char 边界。
    fn split_at_char_boundaries(s: &str, raw_splits: &[usize]) -> Vec<String> {
        if s.is_empty() {
            return vec![String::new()];
        }
        // 收集所有合法 char 边界（含 0 和 len）
        let mut bounds: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
        bounds.push(s.len());
        let mut snap_points: Vec<usize> = raw_splits
            .iter()
            .map(|p| {
                let target = p % s.len().max(1);
                // 找最接近的 char 边界
                bounds
                    .iter()
                    .min_by_key(|&&b| (b as isize - target as isize).abs())
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        snap_points.push(0);
        snap_points.push(s.len());
        snap_points.sort_unstable();
        snap_points.dedup();
        let mut pieces = Vec::new();
        for w in snap_points.windows(2) {
            pieces.push(s[w[0]..w[1]].to_string());
        }
        pieces
    }

    proptest! {
        // 1. chunk 切分独立性：随机 UTF-8 + 已知 filter + 随机切点
        #[test]
        fn prop_chunk_boundaries_independent(
            whole in "[\\PC]{0,200}",          // 任意可打印 Unicode（含 CJK / emoji）
            raw_splits in proptest::collection::vec(0usize..200, 0..10),
        ) {
            let f = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
            let single = run_fsm(vec![f.clone()], HashMap::new(), &[&whole]);
            let pieces = split_at_char_boundaries(&whole, &raw_splits);
            let piece_refs: Vec<&str> = pieces.iter().map(|s| s.as_str()).collect();
            let chunked = run_fsm(vec![f], HashMap::new(), &piece_refs);
            prop_assert_eq!(single, chunked);
        }

        // 2. 任意 UTF-8 不 panic（含空 filter 与空 var）
        #[test]
        fn prop_no_panic_any_input(input in "[\\PC]{0,300}") {
            let f = RegexFilter::from_regex("<thought>[\\s\\S]*?<\\/thought>");
            let mut vars = HashMap::new();
            vars.insert("user".to_string(), "α".to_string());
            let _ = run_fsm(vec![f], vars, &[&input]);
        }

        // 3. 变量替换 chunk 边界独立性
        #[test]
        fn prop_variable_replacement_chunk_independent(
            prefix in "[a-z]{0,20}",
            suffix in "[a-z]{0,20}",
            raw_splits in proptest::collection::vec(0usize..50, 0..6),
        ) {
            let whole = format!("{}{{{{user}}}}{}", prefix, suffix);
            let mut vars = HashMap::new();
            vars.insert("user".to_string(), "ALICE".to_string());

            let single = run_fsm(vec![], vars.clone(), &[&whole]);
            let pieces = split_at_char_boundaries(&whole, &raw_splits);
            let piece_refs: Vec<&str> = pieces.iter().map(|s| s.as_str()).collect();
            let chunked = run_fsm(vec![], vars, &piece_refs);

            prop_assert_eq!(&single, &chunked);
            prop_assert_eq!(single, format!("{}ALICE{}", prefix, suffix));
        }

        // 4. 自闭合 <卷评估/> filter chunk 独立性（已有手写测试 test_seal_tag_across_chunks 的泛化）
        #[test]
        fn prop_seal_tag_chunk_independent(
            head in "[\\PC]{0,30}",
            attrs in "[a-zA-Z0-9=\" 中文/]{0,40}",
            tail in "[\\PC]{0,30}",
            raw_splits in proptest::collection::vec(0usize..120, 0..8),
        ) {
            let whole = format!("{}<卷评估 {}/>{}", head, attrs, tail);
            let f = RegexFilter::from_regex("<卷评估[\\s\\S]*?\\/>");

            let single = run_fsm(vec![f.clone()], HashMap::new(), &[&whole]);
            let pieces = split_at_char_boundaries(&whole, &raw_splits);
            let piece_refs: Vec<&str> = pieces.iter().map(|s| s.as_str()).collect();
            let chunked = run_fsm(vec![f], HashMap::new(), &piece_refs);

            prop_assert_eq!(single, chunked);
        }
    }
}
