# AIRP 控制台 · 权威样板规范（STYLEGUIDE）

本目录是 AIRP 控制台前端的**权威视觉样板**（golden sample）。它与 Ardot 设计稿
「AIRP Engine Console」（33 屏）逐屏对应，是派生 **WebUI** 与 **桌面端 UI** 的
视觉母本。后续 WebUI 与桌面端临时 UI 均须基于本样板开发。派生实现必须逐像素对齐
本样板；与本样板冲突时，以本样板为准并回写 issue。

---

## 1. 目录结构

```text
airp-engine-console/
├── index.html            # 导航首页：可点击页面流转图 + 分组屏清单（数据驱动）
├── STYLEGUIDE.md         # 本文件
├── assets/
│   ├── tokens.css        # 设计令牌（单一事实源，对应画布变量集 AIRP Tokens）
│   ├── base.css          # reset + 画布规则 + 排版助手
│   ├── components.css    # 共享组件类（全部界面的零件盒）
│   ├── screens.js        # 屏清单注册表（增删屏的唯一注册点）
│   └── app.js            # 样板脚手架：右下角返回导航贴片（非设计内容，勿携带）
├── screens/              # 33 屏，每屏一个独立 HTML，编号与画布画板一致
│   ├── 01-role-list.html … 33-wizard-model.html
└── exports/              # 设计稿归档（32 屏 PDF + 流转图 PNG），只读参照
```

## 2. 设计令牌（tokens.css）

只允许引用变量，禁止在界面代码中散落硬编码色值。新增令牌必须先加进
`tokens.css` 并注明来源。核心令牌：

| 类别 | 令牌 | 值 |
|---|---|---|
| 表面 | `--bg-base` / `--bg-surface` / `--bg-subtle` | `#FAFAF7` / `#FFFFFF` / `#F5F2F0` |
| 描边 | `--border-default` | `#E0DBD9` |
| 文本 | `--text-primary` / `--text-secondary` / `--text-tertiary` | `#1A1A1F` / `#73706E` / `#9E998F` |
| 品牌 | `--primary` / `--primary-strong` / `--primary-action` / `--primary-action-hover` / `--primary-tint` | `#C4663B` / `#A85430` / `#A85430` / `#8E4528` / `#FAEDE6` |
| 语义 | `--success` `--warning` `--danger` + `-tint` | `#3D9E70` `#D98C21` `#CC4559`（tint 见文件） |
| 深色表面 | `--ink` | `#2A2927`（Toast） |
| 圆角 | `--radius-input/card/modal/pill` | `6 / 10 / 14 / 9999 px` |
| 间距 | `--space-1/2/3/4/6` | `4 / 8 / 12 / 16 / 24 px` |
| 字体 | `--font-body` / `--font-mono` | Inter / JetBrains Mono（含中文与系统回退栈） |
| 阴影 | `--shadow-card` / `--shadow-pop` | 见文件 |

图表辅助色（非语义 token）允许局部定义并注释，例如装配预览堆叠条的
`--chart-history: #8A94A6`。

## 3. 画布与布局规则

- **基准画布 1440×900**，样板按原像素还原（`.canvas`）。窗口更窄时居中 +
  横向滚动，不做响应式重排——响应式策略由各派生实现自行决定，但令牌不变。
- **两套框架**：
  - 框架 A「顶栏 + 应用侧边栏」（仅 01 角色列表）：`.topbar`（含引擎地址框）
    + `.sidebar-app`（260px：搜索/分组/主按钮/导航项/配置卡）+ `.pane-main`。
  - 框架 B「顶栏 + 可选侧栏」（其余全部屏）：`.topbar`（54px）+ `.screen-body`
    （`.pane-side` 240/260px / `.pane-main` / `.pane-right` 300/320px）。
- 顶栏构成固定：Logo 橙块 `A` + 产品名 + 分隔线 + 面包屑；右侧状态区
  （`.status-pill` 胶囊 / `.tag` / 按钮）。
- 中文界面；字号阶梯：正文 13px、次级 11–12px、辅助/图注 10px、页标题 20px、
  统计大数字 28px；`mono` 类信息（时间、tokens、版本、文件、错误码）一律
  `--font-mono`。

## 4. 组件清单（components.css）

| 组件 | 类 | 使用屏 |
|---|---|---|
| 顶栏 | `.topbar`（54px）`.icon-btn`（32×32 图标钮） | 全部 |
| 顶栏状态 | `.status-pill(.ok/.warn/.danger)`（tint 胶囊，顶栏标准件）/ `.status-dot`（行内裸文字） | 全部 / 16 |
| 应用侧栏/导航 | `.sidebar-app`（260px）`.nav-item(.active)`（tint 底 + strong 字）`.side-card` | 01 |
| 内容侧栏 | `.pane-side`（240px，`.wide`=260px）`.pane-item(.active)` | 02–06/09/10/17 |
| 右侧面板 | `.pane-right`（300px，`.wide`=320px） | 02/07/14/18 |
| 按钮 | `.btn` `.btn-primary` `.btn-secondary` `.btn-danger` `.btn-danger-solid` | 全部 |
| 标签 | `.tag` `.tag-success/warning/danger/primary/neutral`（`.mono`） | 全部 |
| 卡片 | `.card` `.stat-card` `.char-card` `.card.danger-zone` | 01/05/08/17… |
| 表单 | `.field` `.input` `.select` `.textarea` | 03/08/09/12/16 |
| 滑杆 | `.slider` `.slider-scale` | 05/09/18 |
| 开关 | `.switch(.on)` | 11/12 |
| 表格 | `.table` | 04/20/23 |
| 消息 | `.msg-row` `.avatar` `.bubble(.user/.ai/.warn)` `.caret` | 02/14/27/31 |
| RP 消息件 | `.reason-chip` `.action-block` `.state-change` `.scene-divider` | 31 |
| 输入区 | `.composer(.locked)` `.send-btn` `.composer-hint` | 02/27/28 |
| 事件日志 | `.log-item` | 02 |
| 标签页 | `.tabbar` `.tab(.active)` | 03/09/10/11 |
| 横幅/警示卡 | `.banner(.warn/.neutral)` `.alert-card(.danger/.warn)` | 24/27/28 |
| 进度条 | `.progress(.warn)` | 28 |
| 空态/骨架 | `.empty-state` `.skeleton` | 26/29 |
| 通知/Toast | `.notification(.success/.warn/.danger)` `.toast` `.round-ico` | 30 |
| 模态 | `.modal-mask` `.modal` | 13/15 |
| 工具调用卡 | `.tool-card(.pending)` | 07 |
| 覆盖项行 | `.override-row` | 11 |
| NL enhance 区 | `.nl-zone` `.nl-planned-tag` `.nl-diff-label` `.nl-confirm` `.nl-note` | 03/04（规划中·契约未交付） |
| diff 视图 | `.diff-block` `.diff-line(.del/.add)` | 03/04 NL 区内 |
| JSON 高级折叠区 | `.json-advanced` `.ja-bar` `.ja-body` `.ja-warn` `.ja-code` `.ja-actions` | 03/04 |
| 行内操作列 | `.op-col` `.op-link(.del)` | 04 表格操作列 |
| Combobox | `.combobox(.error-state)` `.combobox-error` | 08 降级态 |
| 头像色相 | `.avatar.hue-violet/green/blue/red/amber` | 01/18 |

图标目前用文字/符号占位（`.ico`）。派生实现应替换为统一图标库（描边 1.5px、
圆角一致），尺寸沿用占位规格（14/16/18px）。

## 5. 屏文件模板

每屏是**完整独立 HTML**（这是有意为之：任何一屏都可单独打开查阅，
派生实现复制时看到的是完整上下文）：

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>NN 屏名 · AIRP 控制台样板</title>
<link rel="stylesheet" href="../assets/tokens.css">
<link rel="stylesheet" href="../assets/base.css">
<link rel="stylesheet" href="../assets/components.css">
<style>/* 仅本屏私有的布局样式，通用件一律进 components.css */</style>
</head>
<body>
<div class="canvas"> … </div>
<script src="../assets/screens.js"></script>
<script src="../assets/app.js"></script>
</body>
</html>
```

规则：
- 屏私有样式放在屏内 `<style>`，跨屏复用 ≥2 次就提升到 `components.css`。
- 文案即设计稿文案，禁止随意改写（含引擎术语：rev、commit_state、provenance…）。
- `app.js` 注入的右下角贴片是样板脚手架（`data-sample-chrome`），不是设计内容。

## 6. 增删屏流程（迭代维护）

1. 在 Ardot 画布上新增/修改画板（保持编号约定 NN）。
2. 在 `assets/screens.js` 的 `AIRP_SCREENS` 增删条目（id/slug/title/group/
   design/file；规划预留加 `planned: true`）。
3. 在 `screens/` 增删对应 HTML 文件（复制最接近的现有屏改造）。
4. 若导航关系变化，更新 `AIRP_FLOWS`（index.html 流转图自动重绘）。
5. 重新导出设计稿 PDF 归档到 `exports/`（可选但推荐）。

## 7. 派生实现注意（WebUI / 桌面端 UI）

- **先映射令牌，再写组件**：把 `tokens.css` 整体翻译成目标技术栈的令牌层
  （CSS variables / TS 常量 / 主题对象），组件只许消费令牌。
- 组件映射以第 4 节清单为准，逐个实现并截图与本样板对拍。
- 本样板是静态快照：交互状态（hover/focus/流式光标/骨架呼吸/转圈）已用
  CSS 表达，派生实现补全真实行为即可，视觉不得偏离。
- 状态变体（26–31）与主界面同等重要：空/加载/错误/断流/配额/通知是
  核心流的一部分，不是可选抛光项。
- 规划预留屏（18/19/24，清单中 `planned: true`）只定义视觉与信息架构，
  不承诺当前能力，派生实现可后置。
- **边界约定**：08「设置」管理 LLM Provider 底层连接（endpoint/key/picker），是引擎
  全局配置的唯一入口；25「笔记与连接」管理角色/场景绑定的 profile 打包，属于场景级。
  两者不可交叉——08 不做角色绑定，25 不做 key 管理。
- 数据安全语义（不可视觉降级）：密钥不回显、partially_committed 禁止盲重发、
  危险操作二次确认（15）、破坏性 tool_call 需控制台确认（07）。

## 8. 溯源

- 设计稿：Ardot「AIRP Engine Console」file id 706339765412318；
  每屏的 `design` 字段（screens.js）记录画板节点 ID。
- 令牌来源：画布变量集「AIRP Tokens / Light」（2026-07-22 fetch_variables）。
- 归档：`exports/AIRP Engine Console.pdf`（33 屏）、`exports/13_1.png`（流转图）。
