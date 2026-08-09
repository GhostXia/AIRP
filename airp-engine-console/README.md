# AIRP Engine Console · 权威样板

本目录是 AIRP 控制台 1–33 屏的**权威视觉样板**（golden sample），与 Ardot 设计稿
「AIRP Engine Console」逐屏对应。WebUI 已扩展到 44 屏；34–44 屏没有对应的原始
Ardot 画板，必须复用本样板的 token、布局语言和交互规则，并通过独立视觉审查。

详细规范见 [STYLEGUIDE.md](STYLEGUIDE.md)。

## 基线定义

**v0.0.2 release（2026-07-22）的 1–33 屏是视觉基线。** 后续所有样板/WebUI 变更
均以此为起点衡量。基线包含的核心能力：

- PR #294 重建的无构建、多页面、CSP 兼容同源客户端
- 角色导入/搜索、Provider 设置、Preset/Persona 管理、命名 session、流式聊天
- continue/regen、durable-ID rollback/edit/delete、Swipe/branch 管理
- Agent Run、装配预览、诊断、5-stage onboarding 向导
- PR #297/#296 的删除角色/会话、FTS5 搜索、Persona 删除解绑、状态历史

## 溯源与回退

样板与派生 WebUI 同处一个仓库，共享 commit 历史。每次同步后在此记录对应的
WebUI commit，便于回退与溯源：

| 样板同步批次 | 样板 commit | WebUI commit | WebUI 版本 | 变更摘要 |
|---|---|---|---|---|
| #304 编辑体验 | `fbcab6e` | `fbcab6e` | v0.0.2+ | NL区 / JSON高级折叠区 / diff视图 / 行内操作列 / model picker / 33屏向导 / 05重命名 |
| #304 item 1 | `3caafa1` | `3caafa1` | v0.0.2+ | model picker for onboarding step3 + console provider card |
| 基线 | `bc72c48` | `bc72c48` | v0.0.2 | 305 fix: character_id query string |

> **注意**：样板与 WebUI 在同一仓库内同步更新，所以上述 commit SHA 同时涵盖
> `airp-engine-console/` 和 `webui/` 的变更。要查看某批次对 WebUI 的具体影响，
> 运行 `git diff <样板commit> -- webui/`；对样板的影响则运行
> `git diff <样板commit> -- airp-engine-console/`。

回退到某批次状态：

```bash
# 回退 WebUI 到基线 (bc72c48)
git checkout bc72c48 -- webui/

# 回退样板到 #304 同步前
git checkout 3caafa1 -- airp-engine-console/
```

## 设计稿溯源

- **Ardot 文件**：AIRP Engine Console，file id `706339765412318`
- **画板 → 屏映射**：每屏的 `design` 字段在 `assets/screens.js` 中记录画板节点 ID
- **归档**：`exports/AIRP Engine Console.pdf`（33 屏）、`exports/13_1.png`（流转图）

## 未纳入样板屏登记

以下屏没有对应的 Ardot 画板设计稿，不在本权威样板覆盖范围内（样板只覆盖
1–33 屏，缺 32）。这些屏必须复用样板的 token、布局语言和交互规则，但在补齐
设计稿并过审查前，一律登记为「未纳入样板」：

| 屏 | WebUI 文件 | 登记状态 |
|---|---|---|
| 32 风格审查 | `webui/screens/32-style-review.html` | 未纳入样板 |
| 34 角色关系图谱 | `webui/screens/34-relationship-graph.html` | 未纳入样板 |
| 35 剧情弧编辑器 | `webui/screens/35-plot-arc.html` | 未纳入样板 |
| 36 场景插图生成 | `webui/screens/36-image-gen.html` | 未纳入样板 |
| 37 角色卡模板库 | `webui/screens/37-character-templates.html` | 未纳入样板 |
| 38 风格迁移 | `webui/screens/38-style-learn.html` | 未纳入样板 |
| 39 对话示例生成器 | `webui/screens/39-dialogue-gen.html` | 未纳入样板 |
| 40 世界书知识图谱 | `webui/screens/40-worldbook-graph.html` | 未纳入样板 |
| 41 剧情时间线导出 | `webui/screens/41-timeline-export.html` | 未纳入样板 |
| 42 角色卡版本对比 | `webui/screens/42-card-diff.html` | 未纳入样板 |
| 43 多 Provider 路由 | `webui/screens/43-provider-management.html` | 未纳入样板 |
| 44 插件工具 | `webui/screens/44-plugin-tools.html` | 未纳入样板 |

登记规则：

1. 新增无设计稿的屏**必须**在上表登记（屏号、标题、文件、状态），否则不视为交付完成。
2. 登记屏不得宣称通过样板一致性审查；「页面已实现」≠「已通过审查」。
3. 后续补齐 Ardot 画板并纳入样板后，将状态更新为已纳入并记入上方「溯源与回退」表。
4. 扩充新页面的完整硬清单见 `webui/STYLEGUIDE.md` §9「扩充新页面清单」。

## 快速开始

直接在浏览器中打开 `index.html` 即可浏览全部 33 屏样板。
每屏 HTML 也可独立打开（`screens/NN-*.html`），无需服务器。

> 2026-08-09 状态（`main@affa315` / `v0.0.5-rc.2` prerelease candidate）：WebUI 32 及 34–44 屏尚未纳入本样板目录；不得把“页面已实现”写成
> “已通过样板一致性审查”。开放审查项以 GitHub issues 为准。
