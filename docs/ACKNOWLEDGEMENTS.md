# 项目沿革、设计参考与致谢

> 状态：**待持续更新的活文档**
>
> 最后基线校准：2026-07-18，`main@2a14b7e`；上游版本与许可证的实际核验日期见各表。`tools/dep-governance/`（PR #218/#229）已提供 Cargo + npm 依赖发现与 SPDX/CycloneDX SBOM 生成器，当前 SBOM 快照存于 `docs/sbom/`；该工具是手动离线运行，不替代引入新依赖时的逐项许可证/provenance 核验。

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

| 项目 | AIRP 研究或吸收的经验 | 当前关系 | 审查基线（固定版本/日期） | 许可证核验 | AIRP 记录 |
|---|---|---|---|---|---|
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | RP 功能清单、角色卡/世界书兼容面、Preset 与 Persona 交互、扩展生态、多来源世界书绑定、用户数据隔离，以及用户数据目录内集中保存且默认不回显 API secrets 的公开行为 | 功能、公开行为与互操作性参考；AIRP 采用自己的版本化 `data/secrets.json` schema、稳定 ID、session 自包含快照和 provenance 独立实现，不复用其代码或 schema | `380e31e8c58d196969b6a0da74f431ba999c7e0a` / 2026-07-12 checkout，secrets 行为 2026-07-19 复核 | AGPL-3.0 | [TAVERN-PARITY.md](TAVERN-PARITY.md)、[SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[#168](https://github.com/GhostXia/AIRP/issues/168) |
| [Hermes Agent](https://github.com/NousResearch/hermes-agent) | 有界长期记忆、frozen snapshot、skills、用户建模、headless Agent 形态、credential redirect 边界 | 重要架构理念参考 | `3b2ef789df` | MIT | [HERMES-MEMORY.md](HERMES-MEMORY.md)、[#117](https://github.com/GhostXia/AIRP/issues/117) |
| [NeuroBook](https://github.com/notnotype/neuro-book) | 结构化 prompt 装配、长篇记忆、角色知识视角、Agent change inbox 与 authoring workflow | 研究参考；未作为当前 capability 事实 | `138e16d216` | AGPL-3.0 | [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)、[#117](https://github.com/GhostXia/AIRP/issues/117) |
| [pi-forge](https://github.com/MacroSony/pi-forge) | Preset 导入报告、prompt assembly trace、一次性 payload inspector、history integrity | 理念参考；AIRP 已按自身模型独立实现 Preset 报告/原始 sidecar/原子版本切换，以及真实 pipeline 驱动的脱敏 HTTP/WebUI trace；完整 revision/provenance 仍由 issue 跟踪 | `161f434ba5` | MIT | [#115](https://github.com/GhostXia/AIRP/issues/115)、PR #172/#174/#176/#177 |
| [llmlint](https://github.com/notnotype/llmlint) | 声明式风格规则、候选诊断、确认式修复、误报与分层评测 | 规划参考，尚待 issue 实施 | `9aabfc2839` | PolyForm Noncommercial 1.0.0 | [#116](https://github.com/GhostXia/AIRP/issues/116) |
| [caveman PR #554](https://github.com/JuliusBrussee/caveman/pull/554) | CJK 输出邻近结构化 tool call 时发生截断的用户实遇信号；将“已确认现象、根因假设、未验证缓解方案”分层记录，并以真实复现样本作为兼容性扩展门槛 | 仅作审计与兼容性决策方法参考；未确认是 AIRP 缺陷，不复用上游代码、规则文本或 prompt | `5b80d5ae15` / 2026-06-23 | MIT | [#149](https://github.com/GhostXia/AIRP/issues/149)、[AIRP 提交者的复现与换行候选说明](https://github.com/JuliusBrussee/caveman/pull/554#issuecomment-4785334058) |

列入本表仅表示 AIRP 曾研究其公开设计、产品行为或互操作格式，不表示原项目维护者认可、参与或支持 AIRP，也不自动表示 AIRP 复用了其代码、规则、数据、测试或视觉资产。

## 3. 第三方研究与普通依赖规则

用于学习、对标或吸收理念的第三方项目只允许研究公开行为、协议、格式和需求洞察；AIRP 必须按自己的 domain model、命名、控制流、安全边界和测试独立实现，不复制、翻译、改写或移植其源码、规则文本、prompt、测试、数据、HTML/CSS、图标或视觉资产。许可证表面允许也不改变这条默认规则。

普通第三方依赖库和基础设施可以模块化接入并深度参与功能，但必须满足：

1. 依赖解决的是明确的工程问题，而不是替 AIRP 决定产品边界；
2. 接入点有清晰接口、默认配置和移除/替换路径，核心数据真相不归依赖所有；
3. 锁定版本，记录上游、许可证、用途、运行时/构建时关系和分发义务；
4. 通过 AIRP 自己的安全、失败、升级和回归测试，不把上游默认值当作项目合同；
5. 若形成运行时或发布依赖，进入 provenance、notices 与 SBOM，而不是继续列作“理念参考”。

因此，模块化深度干预不违背“便于维护和未来移植”；不可替换、边界不透明、把第三方内部模型扩散到全项目才违背该准则。

### 已单独核验的普通依赖与基础设施

| 组件 | 固定版本/核验日期 | 计划用途 | 许可证与 provenance | 当前状态 |
|---|---|---|---|---|
| [Caddy](https://github.com/caddyserver/caddy) / [Docker Official Image](https://hub.docker.com/_/caddy) | `2.11.4` / 2026-07-13 | WebUI 首方 OCI/Compose bundle 的 HTTPS、Basic perimeter auth、静态文件、安全 headers 与 reverse proxy | 上游 Caddy `v2.11.4` 为 Apache-2.0；官方 multi-platform image 固定为 `sha256:af5fdcd76f2db5e4e974ee92f96ee8c0fc3edb55bd4ba5032547cbf3f65e486d` | 已进入 `deploy/production/Dockerfile.gateway`；仍须在 P3 生成完整基础镜像/传递组件 notices 与 SBOM 后才能正式发布 |
| [Debian](https://www.debian.org/) Docker Official Image | `bookworm-slim` / 2026-07-13 | `airp-core` runtime base 与 CA trust store | 官方 multi-platform image 固定为 `sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df`；各 Debian package 许可证须由最终 SBOM/notices 枚举 | 已进入 `deploy/production/Dockerfile.engine` runtime stage；正式发布 provenance 仍开放 |
| [Rust](https://www.rust-lang.org/) Docker Official Image | `1.96.0-bookworm` / 2026-07-13 | 仅用于可重复构建 `airp-core` 的 builder stage，不进入 runtime image | 官方 multi-platform image 固定为 `sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc`；Rust toolchain 为 Apache-2.0 OR MIT | 已进入 `deploy/production/Dockerfile.engine` builder stage；不随 runtime 分发 |
| [Playwright Core](https://github.com/microsoft/playwright) | `1.61.1` / 2026-07-13 | 仅作为 CI dev dependency 驱动 runner 预装的 system Chrome，验证 production WebUI CSP、文本注入安全与 SSE 取消 | npm lockfile 固定 tarball integrity；上游许可证 Apache-2.0；未下载或分发 Playwright browser bundle | 已进入 `ui/package-lock.json` 与 production topology smoke；不进入 AIRP runtime images，不复用上游测试或实现代码 |
| [Vite](https://github.com/vitejs/vite) / [Vue plugin](https://github.com/vitejs/vite-plugin-vue) / [Vitest](https://github.com/vitest-dev/vitest) | `8.1.4` / `6.0.8` / `4.1.10`，2026-07-16 核验 | `ui/` 的 Vue 构建、开发服务器与测试工具链 | 三个上游均为 MIT；manifest 使用不跨主版本的有界兼容范围，npm lockfile 固定实际版本、来源与 tarball integrity | 仅为开发/测试依赖，不进入 production WebUI gateway 或 engine runtime image；升级由 #137 / PR #191 审计 |

AIRP 只配置并分发普通上游组件，不复制、翻译或改写其源码/文档。上表分别记录 P0 artifact 的精确镜像和开发/测试依赖的锁定版本；它不把 preview artifact 写成正式发布能力。正式 tag 前仍必须补构建 provenance、机器可读 notices 与完整 SBOM。

## 4. 维护待办

- [ ] 每次新增外部研究 issue 或学习文档时，将项目补入本表。
- [ ] 每次准备复用第三方资产时，在合入前完成许可证与 provenance 审查。
- [ ] 发布前复核所有上游许可证是否变化，并更新“最后核验”日期。
- [ ] 若第三方组件随二进制分发，建立并维护机器可读的 third-party notices/SBOM。
- [ ] 定期检查“规划参考”是否已经落地；落地后补充实现位置和验证证据。

## 5. 当前非致谢对象

审计模型、IDE 托管服务、代码审查机器人及贡献者 trailer 属于工具或历史来源记录，不因参与审计就成为 AIRP 的设计上游。此类来源应保留在对应 audit/commit/issue 中，不加入本页项目致谢，除非未来确实吸收了其公开项目设计。
