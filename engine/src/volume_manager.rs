//! 封卷业务流程。负责：
//! 1. 从 LLM 输出里解析 `<卷评估/>` 信号
//! 2. 判断是否进入软压力区间或硬阈值
//! 3. 触发独立的封卷 API 调用，落盘新卷并合并 index
//! 4. 周期性维护：归档已解决线索

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use crate::adapter::{self, ChatMessage, GenerationParams, ProviderConfig};
use crate::error::AirpError;
use crate::index_parser;
use crate::volume_store;

/// 跨卷实体提升阈值：实体在多少个不同卷的 `[卷索引]` 中出现后自动晋升 index.md。
const PROMOTE_THRESHOLD: usize = 3;

/// CodeRabbit #6：plot 评估累积输出上限（字符数），防止 LLM 输出爆炸。
const PLOT_EVALUATION_MAX_CHARS: usize = 4000;

// M2.5：所有 Regex 预编译为静态，避免每次调用 chat_completion 重新 compile。
static SEAL_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"<卷评估(?:\s+([^/>]*?))?\s*/>"#).expect("SEAL_TAG_RE compiles"));
static ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(\w+)\s*=\s*["']([^"']*)["']"#).expect("ATTR_RE compiles"));
static VOL_HEADER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<卷索引>(.*?)</卷索引>").expect("VOL_HEADER_RE compiles"));
static VOL_CONTENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<卷内容>(.*?)</卷内容>").expect("VOL_CONTENT_RE compiles"));
static VOL_DIFF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<全局index更新>(.*?)</全局index更新>").expect("VOL_DIFF_RE compiles")
});

/// AI 在主对话末尾发出的封存信号。
#[derive(Debug, Clone)]
pub struct SealSignal {
    /// 是否应该立即触发封卷流程。
    pub should_seal: bool,
    /// 封存原因（保留供 tracing 与未来 UI 调用）。
    #[allow(dead_code)]
    pub reason: String,
}

/// 从 LLM 原始输出（**未经 FSM 过滤**）中提取 `<卷评估 .../>` 标签。
/// 返回 (剥离标签后的干净文本, 信号)。
///
/// 容忍多种属性顺序与引号风格。
pub fn parse_seal_signal(raw: &str) -> (String, Option<SealSignal>) {
    let Some(caps) = SEAL_TAG_RE.captures(raw) else {
        return (raw.to_string(), None);
    };

    // M0 F-26 / 0.10：let-else 显式绑定 group 0，避免裸 unwrap。
    // 正则匹配成功时 group 0 必存在（Regex 保证），此处保险路径。
    let Some(full_match_m) = caps.get(0) else {
        return (raw.to_string(), None);
    };
    let full_match = full_match_m.as_str().to_string();
    let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");

    let mut should_seal = false;
    let mut reason = String::new();
    for ac in ATTR_RE.captures_iter(attrs) {
        let key = ac.get(1).map(|m| m.as_str()).unwrap_or("");
        let val = ac
            .get(2)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        match key {
            "封存" => {
                should_seal = val == "true" || val == "1";
            }
            "原因" => {
                reason = val;
            }
            _ => {}
        }
    }

    let cleaned = raw.replace(full_match.as_str(), "");
    (
        cleaned,
        Some(SealSignal {
            should_seal,
            reason,
        }),
    )
}

/// 进入软压力区间时，返回需要追加到 System Prompt 的提示行。
pub fn soft_pressure_hint(
    session_dir: &Path,
    soft_threshold: usize,
    hard_threshold: usize,
) -> Option<String> {
    let tokens = volume_store::count_tokens_current(session_dir);
    if tokens >= soft_threshold && tokens < hard_threshold {
        Some(format!(
            "\n[系统提示]\n当前剧情段落已较长（约 {} tokens），若本轮存在自然停顿点或重大转折，请在回复末尾输出 <卷评估 封存=\"true\" 原因=\"...\"/>。否则继续推进剧情。\n",
            tokens
        ))
    } else {
        None
    }
}

/// 检查是否到达硬阈值，需要无条件强制封卷。
pub fn should_force_seal(session_dir: &Path, hard_threshold: usize) -> bool {
    volume_store::count_tokens_current(session_dir) >= hard_threshold
}

/// 构建封卷调用的 System Prompt。
fn build_seal_system_prompt() -> String {
    r#"你是剧情归档助手。你将接收"全局索引现状"和"未封存段落"两份输入，
任务是把未封存段落整理成一卷的归档内容，并输出对全局索引的更新指令。

请严格按以下三个 XML 块输出，不要输出任何额外说明：

<卷索引>
- 卷标题: （一句话主题）
- 时间范围: （如 D1-D7 或具体日期段，若无明确时间可写"未指明"）
- 登场: 角色名(关键身份/状态), ...
- 关键事件: 用简短列表罗列核心情节节点
- 新增线索: 本卷新引入的悬念
- 解决线索: 本卷揭晓或了结的悬念
- 状态变化: 玩家与重要角色的关系/身份/物品变化
</卷索引>

<卷内容>
（完整自然语言叙事，保留对话与细节，但去除冗余口水内容；保持时间顺序）
</卷内容>

<全局index更新>
[人物] 提升: 名字(卷N·一句话说明)
[人物] 仅本卷: 名字
[物品] 提升: 名字(卷N·说明)
[地点] 提升: 名字(卷N·说明)
[线索] 名字 → 卷N(已解)
[线索] 名字 → [新增·未解]
</全局index更新>

判断"提升"还是"仅本卷"的标准：
- 提升：与玩家建立明确关系、携带未解线索、有跨卷复现可能
- 仅本卷：一次性背景角色、无后续意义的过场人物

每行一条指令；卷号 N 由系统自动分配，可写"本卷"或留空，系统会补齐。
"#
    .to_string()
}

/// M0 F-27 / 6.0g：封卷 LLM 输出解析结果。
#[derive(Debug, Clone, PartialEq)]
struct SealingOutput {
    header: String,
    content: String,
    diff: String,
}

/// 把封卷 LLM 输出解析为结构化字段。
fn parse_sealing_output(raw: &str) -> Result<SealingOutput, AirpError> {
    let header = VOL_HEADER_RE
        .captures(raw)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .ok_or_else(|| AirpError::Volume("封卷输出缺少 <卷索引> 块".to_string()))?;

    let content = VOL_CONTENT_RE
        .captures(raw)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .ok_or_else(|| AirpError::Volume("封卷输出缺少 <卷内容> 块".to_string()))?;

    let diff = VOL_DIFF_RE
        .captures(raw)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default();

    Ok(SealingOutput {
        header,
        content,
        diff,
    })
}

/// 把 AI 输出里所有 "本卷" 字样替换为真实卷号 N（在 diff 块中）。
fn substitute_volume_placeholder(diff_block: &str, volume_number: u32) -> String {
    let n_str = format!("卷{}", volume_number);
    // "本卷" → "卷N"
    let mut out = diff_block.replace("本卷", &n_str);
    // 兼容 LLM 写 "卷N"（字面）或留空括号 "(...)" 的情况：在这里不做更复杂的修复，
    // 交给 index_parser 处理。
    if !out.contains('卷') && volume_number > 0 {
        // 极端情况：LLM 完全没写卷号，无法补全，原样返回
        out = diff_block.to_string();
    }
    out
}

/// 完整封卷流程。读 current+index → 调 LLM → 写新卷 + 更新 index → 清空 current。
///
/// **M0 F-01**：`client` 由调用方注入以复用 daemon 的连接池。
/// **M4.2**：`provider` 用 `Arc` 共享，`params` 已由调用方派生（覆盖 seal_temperature
/// / seal_model），此处直接传给 adapter。
pub async fn run_seal_flow(
    client: &reqwest::Client,
    session_dir: &Path,
    provider: Arc<ProviderConfig>,
    params: GenerationParams,
) -> Result<Option<u32>, AirpError> {
    let current = volume_store::read_current(session_dir)?;
    if current.trim().is_empty() {
        return Ok(None); // 没有内容可封
    }
    let index = volume_store::read_index(session_dir)?;
    let next_n = volume_store::next_volume_number(session_dir);

    let system_prompt = build_seal_system_prompt();
    let user_input = format!(
        "[全局索引现状]\n{}\n\n[未封存段落 / 即将封为卷{}]\n{}",
        index, next_n, current
    );

    let user_message = ChatMessage {
        role: crate::adapter::MessageRole::User,
        content: user_input,
    };

    let stream = adapter::call_streaming_api(
        client.clone(),
        provider.clone(),
        params.clone(),
        system_prompt,
        vec![user_message],
    );

    futures_util::pin_mut!(stream);
    let mut full = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(tok) => full.push_str(&tok),
            Err(e) => return Err(AirpError::Volume(format!("封卷 API 调用失败: {}", e))),
        }
    }

    let SealingOutput {
        header: header_block,
        content: content_block,
        diff: diff_block_raw,
    } = parse_sealing_output(&full)?;
    let diff_block = substitute_volume_placeholder(&diff_block_raw, next_n);

    // 组装完整卷文件：标题 + [卷索引] + --- + 正文
    let title = header_block
        .lines()
        .find(|l| l.contains("卷标题"))
        .map(|l| {
            l.split(':')
                .nth(1)
                .or_else(|| l.split('：').nth(1))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| format!("卷{}", next_n))
        })
        .unwrap_or_else(|| format!("卷{}", next_n));

    let volume_md = format!(
        "# 卷{}：{}\n\n## [卷索引]\n{}\n\n---\n\n{}\n",
        next_n, title, header_block, content_block
    );

    volume_store::write_volume(session_dir, next_n, &volume_md)?;

    // 合并 index
    let diff = index_parser::parse_index_diff(&diff_block);
    let new_index = index_parser::apply_diff(&index, &diff);
    volume_store::write_index(session_dir, &new_index)?;

    // 清空 current.md
    volume_store::clear_current(session_dir)?;

    // 阶段三补全 D3：封卷后评估剧情进度，生成下卷悬念/方向（best-effort）。
    // CodeRabbit #7：先清除旧 plot_direction，防止评估失败时残留上卷方向
    // 被误注入新卷的 prepare。
    if let Err(e) = volume_store::write_plot_direction(session_dir, "") {
        tracing::warn!(err = %e, "清除旧 plot_direction 失败（best-effort）");
    }
    // plot 评估的流式空闲由 adapter 统一限时，不再限制正常长任务的总时长。
    match run_plot_evaluation(client, provider, params, &volume_md, &new_index).await {
        Ok(direction) => {
            if !direction.trim().is_empty() {
                if let Err(e) = volume_store::write_plot_direction(session_dir, &direction) {
                    tracing::warn!(err = %e, "剧情方向写入失败（best-effort）");
                }
            }
        }
        Err(e) => tracing::warn!(err = %e, "剧情进度评估失败（best-effort）"),
    }

    Ok(Some(next_n))
}

/// 阶段三补全 D3：封卷后评估剧情进度，生成下卷悬念/方向。
///
/// 读取刚封存的卷与全局索引，调用 LLM 评估剧情进度并输出下卷方向，
/// 写入 `plot_direction.md`（下轮 prepare 注入 prompt）。与 `advance_plot`
/// 工具联动：封卷时的宏观方向为后续 `advance_plot` 提供指引。
async fn run_plot_evaluation(
    client: &reqwest::Client,
    provider: Arc<ProviderConfig>,
    params: GenerationParams,
    sealed_volume: &str,
    index: &str,
) -> Result<String, AirpError> {
    let system_prompt = "你是 RP 剧情编排助手。基于刚封存的卷和全局索引，评估当前剧情进度，\
并为下一卷生成悬念/方向指引。输出简洁的 markdown 条目（以 \"- \" 开头），\
包括：当前剧情阶段、未解决的伏笔、下一卷建议推进的方向。不要输出其他内容。";

    let user_input = format!("[全局索引]\n{}\n\n[刚封存的卷]\n{}", index, sealed_volume);

    let user_message = ChatMessage {
        role: crate::adapter::MessageRole::User,
        content: user_input,
    };

    let stream = adapter::call_streaming_api(
        client.clone(),
        provider,
        params,
        system_prompt.to_string(),
        vec![user_message],
    );

    futures_util::pin_mut!(stream);
    let mut full = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(tok) => {
                full.push_str(&tok);
                // CodeRabbit #6：累积输出超上限时停止，防止 LLM 输出爆炸。
                if full.chars().count() > PLOT_EVALUATION_MAX_CHARS {
                    tracing::warn!(
                        max_chars = PLOT_EVALUATION_MAX_CHARS,
                        "plot 评估输出超上限，截断"
                    );
                    break;
                }
            }
            Err(e) => return Err(AirpError::Volume(format!("剧情评估 API 调用失败: {}", e))),
        }
    }

    // 只保留以 "- " 开头的条目行。
    let cleaned: Vec<&str> = full
        .lines()
        .filter(|l| l.trim().starts_with("- "))
        .collect();
    Ok(cleaned.join("\n"))
}

/// 从某一卷 `[卷索引]` 头部的 `- 登场: ...` 行解析角色名列表。
///
/// 兼容中英括号；`艾莉娅(关键身份)` / `艾莉娅（关键身份）` 都剥成 `艾莉娅`。
/// 多个名字以中英逗号分隔。
fn parse_appearance_line(header: &str) -> Vec<String> {
    for line in header.lines() {
        let trimmed = line.trim_start();
        let stripped = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        let rest = stripped
            .strip_prefix("登场:")
            .or_else(|| stripped.strip_prefix("登场："));
        let Some(rest) = rest else { continue };
        return rest
            .split(&[',', '，'][..])
            .map(|s| {
                let s = s.trim();
                let cut = s.find('(').or_else(|| s.find('（'));
                match cut {
                    Some(idx) => s[..idx].trim().to_string(),
                    None => s.to_string(),
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
    }
    Vec::new()
}

/// 扫描已封存的卷，构建每个角色实体的出现卷集合。
///
/// 返回 `name -> {vol_num}`；调用方据 `set.len() >= PROMOTE_THRESHOLD` 判断是否晋升。
fn collect_cross_volume_appearances(session_dir: &Path) -> HashMap<String, BTreeSet<u32>> {
    let mut map: HashMap<String, BTreeSet<u32>> = HashMap::new();
    for vol_num in volume_store::list_volume_numbers(session_dir) {
        match volume_store::read_volume_header(session_dir, vol_num) {
            Ok(header) => {
                for name in parse_appearance_line(&header) {
                    map.entry(name).or_default().insert(vol_num);
                }
            }
            Err(e) => {
                tracing::warn!(vol = vol_num, err = %e, "读取卷头部失败，跳过");
            }
        }
    }
    map
}

/// 周期维护任务。两件事：
///   1. **跨卷实体晋升（5.4）**：扫描所有已封存卷的 `[卷索引]`，对在 ≥
///      [`PROMOTE_THRESHOLD`] 个不同卷出现的角色，自动加入 index.md 的 `## 人物`
///      段（若尚未存在）。已存在的实体不重复。
///   2. **已解线索归档**：把 `## 悬挂线索` 中标 `已解` 的行移入 `## [已归档]`。
///
/// 纯本地操作，无 LLM 调用。`tokio::spawn` 在 finalizer 里按 `maintenance_interval`
/// 触发。
pub fn run_maintenance(session_dir: &Path) -> Result<(), AirpError> {
    let index = volume_store::read_index(session_dir)?;
    let mut sections = index_parser::parse_sections(&index);

    // 1) 跨卷实体晋升
    let appearances = collect_cross_volume_appearances(session_dir);
    for (name, vols) in &appearances {
        if vols.len() < PROMOTE_THRESHOLD {
            continue;
        }
        // 已存在则跳过：通过行首 "- {name}:" / "- {name}" 前缀匹配
        let prefix_colon = format!("- {}:", name);
        let bare = format!("- {}", name);
        let already = sections.characters.iter().any(|l| {
            let t = l.trim_start();
            t.starts_with(&prefix_colon) || t == bare
        });
        if already {
            continue;
        }
        let vol_list: Vec<String> = vols.iter().map(|v| format!("卷{}", v)).collect();
        sections.characters.push(format!(
            "- {}: {} (跨卷 {} 次·自动晋升)",
            name,
            vol_list.join(", "),
            vols.len()
        ));
    }

    // 2) 已解线索归档
    let mut still_open = Vec::new();
    for line in sections.threads.drain(..) {
        if line.contains("已解") {
            sections.archived.push(line);
        } else {
            still_open.push(line);
        }
    }
    sections.threads = still_open;

    let new_index = index_parser::serialize_sections(&sections);
    volume_store::write_index(session_dir, &new_index)?;
    Ok(())
}

#[cfg(test)]
mod tests;
