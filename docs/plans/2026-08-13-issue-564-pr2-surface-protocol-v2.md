# #564 PR 2：Surface Protocol v2 合同

> 状态：PR 2 协议层实现记录。基线：`5fa493c691d533698fb6329a6d79299dc43ad402`。
>
> 本文只记录 Blueprint/Surface Protocol v2、Rust/TypeScript binding、fixture 和
> guard/store 合同；不声称 Surface Engine endpoint、HttpEngineBus、桌面 shell 或
> 桌面 UI 已交付。

## 1. 唯一 authority 与边界

机器可读唯一 authority 是
[`protocol/surface-protocol-v2.json`](../../protocol/surface-protocol-v2.json)。
Rust 与 TypeScript 的常量、序列化测试和负例测试均绑定该文件；本 PR 不把
`protocol/sse-events.json`、`widget-grants.json` 或 `widget-intents.json` 改作
Surface 合同。

v2 的可执行边界是刻意关闭的：Blueprint 只描述布局节点、稳定 Widget instance
ID、Widget registry type 和 JSON props。已知字段没有 HTML、CSS、JavaScript、Vue、
eval、component-source 或函数合同；这些字段名即使出现在未知 additive data 中也会
被 guard 拒绝。其他未知字段可以被忽略或作为 opaque data 保留，绝不会进入代码加载、
DOM 注入或动态执行路径。

## 2. 数据合同

`BlueprintV2` 是 `version: 2` 加递归 `root` 和 Widget instance 表：

| 节点 | 必需结构 | 引用规则 |
|---|---|---|
| `split` | `id`、`orientation`、恰好两个 `children` | 子节点 ID 不重复 |
| `tabs` | `id`、`active`、`children` | `active` 必须是直接子节点 ID |
| `stack` | `id`、非空 `children` | 子节点 ID 不重复 |
| `widget` | `id`、`instanceId` | `instanceId` 必须存在于 `widgets`，且只能放置一次 |

所有 layout node ID 和 Widget instance ID 必须非空、受长度限制并唯一。v2 guard
拒绝缺失引用、重复 instance、错误 node kind 和 v1 的 `layout.areas` 形状。

Surface snapshot 的固定字段是：

```text
kind: "snapshot"
protocol: { major: 2, minor: >= 0 }
surfaceId: bounded identifier
revision: decimal u64 JSON string
blueprint: BlueprintV2
```

Patch event 的固定字段是 `kind: "patch"`、同一 `protocol`/`surfaceId`、
`baseRevision`、`revision` 和 RFC 6902 子集 `patch`。revision 永远是十进制字符串，
因此不会经过 JavaScript `number` 舍入；一次 patch 必须满足
`revision = baseRevision + 1`。`/kind`、`/protocol`、`/surfaceId`、`/revision`
及其子路径不可被 patch 改写；除只读 `test` 外，根路径也不可作为目标或来源，避免
整份替换绕过不可变元数据。

## 3. 兼容矩阵

| 发送方 | 接收方 | 结果 |
|---|---|---|
| major 2 / minor 0 | major 2 / minor 0 | 接受 |
| major 2 / 新 minor | major 2 / minor 0 | 接受 additive 字段；未知字段 opaque |
| major 1 | major 2 | 拒绝；必须走显式 v1 migration |
| 未知 major（例如 3） | major 2 | fail closed，要求 snapshot resync |

major/minor 均使用 unsigned 16-bit component；minor `0..=65535` 内按 additive 兼容，
超界值返回 `invalid_version`。

v1 demo 的 `Envelope`、`Blueprint`、`guard.ts` 行为保持不变。唯一的迁移入口是
Rust `migrate_v1_blueprint` 与 TS `migrateV1Blueprint`：它们把 v1 area 中的 Widget
引用转成确定性的 `stack`/`widget` 树。迁移测试只消费专用 fixture；fixture、demo
数据和 MockBus 数据都不是用户资产，不迁移角色卡、世界书、会话或记忆。

## 4. 资源与错误合同

authority 固定以下上限：document 1,048,576 bytes、patch 65,536 bytes、patch
operations 256、Blueprint depth 16、node 512、Widget instances 128、单节点 children 32、ID
128 字符。Rust 与 TS 在反序列化/guard 边界拒绝超限输入。

稳定错误码为：

```text
unsupported_major, invalid_version, invalid_revision, revision_mismatch,
revision_gap, invalid_blueprint, duplicate_instance_id, invalid_reference,
invalid_patch, resource_limit, document_too_large,
forbidden_executable_field, resync_required
```

`SurfaceStore` 先在副本上按顺序执行所有 patch，再对完整 snapshot 做 guard；任一
操作失败、结果不再是合法 v2、revision 不匹配或需要补快照时，当前 snapshot 与
last-known-good 都保持不变，并返回 resync request。成功提交后才更新 revision 和
last-known-good。

## 5. Fixtures 与负例

`protocol/fixtures/surface-v2/` 中的内容是协议测试数据，不是用户资产：

- `rust-to-ts.json`：Rust v2 snapshot 构造/序列化与 TS guard 消费。
- `ts-to-rust.json`：TS patch event 与 Rust deserialize/validation。
- `v1-migration.json`：显式默认布局迁移输入与期望输出。
- `negative.json`：unknown major、重复/孤立 instance、无效引用、可执行字段、根替换、
  revision gap、wire revision 越界与 u64 revision 递增溢出。

`guard.test.ts` 覆盖 authority parity、双向 fixture、unknown additive/major、v1
拒绝、重复/引用/安全字段和 depth/node/document/patch 限制；`surface-v2.test.ts`
覆盖原子 patch、坏 patch、revision mismatch/resync、失败保持 last-known-good 和
v1 migration。Rust fixture tests 覆盖同一 authority、序列化等价、revision 边界和
负例拒绝。

## 6. 本 PR 不包含的内容

本 PR 不修改 Engine、WebUI、Tauri relay、SSE/grant/intent 合同，不实现 Surface
Engine endpoint、HttpEngineBus、renderer、首方 Widget、持久化 workspace 或桌面
产品 UI。上述内容属于后续 PR 的调用方/交付层，必须在本协议通过独立审计后再接入。
