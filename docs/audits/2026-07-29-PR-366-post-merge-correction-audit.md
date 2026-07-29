# 已归档：PR #366 合并后纠正审计

> 本文件只记录 PR [#366](https://github.com/GhostXia/AIRP/pull/366) 在误合并时的历史事实与纠正依据。
> 后续实现由 PR [#367](https://github.com/GhostXia/AIRP/pull/367) 承接；当前有效能力与边界以
> [`CURRENT-BASELINE.md`](../CURRENT-BASELINE.md) 和
> [`CONVERSATION-CONTRACT.md`](../CONVERSATION-CONTRACT.md) 为准。

- 日期：2026-07-29
- 原 PR：#366 `engine: bound Conversation long-history context`
- 被审计提交：`a968b26`
- 误合并提交：`a121252`
- 审计性质：合并后独立复核；不把原开发结论、自动 resolved 或 CI success 当作审计证据
- 历史裁决：**PR #366 的审计门禁未完成；该结论促成 #367 的纠正实现与重新审计**

## 1. 时序事实

CodeRabbit 首轮审计只覆盖 `e82b3b5..1c41a0b`，提出 2 条正式 inline 意见与
6 条 nitpick。`a968b26` 对这些意见进行了代码修改，但增量复审因 rate limit 未实际运行。
两个正式线程随后由 bot 自动标记 `Addressed in a968b26`/resolved，status context 也显示
SUCCESS。开发流程错误地把这些自动状态解释为“修复提交已完整复审通过”，并合并了 #366。

自动识别提交与实际重新审阅不是同一件事。该合并违反了仓库“审计 bot pending、失败或仍有
阻塞意见时不得合并”的实质要求。#362 中原先的“无未修审计遗留项”记录已被纠正。

## 2. 原始八项意见核验

对 `main@a121252` 的源码逐项复核后，原始 2 条正式意见和 6 条 nitpick 均能找到对应实现：

1. current-turn budget trimming 改为候选副本，错误时不再部分淘汰原 history；
2. summary target budget 与 content-over-budget 使用不同诊断；
3. 增加 checkpoint checksum 篡改重建测试；
4. 静态 warm path 不再逐次计算完整 journal SHA-256；
5. 移除无输出作用的 participant set；
6. checkpoint summary payload 只解析一次；
7. committed summary 验证失败映射为内部完整性错误；
8. 引用事件读取复用同一 `BufReader<File>`。

这只能证明修改存在，不能补造 CodeRabbit 对 `a968b26` 的复审通过记录。

## 3. 独立审计历史阻塞项

### 历史发现 A1｜P1｜真实 turn 仍是全 journal 扫描

`load_verified_checkpoint` 在 `events.jsonl` 长度变化时直接丢弃 checkpoint 并调用
`rebuild_checkpoint`。Conversation turn 在 projection 之间必然追加 lifecycle/message
事件，因此“静态 journal 的 50 次 warm projection”没有覆盖真实工作流。50k 历史下的
2.334 ms 数据不能证明回合间 projection 的复杂度；实际仍为 `O(journal events)`。

纠正方向：

- checkpoint schema 升级；
- 已验证旧尾之后只有普通追加事件时，仅扫描新增 suffix 并更新最近消息索引；
- suffix 出现 summary 时仍全量重建，保留完整前缀 ID/SHA-256 验证；
- benchmark 每次 projection 前真实 append，禁止再以静态 warm soak 代替回合路径。

本地纠正证据：

- 定向 `conversation_context` 测试：7 passed / 1 ignored；
- 50,000 events release：cold 117 ms；
- 50 次 append-aware projection：266 ms total / 5.329 ms mean；
- retained messages：128。

## 4. 历史关闭条件（由 PR #367 承接）

1. 纠正实现、合同和本审计记录进入独立 PR；
2. 完整 workspace、神圣 prompt-boundary、UI/WebUI、rustdoc/clippy 与生产/Windows 门禁通过；
3. CodeRabbit 对纠正 PR 的实际 head 完成审计；rate-limited、自动 resolved 或仅识别提交均不算通过；
4. 修复全部新阻塞意见后由人工 review 合并；
5. 合并后再更新 #362 的纠正状态与最终证据。
