# PR #314 视觉风格审查（Visual Style Review）

- **日期**：2026-07-25
- **审计原则**：AGENTS.md §11.1（独立审计 / 可提己见 / 可质疑历史并查证）
- **范围**：PR #314 WebUI 改动（`split-phase1` 相对 `split-base`）
- **方法**：调取画布 + 代码令牌提取 + 像素对比
- **结论**：🔴 视觉风格**不符合**基板风格（relationship-graph 整屏硬编码外部调色板）；chat-space/console-runtime **符合**基板（附带 1 处未定义变量与 1 处 CSS/JS 颜色不一致需修复）

---

## 1. 方法与画布调用

- 已加载 `ardot-design-core` 技能（其规定「所有画布操作必须通过 ardot MCP 工具」）。
- 本会话 **ardot MCP 服务未连接**（connectors 全 disconnected；`ToolSearch` 全局搜索未发现 `mcp__ardot__*` 工具）。
- 按本仓工作记忆既定兜底（"Ardot 适配器断连时用 `exports/` PDF + 离线取色"），改用 `airp-engine-console/exports/AIRP Engine Console.pdf` 作为画布视觉基线——该 PDF 即 Ardot 画布的导出物（32 屏 + 流转图），是单一事实源。
- 渲染工具：受管 venv `~/.workbuddy/binaries/python/envs/default`（新建）+ `pymupdf 1.28.0` + `Pillow 12.x`。所有视觉资产落在 `.workbuddy/review/pr314/`（未污染工作区）。

## 2. 视觉基线（画布关键屏）

### p01 角色列表 / p02 聊天空间 / p03 角色卡编辑
- **品牌色**：暖陶土橙 `#C4663B`（logo / 主按钮 / 选中态底）；hover `#A85430`；浅橙底 `#FAEDE6`
- **表面**：页面底 `#FAFAF7`、卡 `#FFFFFF`、弱表面 `#F5F2F0`
- **圆角**：输入 6px、卡 10px
- **字体**：Inter 正文 / JetBrains Mono 用于 meta 与路径
- **状态语义色**：success `#3D9E70`（已连接胶囊）、warning `#D98C21`、danger `#CC4559`
- 全屏未出现冷色调（靛蓝 / 蓝灰 / Tailwind 彩虹）

## 3. 逐改动区域视觉判断

### 3.1 `chat-space.css`（+9 行）— ✅ 视觉符合基板（1 处未定义变量待修）
- 全文件使用令牌（`--primary-tint` 激活会话、 `--bg-subtle` 世界书卡、 `--radius-input` swipe/load-more、 `--font-mono` HUD 数值、 `--danger` 停止按钮、 `--text-tertiary/secondary/primary` 等），与画布 p02 聊天空间一致。
- 状态 HUD（`.state-hud`/`.hud-bar`/`.hud-fill`）的视觉骨架（圆角 6px / 弱表面底 / primary 填充条 / mono 数值）与基板「事件日志」卡风格同族。
- **缺陷**：`chat-space.css:61` 使用 `var(--bg-default)` —— 该变量**不在 `tokens.css` 内**（基板定义的是 `--bg-base`）。属无效 CSS，HUD 进度条 track 背景会回退为透明（应改为 `var(--bg-base)` 或 `var(--border-default)`）。

### 3.2 `console-runtime.js`（+104/-2）— ✅ 视觉中性（无新增硬编码样式）
- 全文件 grep 未发现新引入的 hex / rgb / box-shadow / border-radius / font-family / var(--) 取值（diff 中亦无可疑样式新增）。
- 本次改动是行为层（路由分发、字段解析、列表渲染），所有视觉由既有 `base.css` / `components.css` / `chat-space.css` 等提供，依赖令牌。

### 3.3 `relationship-graph.{css,js}`（屏 34，全新）— 🔴 视觉不符合基板（重大偏差）

**画布对照**：基板无关系图谱屏（屏 34 为本批新引入），因此无直接画布基线可比对；但按项目惯例，**派生屏仍须复用 `tokens.css`**——这是设计一致性的硬约束。

**实测发现**：

1. **整屏硬编码外部调色板**（`relationship-graph.js:68-69, 134, 143, 151` + `.css:9-10`）：
   ```
   COLORS.primary = '#6366f1'   // 靛蓝，非品牌
   COLORS.edge    = '#e67e22'   // 通用橙，非品牌
   COLORS.text    = '#1e293b'   // slate-800
   TYPE_COLORS    = {friend:'#22c55e', enemy:'#ef4444', family:'#3b82f6',
                      lover:'#ec4899', rival:'#f59e0b', neutral:'#94a3b8'}  // Tailwind 默认调色板
   ctx.fillStyle  = '#64748b'
   ```
   - 与 `tokens.css` 零交集。整张力导向图将渲染成靛蓝节点 + Tailwind 彩虹关系色，**与全系统暖陶土橙风格脱节**。
   - 视觉证据：见 `_palette_compare.png`（本目录）——上排 AIRP tokens（暖橙单族 + 中性 + 三语义色），下排本屏色板（靛蓝 + 彩虹 + slate）。两套体系**无任何颜色重叠**。

2. **CSS 图例 vs JS 画布颜色不一致**（自相矛盾）：
   - CSS：`.lg-dot[data-color="primary"] { background: var(--primary, #6366f1) }` → 因 `--primary` 已定义，实际解析为 **`#C4663B`（品牌橙）**。
   - JS：`COLORS.primary = '#6366f1'` → 节点实际绘为 **靛蓝**。
   - 结果：左下角图例点显示品牌橙，节点显示靛蓝——**图例与图形对不上**。

3. **未定义变量**（`relationship-graph.css:2`）：`background: var(--bg-default)` —— 同 3.1 缺陷，变量未定义，画布容器背景回退透明。

4. **关系类型色无系统语义**：
   - friend=绿 → 松散匹配 `--success` `#3D9E70`
   - enemy=红 → 松散匹配 `--danger` `#CC4559`
   - family=蓝、lover=粉、rival=琥珀、neutral=slate → 这些色在 `tokens.css` 里**完全没有对应**，等于在系统里引入新色族，破坏一致性。

**应改为（最小修复路径）**：
- `COLORS.primary` → `var(--primary)` 取色字符串（canvas 中读 CSS 变量或 `getComputedStyle(document.documentElement).getPropertyValue('--primary').trim()`）；`COLORS.edge` 同理取 `--text-secondary` 或 `--warning`；`COLORS.text` 取 `--text-primary`。
- TYPE_COLORS 收敛到 `--success/-tint`、`--danger/-tint`、`--primary-tint`、`--warning-tint`、`--text-tertiary` 等既有令牌，或新增语义令牌（如 `--rel-family/--rel-lover/--rel-rival`）并入 `tokens.css`。
- `--bg-default` 改为 `--bg-base` 或 `--bg-subtle`。
- 图例 CSS 与画布 JS 都从同一令牌取值，保证一致。

## 4. 视觉裁决

| 区域 | 视觉风格 | 阻塞 |
|---|---|---|
| `chat-space` (02) | ✅ 符合基板 | 否（仅 1 处未定义变量为非阻塞瑕疵） |
| `console-runtime` | ✅ 视觉中性（无新增样式） | 否 |
| `relationship-graph` (34, 新屏) | 🔴 **不符合基板**（外部硬编码调色板 + 图例/画布颜色矛盾） | **是** |
| 其余（02 屏 HTML +2/-2） | —（结构层） | — |

**总判定**：视觉风格审查未通过。**关系图谱整屏必须改为复用 `tokens.css` 令牌**，否则将破坏全系统视觉一致性。

## 5. 与功能审计的关系

- 本视觉审查强化了既有审计（`docs/audits/2026-07-25-PR-314-phase1-webui-audit.md`）的 N1 项（关系图谱硬编码色 `#6366f1` 偏离品牌），并将其**升级为阻塞**：因为不仅是「局部硬编码」，而是整屏 11 个颜色值均未走令牌，且造成图例与画布视觉不一致。
- 其余阻塞项（B1/B2/B3）与本视觉审查正交，仍维持原判定。

## 6. 视觉证据清单（`.workbuddy/review/pr314/`）

- `_montage.png` —— 画布 32 屏 4×8 缩略图（导航用）
- `page_01..32.png` —— 画布全屏高分辨率（p01–p32）
- `_palette_compare.png` —— **AIRP 基板 tokens vs 关系图谱实染色板 对比图**（本次审查核心证据）
- `manifest.txt` —— 各页首段文字索引（画布为矢量，文字多为空）

---

*本报告随 PR 提交（commit 附后），作为 `docs/audits/` 系列归档。*