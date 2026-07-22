/* ============================================================
 * AIRP 控制台 · 屏清单注册表（screens.js）
 * 单一注册点：增删屏只需改本文件 + 增删 screens/ 下对应文件。
 * 导航首页（index.html）与导航贴片（app.js）都从这里读取。
 * design 字段记录对应的 Ardot 画板节点 ID，便于回溯源设计。
 * ============================================================ */

const AIRP_GROUPS = [
  { key: "main",  name: "主屏",     range: "01–08", desc: "日常使用的八个核心界面" },
  { key: "sub",   name: "子页面",   range: "09–15", desc: "主屏下钻的编辑与弹窗" },
  { key: "ext",   name: "扩展功能", range: "16–25", desc: "启动向导与规划预留能力" },
  { key: "state", name: "状态变体", range: "26–31", desc: "空/加载/错误/断流等核心流变体" },
];

const AIRP_SCREENS = [
  // ---- 主屏 ----
  { id: "01", slug: "role-list",          title: "角色列表",       group: "main",  design: "2:99",  file: "screens/01-role-list.html" },
  { id: "02", slug: "chat-space",         title: "对话空间",       group: "main",  design: "2:224", file: "screens/02-chat-space.html" },
  { id: "03", slug: "workbench",          title: "工作台",         group: "main",  design: "2:311", file: "screens/03-workbench.html" },
  { id: "04", slug: "world-book",         title: "世界书",         group: "main",  design: "6:40",  file: "screens/04-world-book.html" },
  { id: "05", slug: "presets-models",     title: "预设与模型",     group: "main",  design: "6:120", file: "screens/05-presets-models.html" },
  { id: "06", slug: "user-persona",       title: "用户人设",       group: "main",  design: "6:197", file: "screens/06-user-persona.html" },
  { id: "07", slug: "agent-runs",         title: "Agent 运行",     group: "main",  design: "6:264", file: "screens/07-agent-runs.html" },
  { id: "08", slug: "settings",           title: "设置",           group: "main",  design: "7:35",  file: "screens/08-settings.html" },
  // ---- 子页面 ----
  { id: "09", slug: "persona-edit",       title: "人格设定",       group: "sub",   design: "7:112", file: "screens/09-persona-edit.html" },
  { id: "10", slug: "greetings",          title: "开场白",         group: "sub",   design: "7:169", file: "screens/10-greetings.html" },
  { id: "11", slug: "llm-params",         title: "LLM 参数",       group: "sub",   design: "7:218", file: "screens/11-llm-params.html" },
  { id: "12", slug: "entry-edit",         title: "条目编辑",       group: "sub",   design: "7:278", file: "screens/12-entry-edit.html" },
  { id: "13", slug: "import-card",        title: "导入角色卡",     group: "sub",   design: "7:335", file: "screens/13-import-card.html" },
  { id: "14", slug: "message-swipe",      title: "消息操作·Swipe", group: "sub",   design: "7:373", file: "screens/14-message-swipe.html" },
  { id: "15", slug: "confirm-modal",      title: "确认弹窗",       group: "sub",   design: "7:439", file: "screens/15-confirm-modal.html" },
  // ---- 扩展功能 ----
  { id: "16", slug: "onboarding",         title: "启动向导",       group: "ext",   design: "8:1",   file: "screens/16-onboarding.html" },
  { id: "17", slug: "memory-state",       title: "记忆与状态",     group: "ext",   design: "8:51",  file: "screens/17-memory-state.html" },
  { id: "18", slug: "group-chat",         title: "群聊编排",       group: "ext",   design: "8:114", file: "screens/18-group-chat.html",    planned: true },
  { id: "19", slug: "branch-tree",        title: "分支树",         group: "ext",   design: "8:202", file: "screens/19-branch-tree.html",   planned: true },
  { id: "20", slug: "assembly-preview",   title: "装配预览",       group: "ext",   design: "8:253", file: "screens/20-assembly-preview.html" },
  { id: "21", slug: "usage-quota",        title: "用量配额",       group: "ext",   design: "8:337", file: "screens/21-usage-quota.html" },
  { id: "22", slug: "backup-restore",     title: "备份恢复",       group: "ext",   design: "8:402", file: "screens/22-backup-restore.html" },
  { id: "23", slug: "diagnostics",        title: "诊断中心",       group: "ext",   design: "8:457", file: "screens/23-diagnostics.html" },
  { id: "24", slug: "plugins",            title: "插件技能",       group: "ext",   design: "8:532", file: "screens/24-plugins.html",       planned: true },
  { id: "25", slug: "notes-connections",  title: "笔记与连接",     group: "ext",   design: "8:575", file: "screens/25-notes-connections.html" },
  // ---- 状态变体 ----
  { id: "26", slug: "chat-empty",         title: "对话·空状态",     group: "state", design: "10:1",   file: "screens/26-chat-empty.html" },
  { id: "27", slug: "chat-reconnect",     title: "对话·断流恢复",   group: "state", design: "10:26",  file: "screens/27-chat-reconnect.html" },
  { id: "28", slug: "chat-errors",        title: "对话·错误与配额", group: "state", design: "10:60",  file: "screens/28-chat-errors.html" },
  { id: "29", slug: "list-empty-skeleton",title: "列表·空态与骨架", group: "state", design: "10:94",  file: "screens/29-list-empty-skeleton.html" },
  { id: "30", slug: "notifications",      title: "通知与 Toast",    group: "state", design: "10:133", file: "screens/30-notifications.html" },
  { id: "31", slug: "message-types",      title: "RP 消息类型",     group: "state", design: "10:171", file: "screens/31-message-types.html" },
];

/* 页面流转关系（index.html 流转图数据；solid=主导航，dashed=模态/状态覆盖） */
const AIRP_FLOWS = [
  ["16", "01", "solid"], ["01", "03", "solid"], ["03", "02", "solid"],
  ["03", "09", "solid"], ["09", "10", "solid"], ["10", "11", "solid"],
  ["02", "14", "solid"], ["04", "12", "solid"],
  ["08", "21", "solid"], ["08", "22", "solid"], ["08", "23", "solid"],
  ["02", "19", "solid"], ["02", "20", "solid"],
  ["06", "20", "solid"], ["17", "20", "solid"],
  ["05", "20", "solid"], ["25", "20", "solid"],
  ["01", "13", "dashed"],
  ["02", "26", "dashed"], ["02", "27", "dashed"], ["02", "28", "dashed"],
];
