# PR #82 独立审计报告

**审计源 LLM**：GLM-5.2
**审计日期**：2026-07-07
**审计范围**：审计遗留小修复 batch 1（commit 39e194a）
**审计依据**：AGENTS.md 三条守则

## 改动清单

3 文件 +22/-11 行，纯文档/注释/1 行 `immediate:true` 移除，无逻辑改动。

| 文件 | 改动 |
|---|---|
| `ui/src-tauri/src/bus.rs` | module doc 更新 + saving emit 加注释 |
| `ui/src/App.vue` | 递归 dispatch 注释改写 |
| `ui/src/widgets/SettingsModal.vue` | immediate:true 移除 + watch 加交互注释 |

## 审计

### #79 W-04：bus.rs module doc 更新 ✅

新增 M4 scope 段落准确列出 5 个已接 intent（characters.list/import、settings.get/update、chat.history）。"Other intents fall back to a minimal ack" 仍保留，对未接 intent（如 chat.regen/rollback、agent.run）仍准确。

### #79 W-01：SettingsModal immediate:true 移除 ✅

**行为等价性验证**：
- 改前：mount 时 immediate 触发回调，v=false 不进 if，不发包
- 改后：mount 时不触发回调
- 改后第一次 visible 翻 true 时触发 refresh

行为相同（mount 时都不发包），移除 `immediate:true` 仅消除无意义回调 + 误导性。✅

### #79 W-05：saving=true 交互注释 ✅

bus.rs saving emit 处加注释说明"set 替换 + 非空同步避免回填清空"交互。SettingsModal watch 处加交叉引用注释。注释准确描述了实际行为。

### #81 W-04：递归 dispatch 注释 ✅

原注释"不递归"误导（实际是递归调用 onIntent，只是 name 不同不无限递归）。新注释"chat.history 走正常 dispatch 路径，不会回到 characters.select 分支"准确描述了为什么不无限递归。

## A 项（阻塞）

**无。**

## W 项

**无。** 纯文档/注释改动，无逻辑风险。

## 测试

- Tauri cargo check: pass
- UI vitest: 97/97 pass
- vue-tsc --noEmit: 0 errors

## 结论

**推荐合并。** 纯文档/注释/1 行无行为变化的改动，零风险。
