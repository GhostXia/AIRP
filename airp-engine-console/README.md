# AIRP Engine Console · 权威样板

本目录是 AIRP 控制台前端的**权威视觉样板**（golden sample），与 Ardot 设计稿
「AIRP Engine Console」逐屏对应。所有派生实现（WebUI、桌面端）须基于本样板开发，
视觉冲突以本样板为准。

详细规范见 [STYLEGUIDE.md](STYLEGUIDE.md)。

## 溯源与回退

样板与派生 WebUI 同处一个仓库，共享 commit 历史。每次同步后在此记录对应的
WebUI commit，便于回退与溯源：

| 样板同步批次 | 样板 commit | WebUI commit | 变更摘要 |
|---|---|---|---|
| #304 编辑体验 | `fbcab6e` | `fbcab6e` | NL区 / JSON高级折叠区 / diff视图 / 行内操作列 / model picker / 33屏向导 / 05重命名 |
| #304 item 1 | `3caafa1` | `3caafa1` | model picker for onboarding step3 + console provider card |
| 基线 | `bc72c48` | `bc72c48` | 305 fix: character_id query string |

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

## 快速开始

直接在浏览器中打开 `index.html` 即可浏览全部 33 屏样板。
每屏 HTML 也可独立打开（`screens/NN-*.html`），无需服务器。
