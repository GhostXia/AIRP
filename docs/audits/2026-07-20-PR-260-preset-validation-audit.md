# PR #260 独立审计报告 — export_context_bundle preset 验证前置（#160 A3）

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#260 engine(agent/tools): validate preset before mutating context bundle (#160 A3)](https://github.com/GhostXia/AIRP/pull/260)
- **分支**：`codex/engine-160-a3-preset-validation`
- **base**：`main`（mergeStateStatus: CLEAN，mergeable: MERGEABLE）
- **commits**：单 commit
- **CI**：Rust workspace SUCCESS / UI and WebUI SUCCESS / Production topology SUCCESS / Portable Windows WebUI SUCCESS / CodeRabbit SUCCESS

## 1. 范围与背景

#160 是 PR #159 审计遗留 issue，A3 是其中 High severity 项。原 `export_context_bundle`
在 `engine/src/agent/tools/volume_context.rs` 中的顺序为：

1. `ensure_context_bundle_dir` 创建 bundle 目录
2. 清理旧 `preset_raw.json` / `extensions.json` sidecar
3. **验证 preset 路径存在**（缺失返回 `AirpError::NotFound`）
4. 复制 preset / 写 extensions / 写 context.md

第 3 步返回 `NotFound` 时，bundle 处于"半提交"不一致状态：
- `preset_raw.json` 已被删除
- `extensions.json` 已被删除
- `context.md` 是旧版本（仍引用已删除的 sidecar）

调用方拿到 NotFound 想重试或换 preset，但 bundle 已经处于不一致状态，下一次成功
导出才能恢复——这不是"调用失败"，是"半提交"。

## 2. 独立证据

### 2.1 改前 / 改后对照（`engine/src/agent/tools/volume_context.rs`）

改前（main）顺序（L222-L243）：

```rust
let bundle_dir = data_dir::ensure_context_bundle_dir(...)?;       // 1. 建目录
for stale in ["preset_raw.json", "extensions.json"] {             // 2. 清 sidecar
    let path = bundle_dir.join(stale);
    if tokio::fs::try_exists(&path).await? {
        tokio::fs::remove_file(path).await?;
    }
}

let mut files = vec!["context.md".to_string()];
if let Some(preset) = preset_id.as_ref() {                        // 3. 验证 preset（缺失返回 NotFound）
    let raw_path = data_dir::preset_json_path(&state.data_root, preset.as_str());
    if !tokio::fs::try_exists(&raw_path).await? {
        return Err(AirpError::NotFound(format!("preset {} has no preset.json", preset)));
    }
    tokio::fs::copy(raw_path, bundle_dir.join("preset_raw.json")).await?;  // 4. 复制
    files.push("preset_raw.json".to_string());
    context.push_str("\n> `preset_raw.json` is verbatim passthrough; ...\n");
}
```

改后（PR）顺序（L222-L254）：

```rust
// 1. 验证 preset 路径存在（缺失返回 NotFound），bundle 目录尚未被触碰
let preset_raw_path = if let Some(preset) = preset_id.as_ref() {
    let raw_path = data_dir::preset_json_path(&state.data_root, preset.as_str());
    if !tokio::fs::try_exists(&raw_path).await? {
        return Err(AirpError::NotFound(format!("preset {} has no preset.json", preset)));
    }
    Some(raw_path)
} else {
    None
};

let bundle_dir = data_dir::ensure_context_bundle_dir(...)?;       // 2. 建目录
for stale in ["preset_raw.json", "extensions.json"] {             // 3. 清 sidecar
    let path = bundle_dir.join(stale);
    if tokio::fs::try_exists(&path).await? {
        tokio::fs::remove_file(path).await?;
    }
}

let mut files = vec!["context.md".to_string()];
if let Some(raw_path) = preset_raw_path.as_ref() {                // 4. 复制
    tokio::fs::copy(raw_path, bundle_dir.join("preset_raw.json")).await?;
    files.push("preset_raw.json".to_string());
    context.push_str("\n> `preset_raw.json` is verbatim passthrough; ...\n");
}
```

### 2.2 行为矩阵（独立 trace）

| 输入 | 改前 | 改后 | 等价？ |
|---|---|---|---|
| preset 存在 | 全部文件写入 | 全部文件写入 | 等价 |
| preset 缺失 → NotFound | sidecar 已删除，context.md 旧版本 | bundle 完全未被触碰 | **不等价（这是修复目标）** |
| preset_id = None | 不写 preset_raw.json | 不写 preset_raw.json | 等价 |

### 2.3 错误码 / 消息 / HTTP 状态不变性

- 错误类型：`AirpError::NotFound`（改前改后一致）
- 错误消息：`"preset {} has no preset.json"`（改前改后一致，字符串完全相同）
- HTTP 状态：由上层 `AirpError::NotFound` → 404 映射决定（未改）

### 2.4 `preset_json_path` 路径解析（`engine/src/data_dir/paths.rs` L339-L344）

```rust
pub(crate) fn preset_json_path(root: &Path, preset_id: &str) -> PathBuf {
    committed_preset_dir(root, preset_id)
        .unwrap_or_else(|| root.join("presets").join(preset_id))
        .join("preset.json")
}
```

`committed_preset_dir` 优先返回 `versions/<gen>/` 目录，否则 fallback 到
`root/presets/<id>/`。改后 PR 把 `preset_json_path` 的结果 `raw_path` 存入
`preset_raw_path: Option<PathBuf>`，复用给后续 `tokio::fs::copy`，**不会重复
解析路径**，避免"验证路径 vs 复制路径不一致"的潜在竞态。

### 2.5 `live_state` 读取顺序

`live_state = tokio::fs::read_to_string(&state_path)` 发生在 L216-L220，
**在 preset 验证之前**。这是纯读取（无副作用），preset 验证失败时 `live_state`
已被读入内存但不写回 bundle，所以不影响 bundle 状态。无需调整顺序。

### 2.6 回归测试（`engine/src/agent/tools/tests/volume_context.rs` L93-L179）

`export_context_bundle_invalid_preset_does_not_modify_existing_bundle`：

1. **baseline 建立**：合法 preset `"story"` 导出一次，记录 baseline
   `context.md` / `preset_raw.json` / `extensions.json` 字节内容。
2. **触发 NotFound**：用不存在 preset `"does-not-exist"` 再调用，断言返回
   `AirpError::NotFound`。
3. **核心断言**：baseline 三文件字节级保持。

```rust
assert!(matches!(second, Err(ref e) if matches!(e, crate::error::AirpError::NotFound(_))));
assert_eq!(std::fs::read_to_string(bundle.join("context.md")).unwrap(), baseline_context, ...);
assert_eq!(std::fs::read_to_string(bundle.join("preset_raw.json")).unwrap(), baseline_preset_raw, ...);
assert_eq!(std::fs::read_to_string(bundle.join("extensions.json")).unwrap(), baseline_extensions, ...);
```

测试覆盖完整：preset 缺失路径下，三个 bundle 文件全部断言字节级保持。

### 2.7 测试基线

PR 描述：755 lib tests = 754（main baseline）+ 1 新回归测试。
- 755 passed; 0 failed; 1 ignored
- 25 passed (integration)
- clippy: 0 warnings
- fmt: clean

CI Rust workspace SUCCESS。基线增量与 PR 描述一致。

## 3. 独立意见（按 §Audit Agent Charter 第 2 条）

### 3.1 关于"半提交"剩余风险

PR 描述明确："修复方向是验证前置，不是重新设计原子事务"。本审计同意此范围限定。
但需明确：即使 preset 验证前置后，`tokio::fs::copy` 在写 `preset_raw.json`
中途失败（如磁盘满、权限撤销），仍可能留下**部分写入**的 sidecar，bundle 仍
可能不一致。issue #160 没有要求修这个，本 PR 也不修。这是已知限制，应在 #160
后续跟踪。

### 3.2 关于"`context.md` 旧版本"问题的彻底性

改前流程中，`context.md` 旧版本的不一致来源是：sidecar 已删除但 context.md
仍引用。改后流程中，preset 验证失败时 `context.md` 写入（L282）尚未发生，
所以 context.md 仍是上一次成功导出的版本——**与 sidecar 一致**。这彻底消除
了"sidecar 已删 / context.md 仍引用"的不一致。本审计同意修复彻底性。

### 3.3 关于 `live_state` 读取的副作用

`tokio::fs::read_to_string` 在 NotFound 上不抛错（main 上是 `match` 返回 None），
所以 `live_state` 读取本身不会让 preset 验证失败路径产生副作用。无需调整。

### 3.4 关于 #160 A1 / A2 不在本 PR

PR 描述明确 A1（async I/O Medium）和 A2（错误文案复用 Low）不在本 PR：
- A1 涉及 `io::Error` → `AirpError` 的 NotFound 文案 mapping，需独立评审
- A2 涉及可观察错误文本变化，需独立评审

本审计同意此范围限定。A1 / A2 应作为 #160 后续 PR 单独评审，不在本 PR 强求。

## 4. 风险评估

| 风险 | 评级 | 说明 |
|---|---|---|
| preset 验证前置遗漏路径 | 零 | `preset_json_path` 调用位置唯一，Option<PathBuf> 复用避免重复解析 |
| 错误码 / 消息 / HTTP 状态变化 | 零 | `AirpError::NotFound` 文案完全一致 |
| 跨文件原子性 | 已知限制 | `tokio::fs::copy` 中途失败仍可能留下部分 sidecar，issue #160 未要求修 |
| 测试覆盖缺口 | 零 | 回归测试覆盖三文件字节级保持 |
| CI flaky | 零 | CI 5/5 SUCCESS |

## 5. 阻塞项

无。所有 CI 通过，行为修复彻底，回归测试完整，scope 严格限定 A3。

## 6. 非阻塞 / 后续可追踪项

| 编号 | 内容 | 建议 |
|---|---|---|
| 260-A1（非阻塞） | `tokio::fs::copy` 中途失败仍可能留下部分写入的 sidecar | 跟进 #160 后续，独立设计跨文件原子事务 |
| 260-A2（非阻塞） | #160 A1（async I/O Medium）未修 | 跟进 #160 后续 PR |
| 260-A3（非阻塞） | #160 A2（错误文案复用 Low）未修 | 跟进 #160 后续 PR |

## 7. 审计结论

**通过（PASS，无阻塞项）**。

PR #260 是 #160 A3 (High severity) 的精准修复：preset 验证前置 + 回归测试。
修复彻底（消除 sidecar/context.md 不一致），错误码/消息/HTTP 状态不变，CI 全绿，
scope 严格。可合并。

## 8. Refs

- Issue #160（A3 来源）
- PR #159 审计报告
- 根 `AGENTS.md` §Audit Agent Charter
