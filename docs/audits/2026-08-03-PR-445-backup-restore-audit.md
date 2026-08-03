# PR #445 独立审计：备份恢复闭环（E-P2-1，closes #342）

> 审计日期：2026-08-03
> 审计对象：PR #445 `feat(engine): backup/restore closed loop (E-P2-1, closes #342)`
> 审计分支：`feat/e-p2-1-backup-restore`（commit e9e6615）
> 审计基线：`main@098ba76`（PR #441 合并后）
> 审计依据：AGENTS.md「审计 Agent 守则」三原则——独立审计、可提己见、可质疑历史并查证
> 关联合同：`docs/CURRENT-BASELINE.md`、`docs/LOCK-ORDER-CONTRACT.md`、`docs/BACKUP-RESTORE.md`、`docs/plans/2026-08-03-e-p2-1-backup-restore.md`

## 1. 审计范围

本审计独立复核 PR #445 的全部内容：

1. **backup 模块**：`engine/src/backup/{mod.rs, manifest.rs, snapshot.rs}` — manifest schema、snapshot 创建/校验/恢复/删除
2. **HTTP 端点**：`engine/src/daemon/handlers/backups.rs` — 5 个端点（create/list/get/verify/restore/delete）
3. **pre-delete 集成**：`engine/src/domain/chat.rs`（`delete_character`/`delete_session`）、`engine/src/agent/tools/character.rs`（agent tool）、`engine/src/daemon/handlers/{characters.rs, sessions.rs}`（HTTP endpoint + `?force=true`）
4. **WebUI**：`webui/assets/console-runtime.js`（`renderBackup`）、`webui/tests/runtime-pages.test.mjs`
5. **文档**：`docs/BACKUP-RESTORE.md`、`docs/plans/2026-08-03-e-p2-1-backup-restore.md`

审计方法：读源码 + 读合同 + 独立运行测试 + 与 `main@098ba76` 对照。**不**把开发 agent 的结论、计划文档或 BACKUP-RESTORE.md 当作不可质疑的前提。

## 2. 独立发现

### 2.1 BLOCKING B-01：BACKUP-RESTORE.md 与实现严重矛盾——文档声称 scoped restore 不支持，但实现已交付且测试通过

**发现**：

`docs/BACKUP-RESTORE.md` 在 4 处明确声称 v1 不支持 scoped restore（Character / Session scope），并指导用户手动拷贝文件：

| 位置 | 原文 |
|---|---|
| §4.3 step 2 | "**不能直接 restore**：v1 仅支持 `Full` scope restore，scoped restore 会被 fail-closed 拒绝（防止当前 restore 流程删除 data_root 下未备份的不相关数据）" |
| §4.3 step 3 | "手动从 backup 目录的 `files/` 子树拷贝所需文件回 data_root" |
| §5 v1 限制表 | "仅支持 `Full` scope restore \| `Character` / `Session` scope backup 不能直接 restore" |
| §6.5 fail-closed | "scoped restore → 拒绝（v1）" |
| §7 HTTP API | "`POST /v1/backups/:id/restore` \| 恢复（v1 仅 Full）" |

**但实现实际支持 scoped restore**，证据如下：

1. `engine/src/backup/snapshot.rs:532-540` — `restore_backup` 根据 `subtree_prefix.is_empty()` 分流到 `swap_full_data_root`（Full）或 `swap_scoped_subtree`（Character/Session）：
   ```rust
   if subtree_prefix.is_empty() {
       swap_full_data_root(data_root, &staging_dir, &rollback_id)?;
   } else {
       swap_scoped_subtree(data_root, &staging_dir, &subtree_prefix, &rollback_id)?;
   }
   ```

2. `engine/src/backup/snapshot.rs:613-694` — `swap_scoped_subtree` 完整实现了 scoped restore：校验 subtree_prefix、resolve 目标路径、trash 旧子树、rename staging 子树到目标、清理空祖先目录。

3. `engine/src/backup/snapshot.rs:1411-1452` — 测试 `restore_scoped_backup_preserves_unrelated_data` 验证：删除 alice 后通过 scoped backup 恢复 alice，bob 的数据完全不受影响。**该测试通过**（本次审计独立运行确认）。

4. `engine/src/domain/mod.rs:957-981` — 测试 `delete_character_pre_delete_backup_can_be_restored` 验证：`delete_character(force=false)` 创建的 Character-scoped PreDelete backup 可以通过 `restore_backup` 恢复。**该测试通过**。

5. HTTP handler `restore_backup_endpoint`（`backups.rs:367-385`）**不**对 scope 做任何限制——任何 scope 的 backup 都可以 restore。

**影响**：

文档误导用户采取**更不安全**的恢复路径。用户按 §4.3 手动拷贝文件时：
- 绕过完整性校验（`verify_against_disk`）
- 不创建回滚备份（`PreRestoreRollback`）
- 不做 post-restore 校验
- 不 `sync_dir` 持久化

而实际 API `POST /v1/backups/:id/restore` 对 scoped backup 完全可用，且提供上述全部安全保障。

**违反的不变式**：

- AGENTS.md §3 不变式 #4「用户资产优先」：文档引导用户走不安全路径
- AGENTS.md §3 不变式 #5「安全默认关闭」：文档声称安全功能不存在
- CURRENT-BASELINE.md §2.1「不能把页面数、工具数或 Phase 合入数量替代黄金路径成功率、恢复能力」：恢复能力已实现但文档未如实反映
- 计划 §6 验收清单「`docs/BACKUP-RESTORE.md` 说明 secret 排除、版本兼容、灾难恢复路径」：灾难恢复路径错误

**严重度**：BLOCKING

**修复建议**：

更新 `docs/BACKUP-RESTORE.md`：
1. §4.3：将"不能直接 restore"改为"可以直接 restore"，删除手动拷贝步骤
2. §5 v1 限制表：删除"仅支持 Full scope restore"行
3. §6.5：删除"scoped restore → 拒绝（v1）"
4. §7 HTTP API：将"恢复（v1 仅 Full）"改为"恢复（Full / Character / Session scope）"
5. 补充 scoped restore 的行为说明：仅替换 `subtree_prefix` 子树，其他 data_root 内容不受影响

---

### 2.2 W-01：LOCK-ORDER-CONTRACT.md 未更新 BACKUP_LOCK

**发现**：

PR #445 引入新全局锁 `BACKUP_LOCK: std::sync::Mutex<()>`（`engine/src/backup/snapshot.rs:62`）。代码有详细 doc comment 说明锁序语义：

```rust
/// LOCK-ORDER: 全局叶锁（与 `revision::atomic::COMMIT_LOCK` 同层级）。持此锁时不得
/// 获取任何 per-character / per-session / per-state 资源锁。调用方在 character_lock
/// 内调用 backup 时合法（外→内序列）。
```

但 `docs/LOCK-ORDER-CONTRACT.md` 未更新：
- §1.5 全局 utility 锁清单未包含 `BACKUP_LOCK`
- R4 叶锁规则未列举 `BACKUP_LOCK`
- §2 已核验嵌套路径未记录 `delete_character` 的 `character.write() → BACKUP_LOCK` 路径

**违反合同 §2 规则**："任何新路径若与下列不一致，必须先更新本合同再合入。"

**证据**：
- `engine/src/domain/chat.rs:884-906`：`delete_character` 持 `character.write()` 后调用 `create_backup`，后者 acquire `BACKUP_LOCK`。锁序为 `character.write() → BACKUP_LOCK`。
- `engine/src/domain/chat.rs:946-972`：`delete_session` 持 `character.read() + session_lock` 后调用 `create_backup`。锁序为 `character.read() → session_lock → BACKUP_LOCK`。
- `engine/src/backup/snapshot.rs:465-469`：`restore_backup` acquire `BACKUP_LOCK` 后调用 `create_backup_locked`（不重入 BACKUP_LOCK），无 character_lock。锁序为 `BACKUP_LOCK`（叶锁，无嵌套）。

**严重度**：W-01（warning）

**修复建议**：
1. §1.5 新增 `BACKUP_LOCK` 行：`| BACKUP_LOCK | backup/snapshot.rs | Mutex<()>（std） | 串行化 backup vs backup / backup vs restore |`
2. R4 列表新增 `BACKUP_LOCK`
3. §2 新增 §2.9 记录 `delete_character` / `delete_session` 的 `character_lock → BACKUP_LOCK` 路径
4. §7 验收记录新增 PR #445 条目

---

### 2.3 W-02：restore_backup 不持 character_lock，可与并发写竞态

**发现**：

`restore_backup`（`snapshot.rs:465-549`）仅 acquire `BACKUP_LOCK`，不 acquire 任何 `character_lock` / `session_lock` / `state_lock`。restore 的 swap 阶段直接修改 `data_root` 文件系统。

`BACKUP-RESTORE.md` §5 和 `backup/mod.rs` 模块注释声称："`PreDelete` / `PreRestoreRollback` 场景由调用方持有的 character_lock / backup_lock 自然串行化相关资源。"

**该说法部分不准确**：
- `PreDelete`：`delete_character` / `delete_session` 持 `character_lock` 调 `create_backup`（acquire `BACKUP_LOCK`），backup 阶段确实被 character_lock 串行化。但 `delete_character` 的 `remove_dir_all` 阶段（`chat.rs:908`）在 `BACKUP_LOCK` 释放后、`character.write()` 持有期间执行——此时若并发 `restore_backup`，restore 的 swap 与 delete 的 remove_dir_all 竞态。
- `PreRestoreRollback`：`restore_backup` 内部调 `create_backup_locked`（持 `BACKUP_LOCK`），rollback backup 创建阶段确实串行化。但 restore 的 swap 阶段（`swap_full_data_root` / `swap_scoped_subtree`）不持任何 character_lock，可与并发的 `append_to_current`（持 `character.read() + session_lock`）、`StateService::mutate`（持 `character.read() + state_lock`）竞态。

**实际风险**：用户在维护窗口执行 restore 时风险低（无活跃 session）；但文档应更准确地描述限制，不应声称"自然串行化"。

**严重度**：W-02（warning）

**修复建议**：
1. `backup/mod.rs` 模块注释和 `BACKUP-RESTORE.md` §5 修正措辞：明确"backup 创建阶段由 BACKUP_LOCK 串行化；restore swap 阶段不持 character_lock，必须确保无活跃写"
2. follow-up issue：考虑在 `restore_backup` swap 阶段 acquire 所有 character_lock（或文档化"restore 前必须暂停 daemon"更强约束）

---

### 2.4 W-03：sync_dir 在 Windows 上为 no-op，crash safety 弱于 Unix

**发现**：

`engine/src/backup/snapshot.rs:816-828` 的 `sync_dir` 在 Windows 上完全不打开目录句柄：

```rust
#[cfg(not(unix))]
{
    let _ = path;
}
```

这意味着 Windows 上 backup/restore 的 staging → rename 流程中，目录元数据变更不 fsync。若 Windows 在 rename 后、下一次 fsync 前崩溃，目录条目可能丢失。

这与 `revision::atomic::sync_dir` 行为一致（既有 debt），但 `BACKUP-RESTORE.md` 未在 v1 限制中提及 Windows crash safety 弱于 Unix。

**严重度**：W-03（warning）

**修复建议**：`BACKUP-RESTORE.md` §5 v1 限制表新增一行说明 Windows sync_dir no-op。

---

### 2.5 W-04：缺少 restore 失败保留 staging + rollback 的测试

**发现**：

计划 §3 Slice 2 要求："restore 失败时 data_root 不半删（模拟 staging 写入失败）"。

现有测试 `restore_rejects_tampered_backup`（`snapshot.rs:1302-1319`）验证了 verify 失败时 data_root 不变，但 verify 在 rollback backup 创建之前，所以该测试不覆盖"rollback 已创建后 swap 失败"的场景。

缺少以下测试：
- swap 阶段失败后 staging 目录仍存在
- swap 阶段失败后 rollback backup 仍存在且可读
- swap 阶段失败后返回 `Internal` 错误

**严重度**：W-04（warning）

**修复建议**：新增测试，模拟 swap 阶段失败（如向 staging 注入不可 rename 的路径），验证 staging + rollback backup 保留。

---

### 2.6 W-05：validate_backup_id_segment（HTTP）与 validate_backup_id（manifest）规则不一致

**发现**：

| 校验函数 | 文件 | 拒绝 `:` | 允许字符集 |
|---|---|---|---|
| `validate_backup_id_segment` | `backups.rs:224-244` | 否 | 仅拒绝 `/ \ . .. 空字节` |
| `validate_backup_id` | `manifest.rs:274-295` | 是 | 仅允许 `alphanumeric + - + _` |

`validate_backup_id_segment` 更宽松，允许 `:` 等 Windows 非法文件名字符通过 HTTP 校验。虽然 `read_backup_manifest` 会通过 `from_json_bytes` → `validate_backup_id` 拒绝不合规 manifest（fail-closed），但两层校验不一致会增加混淆。

**严重度**：W-05（warning）

**修复建议**：`validate_backup_id_segment` 改为复用 `validate_backup_id`，或至少补齐 `:` 拒绝。

---

### 2.7 W-06：BACKUP-RESTORE.md 未在"当前不能宣称"层校准

**发现**：

`CURRENT-BASELINE.md` §4「当前不能宣称」仍包含："不能宣称完整 session 自包含、跨资源 Turn 事务、全仓统一 migration registry、自动定时备份/恢复、浏览器矩阵或长会话 soak 已交付。"

以及 §2.2 能力矩阵："高级生命周期、完整导出/恢复未闭合（#342/#346）"。

PR #445 交付了 #342 的最小闭环，但 `CURRENT-BASELINE.md` 未更新。BACKUP-RESTORE.md 也不应替代基线更新——基线应在 PR 合并后由开发 agent 校准（非审计 agent 职责）。

**严重度**：W-06（warning，process）

**修复建议**：PR 合并后由开发 agent 更新 `CURRENT-BASELINE.md` §2.2 能力矩阵与 §4「不能宣称」列表，将 #342 backup/restore 最小闭环标记为已交付（含 v1 限制）。

---

### 2.8 实现质量评估（独立确认）

以下方面经独立复核确认实现正确：

**backup manifest 完整性** ✅
- schema 版本校验（`from_json_bytes` 强制 `schema == 1`）
- hash 校验（per-file SHA-256 + tree SHA-256，`verify_against_disk` 完整重算）
- secret 排除（`SECRET_EXCLUDE_LIST` denylist，`secrets_excluded: true` 强制）
- 路径安全（`validate_approved_path` 拒绝 `..` / 绝对路径 / 反斜杠 / 非 NFC）
- 版本兼容（`data_schema_version` 向前兼容旧 backup，拒绝未来版本）

**原子 snapshot 工作流** ✅
- staging → copy → hash → manifest → verify → sync_dir → rename → sync_dir
- 残留 staging 清理（`create_backup` 和 `restore_backup` 都检查并清理残留 staging）
- `sync_dir` 在 Unix 上 `sync_data`，Windows 上 no-op（与既有 `revision::atomic::sync_dir` 一致）

**restore 正确性** ✅
- fail-closed：verify 失败 → 拒绝 restore，data_root 不变
- rollback backup：restore 前自动创建 `PreRestoreRollback` Full scope backup
- scoped restore：`swap_scoped_subtree` 仅替换目标子树，保留无关数据（测试 `restore_scoped_backup_preserves_unrelated_data` 证明）
- post-restore 校验：重新枚举 data_root，对比 manifest files + 重算 SHA-256

**BACKUP_LOCK 语义** ✅
- `std::sync::Mutex` 选择合理（sync 文件 I/O，调用方用 `spawn_blocking`）
- poison 恢复 `unwrap_or_else(|p| p.into_inner())` 与 §5 P1 一致
- `restore_backup` 调 `create_backup_locked`（非 `create_backup`）避免 `std::sync::Mutex` 不可重入死锁——正确
- lock ordering：`character_lock → BACKUP_LOCK` 是合法外→内序列（BACKUP_LOCK 是叶锁）

**pre-delete 集成** ✅
- fail-closed：`create_backup` 失败 → `delete_character` / `delete_session` 返回 `Err`，不删数据
- `force=true` 跳过 backup（HTTP endpoint + domain service）
- agent tool `delete_character` 恒传 `force=false`（不暴露绕过能力给 agent）——正确
- `spawn_blocking` 包装 sync I/O（agent tool + HTTP endpoints）

**路径穿越 / symlink 安全** ✅
- `walk_and_copy`：`symlink_metadata` 拒绝符号链接，`validate_approved_path` 双重校验
- `restore_backup`：`validate_approved_path` + `safe_resolve_for_write` 双重校验
- `swap_scoped_subtree`：`validate_approved_path(subtree_prefix)` + `safe_resolve_for_write` 校验
- scoped backup 边界对齐：`is_ancestor_or_within` / `is_within_subtree` 用 `/` 分隔符对齐，防止 `characters/alice` 误匹配 `characters/alicia`

**secret 处理** ✅
- `SECRET_EXCLUDE_LIST = ["secrets.json", "settings.json"]`（仅 data_root 根目录下同名文件）
- manifest 强制 `secrets_excluded: true`，加载时若为 `false` 直接拒绝
- restore 不写 secret（backup 不含 secret，restore 从 backup files/ 复制）
- post-restore 校验允许 secret 文件缺失

**WebUI 契约** ✅
- `renderBackup` 替换 `renderUnavailable('backup')`（`console-runtime.js:1018` renderers map）
- 5 个 API 端点全部调用（GET/POST/verify/restore/DELETE）
- 创建对话框：secret 排除警告 + 维护窗口建议
- 恢复对话框：回滚备份 + secret 需重配 + 重启 daemon 建议
- 删除对话框：不可恢复警告
- source 标签：manual / pre_delete / pre_restore_rollback

**测试覆盖** ✅（含独立运行确认）
- backup 模块单测：40 passed / 0 failed
- HTTP handler 测试：15 passed / 0 failed
- pre-delete 集成测试：4 passed / 0 failed（delete_character + delete_session × default/force）
- scoped restore 测试：1 passed（`restore_scoped_backup_preserves_unrelated_data`）
- delete_character pre-delete backup 可恢复测试：1 passed（`delete_character_pre_delete_backup_can_be_restored`）
- WebUI 契约测试：22 passed / 0 failed
- 神圣不变式 `subagent_context_has_no_orchestrator_noise`：1 passed
- clippy `--all-targets -- -D warnings`：clean

## 3. 审计结论

**FAIL（1 条阻塞意见）**。

PR #445 的实现质量整体良好：manifest schema、atomic snapshot、scoped restore、pre-delete 集成、path sandbox、secret 排除、BACKUP_LOCK 语义均经独立复核确认正确。40 + 15 + 4 + 22 + 1 = 82 条测试通过，神圣不变式保持，clippy clean。

**但 `docs/BACKUP-RESTORE.md` 与实现存在严重矛盾**（B-01）：文档在 5 处声称 scoped restore 不支持并指导用户手动拷贝文件，而实现已完整交付 scoped restore 且有测试覆盖。这违反文档诚实性不变式，并引导用户走不安全路径（绕过完整性校验、无回滚备份、无 post-restore 校验）。必须修复后才能合并。

非阻塞意见 W-01 ~ W-06 按 AGENTS.md「审计遗留项处理」规则，PR 合并后由执行审计的 agent 写入 GitHub issue。

## 4. Follow-up issues（PR 合并后提交）

| 编号 | 类型 | 描述 | 建议时机 |
|---|---|---|---|
| W-01 | 合同更新 | `LOCK-ORDER-CONTRACT.md` 未更新 `BACKUP_LOCK`。§1.5 全局 utility 锁清单、R4 叶锁规则、§2 嵌套路径均未记录。违反合同 §2「任何新路径若与下列不一致，必须先更新本合同再合入」 | PR 合并后立即（docs-only PR） |
| W-02 | 文档修正 + follow-up | `restore_backup` swap 阶段不持 character_lock，可与并发写竞态。文档"自然串行化"说法不准确。应修正措辞 + 考虑 follow-up 在 swap 阶段 acquire character_lock | 后续 PR（措辞修正）+ follow-up issue（锁收敛） |
| W-03 | 文档补充 | `sync_dir` 在 Windows 上为 no-op，crash safety 弱于 Unix。`BACKUP-RESTORE.md` §5 v1 限制表应补充说明 | 后续 PR |
| W-04 | 测试补充 | 缺少 restore swap 阶段失败后保留 staging + rollback backup 的测试。计划 §3 Slice 2 要求"模拟 staging 写入失败"但未交付 | 后续 PR |
| W-05 | 代码一致性 | `validate_backup_id_segment`（HTTP）与 `validate_backup_id`（manifest）规则不一致。前者不拒绝 `:`，后者拒绝。应统一 | 后续 PR |
| W-06 | 基线校准 | `CURRENT-BASELINE.md` §2.2 能力矩阵与 §4「不能宣称」需更新 #342 backup/restore 最小闭环已交付状态（含 v1 限制） | PR 合并后由开发 agent 校准 |

---

审计 agent：（独立审计 mode，遵循 AGENTS.md 三原则）
