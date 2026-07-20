# PR #258 独立审计报告 — SmoothStreamer.finish() 单次化（#252 D4）

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#258 webui(app.js): unify SmoothStreamer.finish() to single post-streamSse call (#252 D4)](https://github.com/GhostXia/AIRP/pull/258)
- **分支**：`codex/webui-252-d4-smooth-streamer-finish`
- **base**：`main`（mergeStateStatus: CLEAN，mergeable: MERGEABLE）
- **commits**：`5362c05`（单 commit，net +11 / -25）
- **CI**：Rust workspace SUCCESS / UI and WebUI SUCCESS / Production topology SUCCESS / Portable Windows WebUI SUCCESS / CodeRabbit SUCCESS

## 1. 范围与背景

PR #251 引入 Swipe + Smooth Streaming 时，在 `doContinue` 与 `doRegen` 中使用了
"done 分支 + 防御 if 块"的双路径 `streamer.finish()` 模式。PR #251 审计（见
`docs/audits/2026-07-20-PR-251-swipe-smooth-streaming-audit.md` D4）将此列为
低优先级可读性遗留项：`doSend` 已经使用单次 `finish()` 模式，`doContinue`/`doRegen`
应统一。

本 PR 把 `doContinue`（约 L1429-L1451）和 `doRegen`（约 L1879-L1900）两处改为
`streamSse` 返回后单次 `finish + classList.remove('streaming') + innerHTML = renderMarkdown(...)`，
与 `doSend`（L1522-L1526）模式一致。

## 2. 独立证据

### 2.1 改前 / 改后对照（基于 `git diff main..HEAD -- webui/app.js`）

`doContinue`（main → PR）：

```diff
       await streamSse(res, (chunk, seq) => {
         if (chunk.type === 'body_chunk' && chunk.text) {
           acc += chunk.text;
           streamer.push(chunk.text);
           if (seq % 5 === 0) chatLog.scrollTop = chatLog.scrollHeight;
         }
-        if (chunk.type === 'done') {
-          streamer.finish();
-          acc = streamer.getRendered();
-          textNode.classList.remove('streaming');
-          textNode.innerHTML = renderMarkdown(existingText + acc);
-        }
       });
-      if (textNode.classList.contains('streaming')) {
-        streamer.finish();
-        acc = streamer.getRendered();
-        textNode.classList.remove('streaming');
-        textNode.innerHTML = renderMarkdown(existingText + acc);
-      }
+      streamer.finish();
+      acc = streamer.getRendered();
+      textNode.classList.remove('streaming');
+      textNode.innerHTML = renderMarkdown(existingText + acc);
```

`doRegen` 同型，省略。

`doSend`（参考点，main 与 PR 一致，L1522-L1526）：

```js
const seq = await streamSse(res, (chunk, seq) => {
  if (chunk.type === 'body_chunk' && chunk.text) { ... }
});
streamer.finish();
acc = streamer.getRendered();
msgEl.classList.remove('streaming');
if (acc) msgEl.innerHTML = renderMarkdown(acc);
```

### 2.2 `streamSse` 控制流（`webui/app.js` L1703-L1761）

静态读 `streamSse`：

1. `while (!sawDone)` 主循环：`reader.read()` 成功 → 解析行 → `[DONE]` 设 `sawDone=true` 并 break → 否则 `onChunk(chunk, seq)`；
2. `reader.read()` 抛 `AbortError` → 透传抛出；
3. `reader.read()` 抛其他 → 转 `kind: 'stream_interrupt'` 抛出；
4. `event: error` chunk → 转 `kind: 'stream_error'` 抛出；
5. `reader.read()` 返回 `done:true`（流自然结束） → `break` 跳出循环；
6. 任何路径下 return `seq`。

因此 `streamSse` 调用方只可能进入两类状态：
- **A. 正常返回**：`sawDone=true` 或 `done:true` 自然结束；
- **B. 抛异常**：`AbortError` / `stream_interrupt` / `stream_error`。

### 2.3 行为等价性矩阵（独立 trace）

| 流结果 | 改前 doContinue/doRegen | 改后 doContinue/doRegen | 等价？ |
|---|---|---|---|
| 正常 `[DONE]` chunk | done 分支 finish+render，classList 已 remove，防御 if 跳过 | streamSse 返回后单次 finish+render | 等价（延迟仅几 ms，无可见差异） |
| 流自然结束（`reader.read` 返回 `done:true`，无 `[DONE]`） | done 分支不触发，防御 if 触发 finish+render | streamSse 返回后单次 finish+render | 等价 |
| `reader.read` 抛 `AbortError` | streamSse 抛 → catch block（不调用 finish+render） | 同左，try 块外的 finish+render 不执行 | 等价 |
| `reader.read` 抛非 Abort | streamSse 转 `stream_interrupt` 抛 → catch block | 同左 | 等价 |
| `event: error` chunk | streamSse 转 `stream_error` 抛 → catch block | 同左 | 等价 |

`doSend` 的 catch block 在 `AbortError` 上 `return`，但 doContinue/doRegen 的
catch block 在所有异常上都 `return` 或 fall-through 到 finally（无显式 return
但不再执行 try 块外代码）。PR 改后的 try 块外 `finish+render` 只在 `streamSse`
正常返回时执行，与改前一致。

### 2.4 catch block 不变性

PR 未改 `doContinue` / `doRegen` 的 catch block：

```js
// doContinue
} catch (e) {
  if (e && e.name === 'AbortError') return;
  if (textNode) { textNode.classList.remove('streaming'); textNode.innerHTML = renderMarkdown(existingText + acc); }
}

// doRegen
} catch (e) {
  if (e && e.name === 'AbortError') return;
  if (msgEl) { msgEl.classList.remove('streaming'); msgEl.innerHTML = renderMarkdown(acc || '[regen interrupted]'); }
}
```

异常路径下 `acc` 是已 push 但未 `getRendered` 的部分字符串，与改前一致；
`doSend` 的 catch block 也未调用 `finish()`，PR 描述里明确这是既有行为（"text
pushed to the queue but not yet rendered by rAF is dropped"）。

### 2.5 doSend 与 doContinue/doRegen 的剩余差异（PR 未触及）

- doSend 用 `if (acc)` 守卫 `innerHTML = renderMarkdown(acc)`（空 acc 保留
  `appendMsg` 初始空 textContent），doContinue/doRegen 不用守卫。doContinue 的
  `existingText` 至少含上次 assistant 消息 markdown 源，永远非空；doRegen 的
  `renderMarkdown('')` 返回空串，与原防御 if 行为一致（防御 if 也调用
  `renderMarkdown(acc)`，空 acc 同样得空 innerHTML）。这是 #251 既有差异，本 PR
  不动。
- doSend 不写 `existingText + acc`（fresh 消息无 prefix），doContinue 用
  `existingText + acc`（continue 模式 prefix）。这是 #249 Smooth Streaming B2
  修复有意为之，本 PR 不动。

### 2.6 测试基线

PR 描述：`cd webui; node --test` → 98 pass / 0 fail，与基线一致。CI `UI and
WebUI` SUCCESS（含 webui 测试）。

### 2.7 `SmoothStreamer` 单元测试覆盖

PR 描述明确 `SmoothStreamer` 缺乏 dedicated webui unit tests（gap tracked as
#252 §2.H.2）。本 PR 不补测试。这一缺口在 #252 已立案，本审计不要求本 PR 补，
但需明确：本 PR 的行为等价性证据全部来自静态代码 trace，**没有 automated
test 直接覆盖 finish() 单次化路径**。这是可接受的范围（PR 描述已声明 Scope
discipline）。

## 3. 独立意见（按 §Audit Agent Charter 第 2 条）

### 3.1 关于"防御 if 块"的删除

main 上的防御 if `if (textNode.classList.contains('streaming'))` 在 done 分支
已 remove classList 后会跳过，唯一作用是兜底"流自然结束（无 done chunk）"路径。
PR 改后该路径由 try 块外的无条件 `finish+render` 覆盖。删除防御 if **不减弱**
任何兜底语义，反而消除了"两个 finish 路径竞态"的可读性陷阱。本审计同意删除。

### 3.2 关于 doSend `if (acc)` 守卫

doSend 的 `if (acc)` 守卫在 main 上等价于"空响应不重写 innerHTML"。本 PR 不
动 doSend，doContinue/doRegen 也没有引入守卫。若 doRegen 在 engine 返回空响应
（如 server 立即 close 且无 body_chunk），`renderMarkdown('')` 写空 innerHTML
是合法行为，不破坏 DOM。本审计同意不引入守卫。

### 3.3 不要求补充 smooth streamer 单元测试

理由：
- 本 PR 是纯重构，行为等价性可由静态 trace 完整证明（见 §2.3）；
- `SmoothStreamer` 单元测试缺失是 #251 既有缺口，已在 #252 §2.H.2 立案；
- 强行在本 PR 补测试会违反 #252 Scope discipline，且测试设计本身需要独立评审。

## 4. 风险评估

| 风险 | 评级 | 说明 |
|---|---|---|
| 行为回归（done 分支延迟 finish） | 极低 | 延迟仅事件循环 tick 级，无可见差异 |
| 行为回归（无 done chunk 路径） | 极低 | 改前防御 if = 改后无条件 finish+render，等价 |
| 行为回归（异常路径） | 零 | catch block 未改，try 块外 finish+render 在异常下不执行 |
| 测试缺口 | 低 | `SmoothStreamer` 无 dedicated unit tests，但本 PR 不引入新行为 |
| CI flaky | 零 | CI 5/5 SUCCESS |

## 5. 阻塞项

无。所有 CI 通过，行为等价性证明完整，scope 严格限定 D4。

## 6. 非阻塞 / 后续可追踪项

| 编号 | 内容 | 建议 |
|---|---|---|
| 258-A1（非阻塞） | `SmoothStreamer` 缺 dedicated unit tests | 跟进 #252 §2.H.2，独立 PR |
| 258-A2（非阻塞） | `doSend` 与 `doContinue`/`doRegen` 在 `if (acc)` 守卫上有差异 | 跟进 #252，确认是否应统一 |

## 7. 审计结论

**通过（PASS，无阻塞项）**。

PR #258 是 PR #251 审计 D4 遗留项的纯粹 readability 重构，行为等价性证明完整，
CI 全绿，scope 严格。可合并。

## 8. Refs

- PR #251 审计报告：`docs/audits/2026-07-20-PR-251-swipe-smooth-streaming-audit.md`
- Issue #252（D4 来源）
- 根 `AGENTS.md` §Audit Agent Charter
