# 审计报告：PR #507 Widget Manifest Trusted Plugin Soft Deps + WebUI Degrade Hint

- **审计来源 LLM**：GLM-5.2（纯文本 LLM，未执行视觉审查）
- **审计时间**：2026-08-07（reaudit after rebase）
- **审计对象**：PR #507（feat/widget-trusted-plugin-deps, head aa4b83e → rebased to 42c4d55 on top of #506 HEAD 34d1117）
- **审计依据**：AGENTS.md「Audit Agent Charter」三原则 + 项目记忆「WebUI 改动 PR 必须使用多模态模组审查」
- **审计方法**：独立读源码（extensions/mod.rs / compat.rs / plugin-deps.js / boot.js / widget-host.js / sandbox-bridge.js / widgets.css / tests）+ 独立跑 `cargo test -p airp-core --lib`（1418 passed, 0 failed, 5 ignored）+ `cargo clippy -- -D warnings`（clean）+ `node --test webui/tests/*.test.mjs`（167 passed, 0 failed）

## 改动概要

widget manifest 的 `trusted_plugins` 软依赖字段 + webui 非阻塞降级提示。

- **engine**：`WidgetManifest.trusted_plugins: Vec<TrustedPluginDependency>`；`validate_manifest` 校验（坏 id / 坏 min_host_api 拒绝整包）；3 个构造点同步；compat 往返 + 校验矩阵测试
- **webui**：`plugin-deps.js`（initFromEngine/missingDependencies/versionAtLeast）；`boot.js` 加 `fetchEnginePlugins()` + 5s 超时；`widget-host.js` render 非阻塞提示条；`sandbox-bridge.js` destroy 清理 ready 定时器；4 个 screen 脚本链；widgets.css 样式
- **tests**：`plugin-deps.test.mjs`（7 用例）；endpoint-guard 登记新 fetch 调用点；golden inventory 加 `GET /v1/plugins`

## 阻塞项（B-series）

### B1：基于旧 #506（1fbdd16），缺少 A4/B2/B3/B4 安全修复 — **已修复（rebase 完成）**

**原问题**：PR #507 基于 #506 的原始 commit 1fbdd16，该 commit 缺少：
- A4：`env_clear` + 白名单（daemon 凭据如 `AIRP_ACCESS_KEY` 会继承给插件子进程）
- B2：级联 kill（孙进程变孤儿，Windows 端口残留）
- B3：`kill_on_drop`（panic/SIGKILL 路径子进程变孤儿）
- B4：fail-closed loopback（无 ConnectInfo 时远程请求可直达插件）

**修复执行**（2026-08-07）：执行 `git rebase --onto 34d1117 1fbdd16 feat/widget-trusted-plugin-deps`，将 #507 三个 commit（feat / fix / docs-audit）replay 到 #506 当前 HEAD（34d1117，含 A4 + B2/B3/B4 全部修复）之上。

**冲突解决**（3 个文件，全部为 #507 的格式化改动 vs #506 的实质安全修复）：
- `engine/src/plugins/proxy.rs`：取 #506 版本（含 B4 fail-closed + bounded_response_body + stream_or_shutdown），丢弃 #507 的 `resp.bytes().await` 格式化改动
- `engine/src/plugins/mod.rs`：取 #506 版本（含 env_clear 文档），丢弃 #507 的 rustfmt 行宽改动
- `engine/src/daemon/mod.rs`：取 #506 版本，丢弃 #507 的 route 单行化改动
- `engine/src/daemon/tests/plugins.rs`：自动合并（#507 的格式化改动与 #506 的 ConnectInfo 注入不冲突）

**rebase 后核验**：
- `git show HEAD:engine/src/plugins/spawn.rs` 含 `env_clear` / `kill_on_drop` / `process_group` / `killpg` / `taskkill`（A4 + B2 + B3 全部继承）
- `git show HEAD:engine/src/plugins/proxy.rs` 含 `ConnectInfo` / `plugin_remote_forbidden` / `bounded_response_body` / `stream_or_shutdown` / `fail-closed`（B4 全部继承）
- 1418 lib tests 全绿（#506 的 1417 + #507 的 1 个新 compat 测试）
- 167 WebUI tests 全绿
- clippy clean

**合并顺序**：rebase 后 #507 直接基于 #506 HEAD（34d1117），#506 合并后 #507 可直接 rebase 到 main，无再次冲突预期（#507 与 main 的 diff 只含 extensions/* + webui/*，与 #506 的 plugins/* 改动不重叠）。

## 已核实（V-series，独立验证）

| 编号 | 核实项 | 证据 |
|------|--------|------|
| V1 | trusted_plugins id 校验 | `extensions/mod.rs:729-741`：empty/len>128/starts with `.`/ends with `.`/contains `/` or `\` → `invalid_manifest`。compat 测试 `../evil` → 拒绝 |
| V2 | min_host_api 校验 | `extensions/mod.rs:743-758`：空串拒绝（缺省语义用 omit）；非法 semver 拒绝；跨 major 只校验格式不钉 major（软依赖不是安装合同）。compat 测试 `1.x`/`""` → 拒绝，`9` → 通过 |
| V3 | versionAtLeast 逐段比较 | `plugin-deps.js:41-54`：缺段视为 0；非数字 → false（fail-closed）。测试覆盖 `1` vs `1`/`1.2` vs `1`/`2` vs `1.9`/`1` vs `2`/`x` vs `1`/空串/null |
| V4 | missingDependencies 四态 | `plugin-deps.js:62-78`：not-installed / stopped / version-too-low / satisfied。测试覆盖全部四态 + 无声明 + 空缓存 fail-closed |
| V5 | XSS 防护 | `widget-host.js:42-47`：`el()` 用 `textContent`（非 `innerHTML`）。`m.id` 来自 manifest 校验后的数据，即使含 HTML 也被渲染为纯文本 |
| V6 | boot 超时防护 | `boot.js:149`：`AbortSignal.timeout(5000)`——engine 挂起时 boot 不悬挂（fail-closed：超时后保持空缓存 → 全部声明按缺失提示） |
| V7 | sandbox-bridge 定时器清理 | `sandbox-bridge.js:116-118`：`destroy()` 清理 `readyWaiters` 的 `clearTimeout(w.timer)`——iframe 未 ready 就卸载时不再泄漏 5s 定时器 |
| V8 | 非阻塞降级 | `widget-host.js:194-210`：提示条在 widget sandbox 之前插入，widget 仍加载（`widget-sandbox` div 仍 appendChild）。测试验证 `container.children.some(c => c.className === 'widget-sandbox')` |
| V9 | engine 不可达 fail-closed | `boot.js`：fetch 失败 → 保持空缓存 → `missingDependencies` 全部返回 `not-installed`。测试覆盖 `initFromEngine({ plugins: [] })` → 全部 missing |
| V10 | additive-only 兼容 | `trusted_plugins` 用 `#[serde(default, skip_serializing_if = "Vec::is_empty")]`；旧 manifest 反序列化为空 Vec；compat 测试 `compat_host_api_roundtrip` 验证旧记录缺 `trusted_plugins` → 空 Vec |
| V11 | 1418 lib tests 全绿（rebase 后） | `cargo test -p airp-core --lib`：1418 passed, 0 failed, 5 ignored（#506 的 1417 + #507 的 1 个新 compat 测试） |
| V12 | 167 WebUI tests 全绿 | `node --test webui/tests/*.test.mjs`：167 passed, 0 failed |
| V13 | clippy clean | `cargo clippy -p airp-core --all-targets -- -D warnings`：无警告 |
| V14 | endpoint-guard 登记 | `endpoint-guard.test.mjs`：`widgets/boot.js|engineUrl('/v1/plugins')` 已登记，`v1-endpoints.json` golden inventory 已加 `GET /v1/plugins`（ui:true） |

## 非阻塞项（N-series，后续迭代）

| 编号 | 项 | 说明 |
|------|----|------|
| N1 | ~~rebase 后需重跑全部测试~~ 已完成 | rebase 后 1418 tests 全绿（含 B2/B3/B4 新增测试 + #507 compat 测试），原 N1 关闭 |
| ~~N2~~ | ~~`versionAtLeast` 用 `Number()` 而非 `parseInt`~~ **已修复** | 改为 `/^\d+$/` 严格校验每段 + `parseInt(seg, 10)`。拒绝 hex（`'0x10'`）、科学计数法（`'1e2'`）、带符号（`'+1'`）、前导空格（`' 1'`）。测试覆盖全部新边界。 |
| N3 | 提示条文案无 i18n | `widget-plugin-hint-title` 硬编码中文「依赖的 trusted plugin 不可用」。与既有 widget-host 文案一致（均为中文），但后续 i18n 时需统一。 |
| N4 | `min_host_api` 不钉 major 的文档化 | engine 只校验格式不钉 major（软依赖不是安装合同），但 `parse_host_api_major` 函数名暗示会钉 major。建议在 `TrustedPluginDependency` 的 doc comment 中明确说明「只校验格式，不钉 major」。 |

## 未执行视觉审查

**本 PR 含 WebUI 视觉改动**（`widgets.css` 新增 `.widget-plugin-hint` 样式 + `widget-host.js` render 提示条 + 4 个 screen HTML 加 script 链）。

按 AGENTS.md 规则（2026-07-26 用户立，issue #319）：WebUI 改动 PR 必须使用 KIMI K3 或性能超过 KIMI K3 的多模态模组对视觉效果进行审查。本审计 agent 为纯文本 LLM（GLM-5.2），**未执行视觉审查**。

**待补审查项**（需多模态 agent 截图审查）：
- `.widget-plugin-hint` 视觉一致性（token/间距/字号/颜色是否符合设计系统）
- 提示条布局正确性（在 widget 上方、不遮挡、flex 布局）
- 可访问性（对比度/焦点/键盘）
- 交互完整性（提示条不影响 widget 加载与交互）
- 与设计 baseline 的偏离

## 结论：**CONDITIONAL APPROVE**（条件：rebase 已完成；剩视觉审查）

代码层面无阻塞缺陷。B1（rebase）已修复并独立验证：1418 lib tests + 167 WebUI tests 全绿，clippy clean，#506 的 A4/B2/B3/B4 全部安全修复已继承。

**合并前置条件**：
1. ✅ PR #506 先合并（含 B2/B3/B4/A4 修复）— 待人工 review
2. ✅ PR #507 rebase 到 #506 HEAD（34d1117），重跑全部测试 — **已完成**
3. ⏳ 多模态 agent 补审视觉改动（含截图证据）— **待执行**

满足以上条件后方可合并。
