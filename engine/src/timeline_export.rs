//! Phase 4.5: 剧情时间线导出 (timeline export)。
//!
//! 从 chat history + world_events + world_clock 生成结构化时间线，并导出为
//! Markdown / HTML / JSON 三种格式。HTML 可通过浏览器"打印为 PDF"获得 PDF 输出，
//! 避免引入额外 PDF 库依赖。
//!
//! ## 数据来源
//! - **chat history**：`characters/{id}/sessions/{sid}/history/chat_log.jsonl`
//!   每条 StoredMessage 含 `role/content/id/ts`（旧消息可能无 ts）。
//! - **world_events**：`characters/{id}/world_events.json`（角色级，Vec<WorldEvent>）
//!   含 `triggered` 标记与可选 `time_trigger`（world_clock 时刻）。
//! - **world_clock**：`characters/{id}/world_clock.json`（角色级）
//!   含 `current_time / advance_per_turn / time_unit / display_format`。
//!
//! ## 设计要点
//! - **只读**：不修改任何持久化数据
//! - **无 ts 容错**：旧消息 ts=None 时按 jsonl 物理顺序排到带 ts 消息之后或之前
//!   采用 "无 ts 排最后、组内按物理顺序" 策略，与 #73 long-history 合同一致
//! - **world_events 穿插**：已触发事件按 time_trigger 排到对应 world_clock 时刻附近；
//!   没有 time_trigger 的已触发事件作为"附录"放在末尾
//! - **HTML 内嵌 CSS**：单一 .html 文件即可打印为 PDF，无外部依赖
//! - **AIRP 独立实现**：所有渲染逻辑自行编写，不复用第三方模板引擎

use crate::adapter::MessageRole;
use crate::agent::tools::{load_world_clock, load_world_events, WorldClock, WorldEvent};
use crate::chat_store::ChatLog;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};
use serde::{Deserialize, Serialize};

/// 导出格式查询参数。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// 默认：结构化 JSON（完整时间线数据，便于二次处理）
    #[default]
    Json,
    /// Markdown 纯文本
    Markdown,
    /// 内嵌 CSS 的单文件 HTML（可浏览器打印为 PDF）
    Html,
}

/// 时间线条目：可能是聊天消息或世界事件触发。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineEntry {
    /// 聊天消息（来自 chat_log.jsonl）。
    ChatMessage {
        /// Durable message id（#37），可能为 None（legacy 旧消息）。
        message_id: Option<String>,
        /// ISO 8601 时间戳，可能为 None（#73 旧消息）。
        ts: Option<String>,
        /// user / assistant / system
        role: String,
        /// 消息内容。
        content: String,
    },
    /// 世界事件触发记录（来自 world_events.json 中 triggered=true 的条目）。
    WorldEvent {
        /// 事件 id。
        event_id: String,
        /// 事件名称。
        name: String,
        /// 事件描述。
        description: String,
        /// 事件注入内容。
        content: String,
        /// 时间触发条件（world_clock 时刻），可能为 None（关键词触发型事件）。
        time_trigger: Option<u64>,
        /// 是否已触发。
        triggered: bool,
    },
}

/// 角色名快照（从 character card 读取，仅取 name 字段）。
#[derive(Debug, Clone, Serialize)]
pub struct CharacterSnapshot {
    pub character_id: String,
    pub name: String,
}

/// World clock 快照。
#[derive(Debug, Clone, Serialize)]
pub struct WorldClockSnapshot {
    pub current_time: u64,
    pub advance_per_turn: u64,
    pub time_unit: String,
    pub display_format: Option<String>,
    pub display: String,
}

impl From<&WorldClock> for WorldClockSnapshot {
    fn from(c: &WorldClock) -> Self {
        Self {
            current_time: c.current_time,
            advance_per_turn: c.advance_per_turn,
            time_unit: c.time_unit.clone(),
            display_format: c.display_format.clone(),
            display: c.display(),
        }
    }
}

/// 完整时间线导出数据。
#[derive(Debug, Clone, Serialize)]
pub struct TimelineExport {
    /// 元数据：生成时间（ISO 8601 UTC）。
    pub generated_at: String,
    /// 角色 ID。
    pub character_id: String,
    /// Session ID（命名 session 的 UUID）。
    pub session_id: String,
    /// Session 创建时间（来自 chat_log_meta.json）。
    pub session_created_at: String,
    /// Session 最后更新时间。
    pub session_updated_at: String,
    /// 角色快照（含 name）。
    pub character: Option<CharacterSnapshot>,
    /// World clock 快照（角色级，跨 session 共享）。
    pub world_clock: Option<WorldClockSnapshot>,
    /// 时间线条目（已排序：有 ts 的 chat message 按时间正序穿插 world event；
    /// 无 ts 的 chat message 排在末尾，按物理顺序）。
    pub entries: Vec<TimelineEntry>,
    /// 未触发的世界事件列表（作为附录展示，不进入 entries）。
    pub pending_events: Vec<WorldEvent>,
    /// 统计：消息总数。
    pub message_count: usize,
    /// 统计：已触发事件数。
    pub triggered_event_count: usize,
}

/// 构建时间线导出数据。
///
/// 参数：
/// - `data_root`：数据根目录
/// - `character_id`：角色 ID（已校验）
/// - `session_id`：命名 session ID（None 时使用 legacy per-character log）
/// - `character_name`：可选的角色名（从 character card 提前读取）
pub fn build_timeline(
    data_root: &std::path::Path,
    character_id: &str,
    session_id: Option<&SessionId>,
    character_name: Option<String>,
) -> Result<TimelineExport, AirpError> {
    let cid = CharacterId::new(character_id)?;

    // 1. 加载 chat log（read-only：不存在则返回空，不创建新 session，不修复 meta）。
    //    使用 load_for_session_if_exists 避免 timeline export 产生 side effect
    //    （创建空 session 文件 / meta 修复写入）。
    let log = match ChatLog::load_for_session_if_exists(data_root, cid.as_str(), session_id)? {
        Some(log) => log,
        None => {
            // JSONL 不存在 → 返回空 TimelineExport（仅含 world events / clock 元数据）
            let world_events = load_world_events(data_root, cid.as_str())?;
            let world_clock = load_world_clock(data_root, cid.as_str())?;
            let clock_snapshot = WorldClockSnapshot::from(&world_clock);
            let pending_events: Vec<WorldEvent> = world_events
                .iter()
                .filter(|e| !e.triggered)
                .cloned()
                .collect();
            let session_id_str = match session_id {
                Some(sid) => sid.to_string(),
                None => String::new(),
            };
            return Ok(TimelineExport {
                generated_at: chrono::Utc::now().to_rfc3339(),
                character_id: cid.as_str().to_string(),
                session_id: session_id_str,
                session_created_at: String::new(),
                session_updated_at: String::new(),
                character: character_name.map(|name| CharacterSnapshot {
                    character_id: cid.as_str().to_string(),
                    name,
                }),
                world_clock: Some(clock_snapshot),
                entries: Vec::new(),
                pending_events,
                message_count: 0,
                triggered_event_count: world_events.iter().filter(|e| e.triggered).count(),
            });
        }
    };

    // 2. 加载 world_events 与 world_clock（角色级，不依赖 session）
    let world_events = load_world_events(data_root, cid.as_str())?;
    let world_clock = load_world_clock(data_root, cid.as_str())?;
    let clock_snapshot = WorldClockSnapshot::from(&world_clock);

    // 3. 构建条目并排序
    let entries = build_entries(&log, &world_events)?;

    let message_count = log.messages.len();
    let triggered_event_count = world_events.iter().filter(|e| e.triggered).count();
    let pending_events: Vec<WorldEvent> = world_events
        .iter()
        .filter(|e| !e.triggered)
        .cloned()
        .collect();

    let session_id_str = match session_id {
        Some(sid) => sid.to_string(),
        None => log.session_id.clone(),
    };

    Ok(TimelineExport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        character_id: cid.as_str().to_string(),
        session_id: session_id_str,
        session_created_at: log.created_at.clone(),
        session_updated_at: log.updated_at.clone(),
        character: character_name.map(|name| CharacterSnapshot {
            character_id: cid.as_str().to_string(),
            name,
        }),
        world_clock: Some(clock_snapshot),
        entries,
        pending_events,
        message_count,
        triggered_event_count,
    })
}

/// 构建时间线条目并按规则排序。
///
/// 排序规则：
/// 1. 有 ts 的 chat message → 按 ts 升序
/// 2. 有 time_trigger 且 triggered 的 world event → 按 time_trigger 升序，穿插在
///    ts 最接近的 chat message 附近（这里采用"按 time_trigger 顺序，统一排在所有
///    chat message 之前"的简化策略，因为 world_clock 是抽象时间单位，与 wall-clock
///    ts 没有直接映射关系）
/// 3. 无 ts 的 chat message → 按物理顺序排在末尾
/// 4. triggered 但无 time_trigger 的 world event → 排在所有 chat message 之后、
///    pending 之前
fn build_entries(
    log: &ChatLog,
    world_events: &[WorldEvent],
) -> Result<Vec<TimelineEntry>, AirpError> {
    // 收集 (idx, ts) 二元组：保留物理索引以回查 message_ids[idx]
    let mut timed: Vec<(usize, &Option<String>)> = Vec::new();
    let mut untimed: Vec<usize> = Vec::new();

    for (idx, ts) in log.message_timestamps.iter().enumerate() {
        if ts.is_some() {
            timed.push((idx, ts));
        } else {
            untimed.push(idx);
        }
    }

    // timed 按 ts 升序（idx 仅用于回查 message_ids/messages）
    timed.sort_by(|a, b| {
        let ta = a.1.as_deref().unwrap_or("");
        let tb = b.1.as_deref().unwrap_or("");
        ta.cmp(tb)
    });

    // world events：已触发的进入 entries，按 time_trigger 排序
    // 有 time_trigger 的优先；无 time_trigger 的排在有 time_trigger 的之后
    let mut triggered_with_time: Vec<&WorldEvent> = world_events
        .iter()
        .filter(|e| e.triggered && e.time_trigger.is_some())
        .collect();
    triggered_with_time.sort_by_key(|e| e.time_trigger.unwrap());

    let triggered_without_time: Vec<&WorldEvent> = world_events
        .iter()
        .filter(|e| e.triggered && e.time_trigger.is_none())
        .collect();

    // 组装顺序：timed_with_time_trigger events → timed chat messages → untimed chat messages → triggered_without_time events
    // 注：world_clock 时刻与 wall-clock ts 无直接映射，这里把"时间触发事件"作为
    // "剧情节点"放在对话之前，便于阅读时先看到事件背景再看对话发展。
    let mut entries: Vec<TimelineEntry> = Vec::with_capacity(
        triggered_with_time.len() + timed.len() + untimed.len() + triggered_without_time.len(),
    );

    for e in &triggered_with_time {
        entries.push(TimelineEntry::WorldEvent {
            event_id: e.id.clone(),
            name: e.name.clone(),
            description: e.description.clone(),
            content: e.content.clone(),
            time_trigger: e.time_trigger,
            triggered: e.triggered,
        });
    }

    for (idx, ts) in &timed {
        // 防御性索引：message_timestamps 与 messages 长度不一致时不应 panic，
        // 而是跳过该条目（数据损坏由上层修复逻辑处理，timeline export 不修复）。
        let msg = log.messages.get(*idx).ok_or_else(|| {
            AirpError::Internal(format!(
                "message index {} out of bounds (messages.len()={})",
                idx,
                log.messages.len()
            ))
        })?;
        let mid = log.message_ids.get(*idx).cloned();
        entries.push(TimelineEntry::ChatMessage {
            message_id: mid,
            ts: (*ts).clone(),
            role: role_str(msg.role),
            content: msg.content.clone(),
        });
    }

    for idx in &untimed {
        let msg = log.messages.get(*idx).ok_or_else(|| {
            AirpError::Internal(format!(
                "message index {} out of bounds (messages.len()={})",
                idx,
                log.messages.len()
            ))
        })?;
        let mid = log.message_ids.get(*idx).cloned();
        entries.push(TimelineEntry::ChatMessage {
            message_id: mid,
            ts: None,
            role: role_str(msg.role),
            content: msg.content.clone(),
        });
    }

    for e in &triggered_without_time {
        entries.push(TimelineEntry::WorldEvent {
            event_id: e.id.clone(),
            name: e.name.clone(),
            description: e.description.clone(),
            content: e.content.clone(),
            time_trigger: e.time_trigger,
            triggered: e.triggered,
        });
    }

    Ok(entries)
}

/// MessageRole → 字符串，与 serde 序列化（rename_all = lowercase）一致。
fn role_str(role: MessageRole) -> String {
    match role {
        MessageRole::User => "user".to_string(),
        MessageRole::Assistant => "assistant".to_string(),
        MessageRole::System => "system".to_string(),
    }
}

/// 把 TimelineExport 序列化为 Markdown 字符串。
pub fn to_markdown(timeline: &TimelineExport) -> String {
    let mut out = String::with_capacity(8192);

    // 标题
    let title = timeline
        .character
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or(&timeline.character_id);
    out.push_str(&format!("# {} · 剧情时间线\n\n", escape_md(title)));

    // 元数据
    out.push_str("## 元数据\n\n");
    out.push_str(&format!("- 角色 ID: `{}`\n", timeline.character_id));
    out.push_str(&format!("- Session ID: `{}`\n", timeline.session_id));
    out.push_str(&format!(
        "- Session 创建时间: {}\n",
        timeline.session_created_at
    ));
    out.push_str(&format!(
        "- Session 更新时间: {}\n",
        timeline.session_updated_at
    ));
    out.push_str(&format!("- 导出生成时间: {}\n", timeline.generated_at));
    out.push_str(&format!("- 消息总数: {}\n", timeline.message_count));
    out.push_str(&format!(
        "- 已触发世界事件: {}\n",
        timeline.triggered_event_count
    ));

    if let Some(clock) = &timeline.world_clock {
        out.push_str(&format!(
            "\n**世界时钟**: {} ({})\n",
            clock.display, clock.time_unit
        ));
    }

    // 时间线主体
    out.push_str("\n## 剧情时间线\n\n");
    if timeline.entries.is_empty() {
        out.push_str("_（暂无消息）_\n");
    } else {
        for entry in &timeline.entries {
            match entry {
                TimelineEntry::ChatMessage {
                    ts, role, content, ..
                } => {
                    let ts_str = ts.as_deref().unwrap_or("(无时间戳)");
                    let label = match role.as_str() {
                        "user" => "🧑 用户",
                        "assistant" => "🎭 角色",
                        "system" => "⚙️ 系统",
                        _ => role.as_str(),
                    };
                    out.push_str(&format!("### [{}] {}\n\n", ts_str, label));
                    out.push_str(&escape_md_block(content));
                    out.push_str("\n\n---\n\n");
                }
                TimelineEntry::WorldEvent {
                    name,
                    description,
                    content,
                    time_trigger,
                    ..
                } => {
                    let tt = time_trigger
                        .map(|t| format!(" (world_clock T+{})", t))
                        .unwrap_or_default();
                    out.push_str(&format!("### 🌐 世界事件：{}{}\n\n", escape_md(name), tt));
                    if !description.is_empty() {
                        out.push_str(&format!("> {}\n\n", escape_md(description)));
                    }
                    out.push_str(&escape_md_block(content));
                    out.push_str("\n\n---\n\n");
                }
            }
        }
    }

    // 附录：未触发事件
    if !timeline.pending_events.is_empty() {
        out.push_str("## 附录：未触发世界事件\n\n");
        for e in &timeline.pending_events {
            let tt = e
                .time_trigger
                .map(|t| format!(" (time_trigger T+{})", t))
                .unwrap_or_default();
            // 每条 bullet 末尾必须加 \n，否则多个 pending events 会渲染成同一行
            // （Markdown 列表项需要换行分隔）。
            out.push_str(&format!(
                "- **{}**{}: {}\n",
                escape_md(&e.name),
                tt,
                escape_md(&e.description)
            ));
        }
        out.push('\n');
    }

    out
}

/// 把 TimelineExport 序列化为单文件 HTML（内嵌 CSS）。
pub fn to_html(timeline: &TimelineExport) -> String {
    let mut out = String::with_capacity(16384);

    let title = timeline
        .character
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or(&timeline.character_id);

    out.push_str("<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n");
    out.push_str("<meta charset=\"UTF-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    out.push_str(&format!(
        "<title>{} · 剧情时间线</title>\n",
        escape_html(title)
    ));
    out.push_str(HTML_STYLE);
    out.push_str("</head>\n<body>\n");

    out.push_str("<main class=\"timeline-doc\">\n");

    out.push_str(&format!(
        "<h1 class=\"doc-title\">{} · 剧情时间线</h1>\n",
        escape_html(title)
    ));

    // 元数据卡片
    out.push_str("<section class=\"meta-card\">\n");
    out.push_str("<h2>元数据</h2>\n<dl>\n");
    out.push_str(&format!(
        "<dt>角色 ID</dt><dd><code>{}</code></dd>\n",
        escape_html(&timeline.character_id)
    ));
    out.push_str(&format!(
        "<dt>Session ID</dt><dd><code>{}</code></dd>\n",
        escape_html(&timeline.session_id)
    ));
    out.push_str(&format!(
        "<dt>Session 创建时间</dt><dd>{}</dd>\n",
        escape_html(&timeline.session_created_at)
    ));
    out.push_str(&format!(
        "<dt>Session 更新时间</dt><dd>{}</dd>\n",
        escape_html(&timeline.session_updated_at)
    ));
    out.push_str(&format!(
        "<dt>导出生成时间</dt><dd>{}</dd>\n",
        escape_html(&timeline.generated_at)
    ));
    out.push_str(&format!(
        "<dt>消息总数</dt><dd>{}</dd>\n",
        timeline.message_count
    ));
    out.push_str(&format!(
        "<dt>已触发世界事件</dt><dd>{}</dd>\n",
        timeline.triggered_event_count
    ));
    if let Some(clock) = &timeline.world_clock {
        out.push_str(&format!(
            "<dt>世界时钟</dt><dd>{} ({})</dd>\n",
            escape_html(&clock.display),
            escape_html(&clock.time_unit)
        ));
    }
    out.push_str("</dl>\n</section>\n");

    // 时间线主体
    out.push_str("<section class=\"timeline-body\">\n");
    out.push_str("<h2>剧情时间线</h2>\n");
    if timeline.entries.is_empty() {
        out.push_str("<p class=\"empty\">（暂无消息）</p>\n");
    } else {
        out.push_str("<ol class=\"timeline-list\">\n");
        for entry in &timeline.entries {
            match entry {
                TimelineEntry::ChatMessage {
                    ts, role, content, ..
                } => {
                    let ts_str = ts.as_deref().unwrap_or("(无时间戳)");
                    let (label, cls) = match role.as_str() {
                        "user" => ("用户", "msg-user"),
                        "assistant" => ("角色", "msg-assistant"),
                        "system" => ("系统", "msg-system"),
                        _ => (role.as_str(), "msg-other"),
                    };
                    out.push_str(&format!("<li class=\"timeline-item {}\">\n", cls));
                    out.push_str(&format!(
                        "<div class=\"item-meta\"><span class=\"item-ts\">{}</span><span class=\"item-role\">{}</span></div>\n",
                        escape_html(ts_str), escape_html(label)
                    ));
                    out.push_str("<div class=\"item-content\">\n");
                    out.push_str(&content_to_html(content));
                    out.push_str("\n</div>\n</li>\n");
                }
                TimelineEntry::WorldEvent {
                    name,
                    description,
                    content,
                    time_trigger,
                    ..
                } => {
                    let tt = time_trigger
                        .map(|t| format!(" · T+{}", t))
                        .unwrap_or_default();
                    out.push_str("<li class=\"timeline-item msg-event\">\n");
                    out.push_str(&format!(
                        "<div class=\"item-meta\"><span class=\"item-ts\">世界事件{}</span><span class=\"item-role\">🌐 {}</span></div>\n",
                        escape_html(&tt), escape_html(name)
                    ));
                    if !description.is_empty() {
                        out.push_str(&format!(
                            "<div class=\"item-desc\">{}</div>\n",
                            escape_html(description)
                        ));
                    }
                    out.push_str("<div class=\"item-content\">\n");
                    out.push_str(&content_to_html(content));
                    out.push_str("\n</div>\n</li>\n");
                }
            }
        }
        out.push_str("</ol>\n");
    }
    out.push_str("</section>\n");

    // 附录
    if !timeline.pending_events.is_empty() {
        out.push_str("<section class=\"appendix\">\n");
        out.push_str("<h2>附录：未触发世界事件</h2>\n<ul>\n");
        for e in &timeline.pending_events {
            let tt = e
                .time_trigger
                .map(|t| format!(" · T+{}", t))
                .unwrap_or_default();
            out.push_str(&format!(
                "<li><strong>{}</strong><span class=\"tt\">{}</span>: {}</li>\n",
                escape_html(&e.name),
                escape_html(&tt),
                escape_html(&e.description)
            ));
        }
        out.push_str("</ul>\n</section>\n");
    }

    out.push_str("</main>\n</body>\n</html>\n");
    out
}

/// 内嵌 CSS（单文件 HTML，可浏览器打印为 PDF）。
const HTML_STYLE: &str = r#"<style>
:root {
  --fg: #1f2328;
  --fg-muted: #57606a;
  --bg: #ffffff;
  --bg-subtle: #f6f8fa;
  --border: #d0d7de;
  --accent-user: #0969da;
  --accent-assistant: #1a7f37;
  --accent-system: #57606a;
  --accent-event: #bf3989;
}
@media (prefers-color-scheme: dark) {
  :root {
    --fg: #e6edf3;
    --fg-muted: #8b949e;
    --bg: #0d1117;
    --bg-subtle: #161b22;
    --border: #30363d;
    --accent-user: #58a6ff;
    --accent-assistant: #3fb950;
    --accent-system: #8b949e;
    --accent-event: #d2a8ff;
  }
}
* { box-sizing: border-box; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", Arial, "PingFang SC", "Microsoft YaHei", sans-serif;
  color: var(--fg); background: var(--bg);
  margin: 0; padding: 24px; line-height: 1.6;
}
.timeline-doc { max-width: 880px; margin: 0 auto; }
.doc-title {
  font-size: 28px; margin: 0 0 16px; padding-bottom: 12px;
  border-bottom: 2px solid var(--border);
}
.meta-card {
  background: var(--bg-subtle); border: 1px solid var(--border);
  border-radius: 8px; padding: 16px 20px; margin: 16px 0 24px;
}
.meta-card h2 { margin: 0 0 12px; font-size: 16px; color: var(--fg-muted); }
.meta-card dl { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; margin: 0; font-size: 14px; }
.meta-card dt { color: var(--fg-muted); }
.meta-card dd { margin: 0; }
.meta-card code { background: var(--bg); padding: 1px 6px; border-radius: 4px; font-size: 13px; }
.timeline-body h2 { font-size: 20px; margin: 24px 0 12px; }
.timeline-list { list-style: none; padding: 0; margin: 0; }
.timeline-item {
  border-left: 3px solid var(--border);
  padding: 8px 16px;
  margin: 8px 0;
  background: var(--bg-subtle);
  border-radius: 0 6px 6px 0;
}
.timeline-item.msg-user { border-left-color: var(--accent-user); }
.timeline-item.msg-assistant { border-left-color: var(--accent-assistant); }
.timeline-item.msg-system { border-left-color: var(--accent-system); }
.timeline-item.msg-event { border-left-color: var(--accent-event); background: var(--bg); border: 1px dashed var(--accent-event); border-left-width: 3px; }
.item-meta { display: flex; gap: 12px; font-size: 12px; color: var(--fg-muted); margin-bottom: 6px; }
.item-role { font-weight: 600; }
.msg-user .item-role { color: var(--accent-user); }
.msg-assistant .item-role { color: var(--accent-assistant); }
.msg-system .item-role { color: var(--accent-system); }
.msg-event .item-role { color: var(--accent-event); }
.item-desc { font-size: 13px; color: var(--fg-muted); font-style: italic; margin-bottom: 6px; }
.item-content { white-space: pre-wrap; word-wrap: break-word; font-size: 14px; }
.item-content p { margin: 0 0 8px; }
.item-content p:last-child { margin: 0; }
.empty { color: var(--fg-muted); font-style: italic; }
.appendix { margin-top: 32px; padding-top: 16px; border-top: 1px solid var(--border); }
.appendix h2 { font-size: 18px; }
.appendix ul { padding-left: 20px; }
.appendix li { margin: 4px 0; font-size: 14px; }
.appendix .tt { color: var(--fg-muted); }
@media print {
  body { padding: 0; }
  .timeline-item { break-inside: avoid; }
}
</style>
"#;

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

/// Markdown 段落块转义：保留换行，每行前不加转义（因为 content 是大段文本），
/// 但在末尾确保有换行。
fn escape_md_block(s: &str) -> String {
    // 不对整段做字符级转义，避免可读性受损；只在末尾保证换行
    let mut out = s.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// 把消息内容转为 HTML：保留换行，段落化空行分隔的块。
fn content_to_html(s: &str) -> String {
    // 先 HTML 转义，再按空行切分段落
    let escaped = escape_html(s);
    let paragraphs: Vec<&str> = escaped.split("\n\n").collect();
    if paragraphs.len() == 1 {
        // 单段：保留换行
        return format!("<p>{}</p>", paragraphs[0].replace('\n', "<br>\n"));
    }
    paragraphs
        .iter()
        .map(|p| format!("<p>{}</p>", p.replace('\n', "<br>\n")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::{WorldClock, WorldEvent};
    use crate::chat_store::ChatLog;

    fn make_log(messages: Vec<(&str, &str)>, timestamps: Vec<Option<&str>>) -> ChatLog {
        // 通过 JSON 构造 ChatLog，绕开 scope_session_id 私有字段（serde default 给 None）。
        let msgs: Vec<serde_json::Value> = messages
            .into_iter()
            .map(|(role, content)| serde_json::json!({"role": role, "content": content}))
            .collect();
        let ts_arr: Vec<serde_json::Value> = timestamps
            .into_iter()
            .map(|t| match t {
                Some(s) => serde_json::Value::String(s.to_string()),
                None => serde_json::Value::Null,
            })
            .collect();
        let n = msgs.len();
        let json = serde_json::json!({
            "session_id": "test-session",
            "character_id": "test-char",
            "messages": msgs,
            "message_ids": vec!["m0".to_string(); n],
            "message_timestamps": ts_arr,
            "message_candidates": vec![Vec::<String>::new(); n],
            "message_swipe_index": vec![0u64; n],
            "message_parents": vec![Option::<String>::None; n],
            "active_leaf": null,
            "created_at": "2026-01-01T00:00:00+00:00",
            "updated_at": "2026-01-01T00:00:00+00:00",
        });
        serde_json::from_value(json).expect("ChatLog should deserialize from test fixture")
    }

    fn make_event(id: &str, name: &str, triggered: bool, time_trigger: Option<u64>) -> WorldEvent {
        WorldEvent {
            id: id.to_string(),
            name: name.to_string(),
            description: format!("desc for {}", name),
            trigger_keywords: vec![],
            min_turn: None,
            time_trigger,
            content: format!("content for {}", name),
            triggered,
        }
    }

    #[test]
    fn build_entries_empty_log_and_events() {
        let log = make_log(vec![], vec![]);
        let events = vec![];
        let entries = build_entries(&log, &events).expect("build_entries failed");
        assert!(entries.is_empty());
    }

    #[test]
    fn build_entries_only_chat_messages_timed() {
        let log = make_log(
            vec![("user", "hi"), ("assistant", "hello"), ("user", "bye")],
            vec![
                Some("2026-01-01T10:00:00+00:00"),
                Some("2026-01-01T10:01:00+00:00"),
                Some("2026-01-01T10:02:00+00:00"),
            ],
        );
        let entries = build_entries(&log, &[]).expect("build_entries failed");
        assert_eq!(entries.len(), 3);
        // 应按 ts 升序
        if let TimelineEntry::ChatMessage { ts, role, .. } = &entries[0] {
            assert_eq!(ts.as_deref(), Some("2026-01-01T10:00:00+00:00"));
            assert_eq!(role, "user");
        } else {
            panic!("expected ChatMessage");
        }
    }

    #[test]
    fn build_entries_untimed_messages_go_last() {
        let log = make_log(
            vec![
                ("user", "untimed"),
                ("assistant", "timed-later"),
                ("user", "timed-earlier"),
            ],
            vec![
                None,
                Some("2026-01-01T10:01:00+00:00"),
                Some("2026-01-01T10:00:00+00:00"),
            ],
        );
        let entries = build_entries(&log, &[]).expect("build_entries failed");
        assert_eq!(entries.len(), 3);
        // timed 排在前面，按 ts 升序
        match &entries[0] {
            TimelineEntry::ChatMessage { ts, content, .. } => {
                assert_eq!(ts.as_deref(), Some("2026-01-01T10:00:00+00:00"));
                assert_eq!(content, "timed-earlier");
            }
            _ => panic!("expected timed message first"),
        }
        match &entries[1] {
            TimelineEntry::ChatMessage { ts, content, .. } => {
                assert_eq!(ts.as_deref(), Some("2026-01-01T10:01:00+00:00"));
                assert_eq!(content, "timed-later");
            }
            _ => panic!("expected timed message second"),
        }
        match &entries[2] {
            TimelineEntry::ChatMessage { ts, content, .. } => {
                assert!(ts.is_none());
                assert_eq!(content, "untimed");
            }
            _ => panic!("expected untimed message last"),
        }
    }

    #[test]
    fn build_entries_world_event_with_time_trigger_goes_first() {
        let log = make_log(
            vec![("user", "msg")],
            vec![Some("2026-01-01T10:00:00+00:00")],
        );
        let events = vec![
            make_event("e1", "黎明", true, Some(6)),
            make_event("e2", "黄昏", true, Some(18)),
        ];
        let entries = build_entries(&log, &events).expect("build_entries failed");
        // 前 2 个是 world events，按 time_trigger 升序
        assert_eq!(entries.len(), 3);
        if let TimelineEntry::WorldEvent {
            name, time_trigger, ..
        } = &entries[0]
        {
            assert_eq!(name, "黎明");
            assert_eq!(*time_trigger, Some(6));
        } else {
            panic!("expected WorldEvent first");
        }
        if let TimelineEntry::WorldEvent {
            name, time_trigger, ..
        } = &entries[1]
        {
            assert_eq!(name, "黄昏");
            assert_eq!(*time_trigger, Some(18));
        } else {
            panic!("expected WorldEvent second");
        }
        assert!(matches!(&entries[2], TimelineEntry::ChatMessage { .. }));
    }

    #[test]
    fn build_entries_world_event_without_time_trigger_goes_after_messages() {
        let log = make_log(
            vec![("user", "msg")],
            vec![Some("2026-01-01T10:00:00+00:00")],
        );
        let events = vec![make_event("e1", "突发", true, None)];
        let entries = build_entries(&log, &events).expect("build_entries failed");
        assert_eq!(entries.len(), 2);
        assert!(matches!(&entries[0], TimelineEntry::ChatMessage { .. }));
        assert!(matches!(&entries[1], TimelineEntry::WorldEvent { .. }));
    }

    #[test]
    fn build_entries_pending_events_excluded_from_entries() {
        let log = make_log(vec![("user", "msg")], vec![None]);
        let events = vec![
            make_event("e1", "已触发", true, None),
            make_event("e2", "未触发", false, Some(10)),
        ];
        let entries = build_entries(&log, &events).expect("build_entries failed");
        // 只应有一个已触发 event + 一个 message
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn to_markdown_empty_timeline() {
        let timeline = TimelineExport {
            generated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character_id: "alice".to_string(),
            session_id: "s1".to_string(),
            session_created_at: "2026-01-01T00:00:00+00:00".to_string(),
            session_updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character: Some(CharacterSnapshot {
                character_id: "alice".to_string(),
                name: "Alice".to_string(),
            }),
            world_clock: Some(WorldClockSnapshot::from(&WorldClock::default())),
            entries: vec![],
            pending_events: vec![],
            message_count: 0,
            triggered_event_count: 0,
        };
        let md = to_markdown(&timeline);
        assert!(md.contains("Alice"));
        assert!(md.contains("剧情时间线"));
        assert!(md.contains("暂无消息"));
    }

    #[test]
    fn to_markdown_with_messages_and_events() {
        let timeline = TimelineExport {
            generated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character_id: "alice".to_string(),
            session_id: "s1".to_string(),
            session_created_at: "2026-01-01T00:00:00+00:00".to_string(),
            session_updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character: None,
            world_clock: None,
            entries: vec![
                TimelineEntry::WorldEvent {
                    event_id: "e1".to_string(),
                    name: "黎明".to_string(),
                    description: "天亮了".to_string(),
                    content: "第一缕阳光照进森林".to_string(),
                    time_trigger: Some(6),
                    triggered: true,
                },
                TimelineEntry::ChatMessage {
                    message_id: Some("m1".to_string()),
                    ts: Some("2026-01-01T10:00:00+00:00".to_string()),
                    role: "user".to_string(),
                    content: "你好".to_string(),
                },
                TimelineEntry::ChatMessage {
                    message_id: Some("m2".to_string()),
                    ts: Some("2026-01-01T10:01:00+00:00".to_string()),
                    role: "assistant".to_string(),
                    content: "你好啊".to_string(),
                },
            ],
            pending_events: vec![make_event("e2", "黄昏", false, Some(18))],
            message_count: 2,
            triggered_event_count: 1,
        };
        let md = to_markdown(&timeline);
        assert!(md.contains("世界事件：黎明"));
        assert!(md.contains("你好"));
        assert!(md.contains("你好啊"));
        assert!(md.contains("附录：未触发世界事件"));
        assert!(md.contains("黄昏"));
    }

    #[test]
    fn to_html_includes_meta_and_entries() {
        let timeline = TimelineExport {
            generated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character_id: "alice".to_string(),
            session_id: "s1".to_string(),
            session_created_at: "2026-01-01T00:00:00+00:00".to_string(),
            session_updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character: Some(CharacterSnapshot {
                character_id: "alice".to_string(),
                name: "Alice".to_string(),
            }),
            world_clock: None,
            entries: vec![TimelineEntry::ChatMessage {
                message_id: None,
                ts: Some("2026-01-01T10:00:00+00:00".to_string()),
                role: "user".to_string(),
                content: "<script>alert(1)</script>".to_string(),
            }],
            pending_events: vec![],
            message_count: 1,
            triggered_event_count: 0,
        };
        let html = to_html(&timeline);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Alice"));
        assert!(html.contains("&lt;script&gt;")); // HTML 转义
        assert!(html.contains("msg-user"));
    }

    #[test]
    fn to_html_empty_timeline() {
        let timeline = TimelineExport {
            generated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character_id: "bob".to_string(),
            session_id: "s2".to_string(),
            session_created_at: "2026-01-01T00:00:00+00:00".to_string(),
            session_updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            character: None,
            world_clock: None,
            entries: vec![],
            pending_events: vec![],
            message_count: 0,
            triggered_event_count: 0,
        };
        let html = to_html(&timeline);
        assert!(html.contains("暂无消息"));
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
    fn content_to_html_preserves_paragraphs() {
        let html = content_to_html("para1\n\npara2");
        assert!(html.contains("<p>para1</p>"));
        assert!(html.contains("<p>para2</p>"));
    }

    #[test]
    fn content_to_html_preserves_single_newlines() {
        let html = content_to_html("line1\nline2");
        assert!(html.contains("<br>"));
    }

    #[test]
    fn export_format_default_is_json() {
        let fmt = ExportFormat::default();
        assert!(matches!(fmt, ExportFormat::Json));
    }

    #[test]
    fn export_format_deserialize_lowercase() {
        let md: ExportFormat = serde_json::from_str("\"markdown\"").unwrap();
        assert!(matches!(md, ExportFormat::Markdown));
        let html: ExportFormat = serde_json::from_str("\"html\"").unwrap();
        assert!(matches!(html, ExportFormat::Html));
    }

    #[test]
    fn world_clock_snapshot_from_world_clock() {
        let clock = WorldClock {
            current_time: 42,
            advance_per_turn: 2,
            time_unit: "hour".to_string(),
            display_format: Some("第{day}天 {hour}:00".to_string()),
        };
        let snap = WorldClockSnapshot::from(&clock);
        assert_eq!(snap.current_time, 42);
        assert_eq!(snap.advance_per_turn, 2);
        assert_eq!(snap.time_unit, "hour");
        assert!(snap.display.contains("第"));
    }

    #[test]
    fn build_timeline_integration_with_empty_data() {
        // 用 tempdir 创建一个空数据目录，build_timeline 应返回空时间线
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // 不创建任何文件，build_timeline 应能处理 NotFound
        let timeline = build_timeline(root, "ghost-char", None, None).unwrap();
        assert_eq!(timeline.character_id, "ghost-char");
        assert_eq!(timeline.message_count, 0);
        assert!(timeline.entries.is_empty());
        // world_clock 与 world_events 不存在 → 空 / 默认
        assert!(timeline.world_clock.is_some()); // 默认值也算 Some
        assert!(timeline.pending_events.is_empty());
    }

    #[test]
    fn build_timeline_with_real_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let char_id = "alice";

        // 创建角色目录
        let char_dir = root.join("characters").join(char_id);
        std::fs::create_dir_all(&char_dir).unwrap();

        // 创建命名 session 目录 + 空 chat_log.jsonl + meta
        let session_dir = char_dir
            .join("sessions")
            .join("11111111-2222-3333-4444-555555555555")
            .join("history");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("chat_log.jsonl"),
            "{\"role\":\"user\",\"content\":\"hi\",\"ts\":\"2026-01-01T10:00:00+00:00\"}\n\
             {\"role\":\"assistant\",\"content\":\"hello\"}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("chat_log_meta.json"),
            format!(
                "{{\"session_id\":\"11111111-2222-3333-4444-555555555555\",\"character_id\":\"{}\",\"created_at\":\"2026-01-01T00:00:00+00:00\",\"updated_at\":\"2026-01-01T00:00:00+00:00\"}}",
                char_id
            ),
        )
        .unwrap();

        // 写 world_events.json（含一个已触发、一个未触发）
        let events = vec![
            WorldEvent {
                id: "e1".to_string(),
                name: "黎明".to_string(),
                description: "天亮".to_string(),
                trigger_keywords: vec![],
                min_turn: None,
                time_trigger: Some(6),
                content: "阳光".to_string(),
                triggered: true,
            },
            WorldEvent {
                id: "e2".to_string(),
                name: "黄昏".to_string(),
                description: "天黑".to_string(),
                trigger_keywords: vec![],
                min_turn: None,
                time_trigger: Some(18),
                content: "日落".to_string(),
                triggered: false,
            },
        ];
        std::fs::write(
            char_dir.join("world_events.json"),
            serde_json::to_vec_pretty(&events).unwrap(),
        )
        .unwrap();

        let sid = SessionId::parse("11111111-2222-3333-4444-555555555555").unwrap();
        let timeline =
            build_timeline(root, char_id, Some(&sid), Some("Alice".to_string())).unwrap();

        assert_eq!(timeline.character_id, char_id);
        assert_eq!(timeline.session_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(timeline.message_count, 2);
        assert_eq!(timeline.triggered_event_count, 1);
        assert_eq!(timeline.pending_events.len(), 1);
        // 1 world event + 2 messages = 3 entries
        assert_eq!(timeline.entries.len(), 3);
        // 第一个是 world event
        assert!(matches!(
            &timeline.entries[0],
            TimelineEntry::WorldEvent { .. }
        ));
        // 后两个是 chat message
        assert!(matches!(
            &timeline.entries[1],
            TimelineEntry::ChatMessage { .. }
        ));
        assert!(matches!(
            &timeline.entries[2],
            TimelineEntry::ChatMessage { .. }
        ));

        // 渲染为 markdown / html 应无 panic
        let md = to_markdown(&timeline);
        assert!(md.contains("Alice"));
        assert!(md.contains("黎明"));
        let html = to_html(&timeline);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Alice"));
    }
}
