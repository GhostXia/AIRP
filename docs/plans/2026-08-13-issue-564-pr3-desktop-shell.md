# #564 PR 3：桌面 Vue shell 与 WebUI 设计语言

> 状态：PR 3 shell 实现记录。基线：`main@b2091b2`（PR #572 合并后）。
>
> 本 PR 只交付可访问、可响应的桌面 shell、固定测试 Surface 和浏览器截图门禁；
> 不声称 Blueprint v2 renderer、Engine Surface API、HttpEngineBus、`/desktop/` 或
> Tauri 默认入口已交付。

## 1. 设计方向

桌面端定位为“叙事工作台”，而不是通用聊天 WebUI 的另一层壳：

- 左侧是稳定的领域工作区 rail：Story / World / Director / Debug；
- 中央只承载当前动态 Surface；
- 右侧 Context Inspector 绑定当前工作区，并可折叠；
- Focus Mode 收窄导航并移除 Inspector，保留轻量故事路径；
- 持久状态条明确标注当前是固定协议预览，不用 toast 或 MockBus 冒充成功。

视觉直接导入 `webui/assets/tokens.css`，不复制 WebUI DOM/CSS。桌面只新增字体角色、
三栏尺寸和 shell 语义 token。唯一强调结构是贯穿品牌标记、当前工作区与 Surface 标题的
陶土色“章节脊线”；其余表面保持低装饰、细描边和高信息密度。

## 2. 交互与可访问性

- 工作区导航使用 `aria-current="page"`，方向键循环切换，Home/End 直达首尾；
- Inspector toggle 使用 `aria-expanded`；Focus Mode 使用 `aria-pressed`；
- 所有图标按钮具有可访问名称和可见 `focus-visible`；
- `prefers-reduced-motion` 将动画/过渡压缩到近零；
- 1180px 以下收窄 rail/Inspector，760px 以下隐藏 Inspector，不产生页面横向滚动；
- 失败状态为持久 banner，说明发生了什么并提供“重试”动作。

## 3. 固定 Surface 边界

PR 3 在浏览器中只渲染 `SHELL_PREVIEW_BLUEPRINT` 和显式 fixture state，用于验证 shell
几何与 Widget 容器。浏览器不再自动构建 MockBus；Tauri 中仍暂时保留旧 transport，直到
PR 6 的 `HttpEngineBus` 与双入口替换它。固定 fixture 的文字明确说明真实 Engine 投影属于
PR 5，避免把 preview 写成运行能力。

## 4. 自动化证据

`ui/desktop-shell-smoke.mjs` 启动 Vite 并用系统 Chrome 验证：

- 1024×768、1440×900、1920×1080；
- Windows 125% 与 150% device scale；
- 工作区语义、键盘方向切换、按钮 accessible name；
- document/shell 横向 overflow；
- page error 与非空截图像素证据。

PR gate 新增 `npm run build`、`npm run smoke:shell` 和 30 天截图 artifact 上传。截图输出
在 `ui/dist/shell-smoke/`，不作为源码资产提交。

## 5. 后续边界

PR 4 才实现 Blueprint v2 递归 renderer、实例生命周期与生产原子 store；PR 5 才建立
Engine Surface projection/API；PR 6 才提供真实 HttpEngineBus、同源 `/desktop/` 和
Tauri 双入口。因此本 PR 不改变当前正式 WebUI 或 Tauri 默认导航。
