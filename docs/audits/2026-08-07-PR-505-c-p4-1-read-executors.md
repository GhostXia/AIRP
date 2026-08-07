# 审计报告：PR #505 C-P4.1 Widget Intent Read Executors

- **审计来源 LLM**：GLM-5.2
- **审计时间**：2026-08-07
- **审计对象**：PR #505（feat/c-p4-1-widget-intent-read-executors, head 6422573）
- **审计依据**：AGENTS.md「Audit Agent Charter」三原则（独立审计 / 可提己见 / 可质疑历史并查证）
- **审计方法**：独立读源码（api.rs / widget-intents.json / extensions.rs tests）+ 独立跑 `cargo test -p airp-core --lib`（1399 passed, 0 failed, 5 ignored）+ `cargo clippy -- -D warnings`（clean）+ 交叉验证 CharacterId / validate_id_segment / LorebookService::read / StateService::read / read_resident_memory

## 改动概要

C-P4.1：widget intent 授权通过后派发真实 read 执行器（read:memory / read:state / read:worldbook），替代 C-P3 的 echo 语义。未实现执行器的 capability（write:*/call:tool）保持 echo（C-P4.2，YAGNI）。

- `engine/src/extensions/api.rs`：`widget_intent` 授权通过分支按 capability 派发 → `exec_intent_read()`，同步 IO 包 `spawn_blocking`（#433 合规）
- `protocol/widget-intents.json`：合同同步——追加 200/400/404/500 响应定义与 3 个新 error code（additive-only）
- `engine/src/daemon/tests/extensions.rs`：重构 `cp3_intent_allowed_when_capability_granted` + 新增 `cp4_1_read_intent_executors_return_data`（三读成功 / 缺参 400 / 路径遍历 400 / 目标缺失 404 / session_id 类型错 400 / 损坏 JSON 500 脱敏）

## 已核实（V-series，独立验证）

| 编号 | 核实项 | 证据 |
|------|--------|------|
| V1 | 路径遍历防护 | `CharacterId::new` → `data_dir::validate_id_segment`（security.rs:96-121）：拒绝 `.`/`..`/以`.`开头/`/`/`\`/`\0`/`:`/`*`/`?`/`"`/`<`/`>`/`\|`/含`..`。测试覆盖三条 read capability 逐一遍历 `../evil` → 400 |
| V2 | spawn_blocking 合规（#433） | `exec_intent_read` 全部同步 IO（`std::fs::read_to_string` / `LorebookService::read` / `read_resident_memory`）在 `tokio::task::spawn_blocking` 内执行，lease 留在 async 上下文 |
| V3 | 错误脱敏 | 500 → `intent_executor_error` + `"internal error"`（W9 修复），细节只进 `tracing::error!`，不携带 IO 路径 / JSON 解析细节 |
| V4 | 审计日志完整 | executor 分支 `tracing::info!`（W10 修复）字段集 = allow/deny 分支（intent/widget_type/instance_id/capability/extension_id） |
| V5 | session_id 严格校验 | present but non-string → 400 `intent_bad_params`（W10 修复），不静默忽略类型错误 |
| V6 | 执行器语义与既有 handler 一致 | `read:state` ≈ `get_character_state`（state.rs:40-53，直接读 `live.json` 无锁）；`read:memory` ≈ `read_resident_memory`（resident.rs:42-53，无锁）；`read:worldbook` ≈ `LorebookService::read`（lorebook.rs:32-51，带锁 + v3→v4 迁移） |
| V7 | 合同锁测试 | `compat_known_capabilities_match_docs` 通过——KNOWN_CAPABILITIES 与 docs/WIDGET-DEVELOPMENT.md §5 一致 |
| V8 | error.code 封闭锁定集 | protocol/widget-intents.json `extensionRules[2]` 更新为 `{intent_invalid, intent_denied, intent_bad_params, intent_target_missing, intent_executor_error}` |
| V9 | 1399 lib tests 全绿 | 独立运行 `cargo test -p airp-core --lib`：1399 passed, 0 failed, 5 ignored |
| V10 | clippy clean | `cargo clippy -p airp-core --all-targets -- -D warnings`：无警告 |
| V11 | additive-only 兼容 | 200 响应追加 `result` 字段（可选）；消费端忽略未知字段（合同铁律）；C-P3 测试 `cp3_intent_allowed_when_capability_granted` 通过 |

## 非阻塞项（N-series，后续迭代）

| 编号 | 项 | 说明 |
|------|----|------|
| N1 | `read:state` 执行器解析 JSON，既有 handler 返回原始文本 | executor 用 `serde_json::from_str` 解析后返回 `Value`；handler `get_character_state` 直接返回 `fs::read_to_string` 原始文本（Content-Type: application/json 但未解析）。若 `live.json` 含非法 JSON：executor → 500，handler → 200 原始文本。语义差异但可接受——executor 合同定义 `result` 为 object，handler 返回原始文本可视为既有行为。 |
| N2 | `char_count` 语义 | `content.chars().count()` 统计 Unicode 标量值，非字节或 grapheme cluster。合同应注明语义（建议 docs 补充）。 |
| N3 | `read:memory` 返回 `capacity` 来自默认配置 | `ResidentMemoryConfig::default().capacity_chars`——若用户自定义了 capacity，执行器返回的仍是默认值。建议从配置读取实际值。 |
| N4 | 无 widget 消费端集成测试 | 测试仅覆盖 engine 侧（POST /v1/widget-intents）；webui boot.js onIntent 消费 `result` 字段的端到端测试未在此 PR（webui 改动在 #507）。 |

## 未执行视觉审查

本 PR 为 engine-only（Rust 后端 + JSON 协议合同），不涉及 WebUI 视觉改动。按 AGENTS.md 规则无需多模态补审。

## 结论：**APPROVE**

无阻塞项。W9/W10 修复已解决先前审计意见。路径遍历防护、spawn_blocking 合规、错误脱敏、审计日志完整、合同 additive-only 兼容均已独立验证。N-series 为非阻塞后续迭代项。
