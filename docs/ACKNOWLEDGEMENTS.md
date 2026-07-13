# 项目沿革、设计参考与致谢

> 状态：**待持续更新的活文档**
>
> 最后核验：2026-07-13

本文区分 AIRP 作者自己的前序项目、第三方设计参考，以及未来可能发生的第三方资产复用。新增研究对象、实际采用外部资产或上游许可证变化时，必须同步更新本文。

## 1. 第一方项目沿革

以下仓库均为 AIRP 作者自己的前序项目，不属于第三方致谢或外部依赖：

| 前序项目 | 在 AIRP 中的关系 |
|---|---|
| AIRP-Core / AIRPCLI | AIRP engine 主核、provider adapter、chat pipeline、orchestrator、Agent loop 与数据层的主要前序来源 |
| AIRP-MCP-Server | RP 数据域、工具目录、工作流、路径沙箱和插件数据面的前序来源 |
| AIRP-Gateway | 传输、安全硬化、路由与 MCP client 设计的前序来源 |
| AIRP-State-Protocol | Blueprint、Widget、Envelope、state patch、guard、虚拟滚动及 consent/sandbox 的前序来源 |

这些资产在当前仓库中按 AIRP 产品需求汇聚和重构，不继承各前序仓库原有的独立产品目标。详细决策见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

## 2. 第三方设计参考

下表记录已经在文档或 GitHub issue 中形成明确研究结论的第三方项目。

| 项目 | AIRP 研究或吸收的经验 | 当前关系 | 审查基线（2026-07-11） | 许可证核验 | AIRP 记录 |
|---|---|---|---|---|---|
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | RP 功能清单、角色卡/世界书兼容面、Preset 与 Persona 交互、扩展生态 | 功能与互操作性参考；AIRP 按自身架构独立实现 | `8172dcd0ee` | AGPL-3.0 | [TAVERN-PARITY.md](TAVERN-PARITY.md)、[#114](https://github.com/GhostXia/AIRP/issues/114) |
| [Hermes Agent](https://github.com/NousResearch/hermes-agent) | 有界长期记忆、frozen snapshot、skills、用户建模、headless Agent 形态、credential redirect 边界 | 重要架构理念参考 | `3b2ef789df` | MIT | [HERMES-MEMORY.md](HERMES-MEMORY.md)、[#117](https://github.com/GhostXia/AIRP/issues/117) |
| [NeuroBook](https://github.com/notnotype/neuro-book) | 结构化 prompt 装配、长篇记忆、角色知识视角、Agent change inbox 与 authoring workflow | 研究参考；未作为当前 capability 事实 | `138e16d216` | AGPL-3.0 | [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)、[#117](https://github.com/GhostXia/AIRP/issues/117) |
| [pi-forge](https://github.com/MacroSony/pi-forge) | Preset 导入报告、prompt assembly trace、一次性 payload inspector、history integrity | 规划参考，尚待 issue 实施 | `161f434ba5` | MIT | [#115](https://github.com/GhostXia/AIRP/issues/115) |
| [llmlint](https://github.com/notnotype/llmlint) | 声明式风格规则、候选诊断、确认式修复、误报与分层评测 | 规划参考，尚待 issue 实施 | `9aabfc2839` | PolyForm Noncommercial 1.0.0 | [#116](https://github.com/GhostXia/AIRP/issues/116) |

列入本表仅表示 AIRP 曾研究其公开设计、产品行为或互操作格式，不表示原项目维护者认可、参与或支持 AIRP，也不自动表示 AIRP 复用了其代码、规则、数据、测试或视觉资产。

## 3. 第三方资产复用规则

致谢不能替代许可证合规。如果未来复制、修改、链接或分发第三方代码或资产，必须在合入前：

1. 记录上游仓库、固定 commit/tag、具体文件和用途；
2. 核对许可证、版权声明、NOTICE、商标及再分发要求；
3. 记录 AIRP 的修改范围，并在源码或 notice 中保留必要归属；
4. 确认与 AIRP 的 `MIT OR Apache-2.0` 分发方式兼容；
5. 若只研究理念并独立实现，保留设计记录和独立实现证据，不复制受保护的表达或资产。

实际第三方资产一旦进入仓库，应从本页的“设计参考”升级为明确的 provenance/notice 记录；不能继续只写“理念参考”。

### 已批准、尚未进入发布产物的普通依赖

| 组件 | 固定版本/核验日期 | 计划用途 | 许可证与 provenance | 当前状态 |
|---|---|---|---|---|
| [Caddy](https://github.com/caddyserver/caddy) / [Docker Official Image](https://hub.docker.com/_/caddy) | `2.11.4` / 2026-07-13 | WebUI 首方 OCI/Compose bundle 的 HTTPS、Basic perimeter auth、静态文件、安全 headers 与 reverse proxy | 上游 Caddy `v2.11.4` 为 Apache-2.0；执行 slice 还须固定镜像 digest，并为基础镜像/传递组件生成 notices/SBOM | [P0 架构](WEBUI-PRODUCTION-ARCHITECTURE.md)已批准选择；镜像、Compose 与分发仍未进入仓库 |

AIRP 只配置并分发普通上游组件，不复制、翻译或改写其源码/文档。上表不把“计划采用”写成当前已交付能力；真正加入镜像后必须补 digest、构建 provenance 和机器可读 notices。

## 4. 维护待办

- [ ] 每次新增外部研究 issue 或学习文档时，将项目补入本表。
- [ ] 每次准备复用第三方资产时，在合入前完成许可证与 provenance 审查。
- [ ] 发布前复核所有上游许可证是否变化，并更新“最后核验”日期。
- [ ] 若第三方组件随二进制分发，建立并维护机器可读的 third-party notices/SBOM。
- [ ] 定期检查“规划参考”是否已经落地；落地后补充实现位置和验证证据。

## 5. 当前非致谢对象

审计模型、IDE 托管服务、代码审查机器人及贡献者 trailer 属于工具或历史来源记录，不因参与审计就成为 AIRP 的设计上游。此类来源应保留在对应 audit/commit/issue 中，不加入本页项目致谢，除非未来确实吸收了其公开项目设计。
