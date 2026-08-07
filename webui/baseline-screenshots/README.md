# WebUI Golden 基线截图库

本目录存放 webui 44 屏的 golden 基线截图（JPEG q85，1440×900 视口 fullPage），
供后续像素对比（第二阶段引入 pixelmatch/pngjs，见 `ui/smoke-desktop-screenshots.ps1`
的 TODO 段）与人工视觉回归参照。

## 基线来源

- 由 `ui/webui-screenshot-suite.mjs --mode local` 对 `webui/screens/` 全部 44 屏
  逐屏截图（PNG），产物经 `.tmp/convert-baseline.ps1` 转 JPEG q85 落盘到这里。
- 只收录 44 屏静态基线；`<out>/flow/` 下的关键流步骤帧（onboarding 逐步、
  聊天三帧、导入/备份屏）不进基线库——它们依赖运行时状态（会话、provider），
  不适合做像素级 golden。
- 当前基线生成时本地 `provider_configured=false`，屏内容均为无 provider 状态；
  令牌断言（47 个设计令牌 × 每屏）与 CSP/pageerror 门禁在生成该基线的套件运行
  中全部通过。

## 更新方式

1. 重跑套件取新截图：
   `node ui/webui-screenshot-suite.mjs --mode local --out .tmp/baseline-screens`
2. 重新转换：
   `pwsh -NoProfile -File .tmp/convert-baseline.ps1`
   （脚本读 `.tmp/baseline-screens/*.png`，写本目录 `*.jpg`）

## 何时允许更新基线

仅当以下之一成立时更新，并随 PR 说明变更原因：

- webui 视觉层（tokens.css / 全局样式 / 屏布局）发生有意变更；
- 新增或删除屏（屏数变化，注意同步 `webui/tests/runtime-pages.test.mjs` 的屏数断言）；
- 基线生成环境的字体/渲染差异被修复（如 CI 补装 fonts-noto-cjk 后重生成）。

不得在「截图套件出现非预期差异」时直接重灌基线——先排查差异根因，
确认是有意变更后才更新。
