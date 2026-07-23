# PR #301 独立复审报告（补充）

**PR**: feat: Linux WebUI CI + issue #304 样板/webui 同步（合并审计）
**复审日期**: 2026-07-24
**复审性质**: 对 `2026-07-23-PR-301-linux-webui-console-sync-audit.md` 的**独立复审**
**依据**: 逐行读取 `webui/assets/console-runtime.js`、`webui/screens/*`、`airp-engine-console/screens/*`、两套 CSS、`README.md`，独立运行 `runtime-pages.test.mjs`。不以前审结论为不可质疑前提（审计章程 §11.1）。

---

## 复审裁决：可合并（功能正确、测试全绿）；发现 1 个 MEDIUM + 3 个 LOW 缺口，纳入统一审计修复批次

---

## 一、前审 H/M/L 修复复核（读源码确认已真实落地）

| 前审项 | 复核位置 | 结论 |
|---|---|---|
| H-1 webui 身份 | `webui/STYLEGUIDE.md` L1-3「派生 WebUI 实现」 | ✅ 已落地 |
| H-2 虚假"已拉取" | `console-runtime.js` L372 `status-pill neutral` + "保存后自动拉取" | ✅ 已落地 |
| M-1 worldbook 操作列 | `console-runtime.js` L228 `.switch on` / `.switch` | ✅ 已落地，死三元已移除 |
| M-2 按钮语义 | `console-runtime.js` L177「保存 JSON」 | ✅ 已落地 |
| L-4 NL 生成按钮 | `console-runtime.js` L191 / L241 `nlGenBtn.disabled = true` | ✅ 已落地 |
| 33 重定向链路 | `webui/screens/16-onboarding.html` 存在；`screen-redirect.js` 无 inline handler（CSP 干净） | ✅ 已落地 |
| 新 CSS 类 | webui `components.css`/`console.css` 含 nl-zone/json-advanced/op-col/model-picker/combobox 等；`t-mono` 在 `base.css`（运行时已加载） | ✅ 齐全 |
| 无旧文件名残留 | `grep presets-models` webui/airp-engine-console 均空 | ✅ |
| 测试 | `runtime-pages.test.mjs` 11/11 pass | ✅ |

---

## 二、复审新发现（前审未覆盖）

### NEW-1 · [MEDIUM] webui 05 仍渲染"Provider 模型"卡，与 rename 意图及样板冲突

**位置**: `webui/assets/console-runtime.js` `renderPresets` L262–287

**事实**:
- #304 把 05 导航标签从「预设与模型」改为「预设」（`pages` 数组 L25 + 文件 `05-presets-models.html → 05-presets.html`），意图对齐样板 05 改名、并将模型职责移入 08。
- 但 `renderPresets` 内容**未改**，仍渲染完整「Provider 模型」管理卡（拉取 `/v1/models` + 保存模型）。
- 该模型卡源自 `e01ca6f Rebuild WebUI from console sample`（预存在），#304 仅改了标签未改实现 —— 典型"部分同步"遗漏。
- 样板 `05-presets.html` 现已是「预设 / 采样参数」（temperature/top_p/版本快照），**不含模型**；模型在样板 08。

**冲突点**:
1. 导航标签「预设」与屏幕实际内容（含模型管理）不符。
2. 与样板 05 内容不一致（样板无模型）。
3. 模型 UI 在 webui 05 与 08 重复出现，违背 #304 "模型移入 08" 意图。

**影响**: 非运行时错误，但 UX 误导 + 样板/派生视觉契约不符。
**建议**: 二选一 —— (a) webui 05 移除模型卡，对齐样板与 rename 意图；(b) 保留模型卡则恢复 nav 标签「预设与模型」。
**处置**: 不单独阻塞本 PR（功能正确），纳入"统一审计修复"批次。

### NEW-2 · [LOW] webui 08 降级行未使用样板 `.combobox` 样式

**位置**: `console-runtime.js` `renderSettings` L375–378

**事实**: 样板 `08-settings.html` L70 用 `.combobox`（带样式 div）+ `.combobox-error`（错误码展示）；webui 降级行用 `input()` 普通输入框。`.combobox`/`.combobox-error` CSS 已同步进 webui `components.css` 但**未被运行时使用**。
**影响**: 视觉不对齐；拉取失败错误码不展示。
**建议**: 降级行渲染为 `.combobox` 元素 + 可选 `.combobox-error`。

### NEW-3 · [LOW] 缺 #304 新运行时行为的自动化测试

**事实**: `runtime-pages.test.mjs` 仅验证文件存在 + CSP + 共享运行时加载。NL 区 disabled 按钮、JSON 折叠 toggle、worldbook `.switch`、model pill neutral 等新行为**无断言**。
**影响**: 合并后若这些 UI 回归，无测试拦截。
**建议**: 加轻量 DOM 断言（jsdom/headless）验证 `renderWorkbench`/`renderWorldbook`/`renderSettings` 产出含 `.nl-zone`、`.json-advanced`、`.switch`、`.status-pill.neutral`。

### NEW-4 · [OBSERVATION] webui 缺失"采样参数"UI 表面

**事实**: 样板 05 展示 temperature/top_p/版本快照等采样参数；webui 无对应表面。历史 parity 缺口，#304 未覆盖。
**建议**: 记入后续 parity 跟踪，非本 PR 范围。

### NEW-5 · [LOW/可选] README 溯源表缺"webui 版本"列

**事实**: `airp-engine-console/README.md` 已加「v0.0.2 release 的 webui 版本是基线」定义段；但溯源表三行未标版本（均为基线后 #304 开发态）。用户最初诉求为"webui 版本以及它的 commit"。
**建议**: 溯源表补「webui 版本」列，三行标 `v0.0.2+`。

---

## 三、安全与供应链

前审已覆盖且本次未发现新增风险：Actions 全 SHA pin、`permissions: contents: read`、`persist-credentials: false`、`127.0.0.1` 绑定、`--locked`、密钥不回显、`start-airp.sh` chmod 0700 data dir（CodeRabbit 修复）均到位。

---

## 四、结论

PR #301 功能正确、测试全绿、安全无新增风险，前审 H/M/L 修复经源码复核全部真实落地。
唯一实质缺口为 **NEW-1（webui 05 模型卡与 rename/样板冲突）**，属 #304 部分同步遗漏；按用户"等待后续统一审计修复"的安排，建议将其与 NEW-2~NEW-5 一并纳入统一修复批次，不必单独阻塞本 PR 合并。
