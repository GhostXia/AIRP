//! AIRP desktop shell (C-P0).
//!
//! 壳的职责（获批计划 C-P0 / 集成架构研究 §2.5）：
//! 1. sidecar 生命周期：spawn `airp-core daemon`、健康探针、退出即 kill；
//! 2. data root（`AIRP_DATA_DIR`）与 per-user 目录；
//! 3. 进程级随机 access key 注入 sidecar（`AIRP_ACCESS_KEY`，绝不落盘）；
//! 4. 同源 webui 承载：把 webui 目录经 `AIRP_DESKTOP_WEBUI_DIR` 交给 daemon，
//!    webview 内容面即 engine 同源 webui（与浏览器宿主跑同一份资产）；
//! 5. bearer 注入通道：engine 就绪后，壳持 access key 调
//!    `POST /v1/desktop-session` 换短时效 UI token，以 URL fragment
//!    （`#airp-token=...`）导航首屏；fragment 不进服务端日志/Referer，
//!    首屏 `webui/assets/entry.js` 写入 `sessionStorage.airp_bearer` 后清理；
//! 6. 防进程残留（lifecycle 模块）：锁文件 + 三分支探测 + 防双开 + 退出清理，
//!    二次打开自愈，用户无需手动杀进程；
//! 7. 原生能力：窗口/对话框（tauri-plugin-dialog）。
//!
//! 历史：Phase 0~B 的 `bus.rs` intent relay（6 个 intent 手工转发 /v1）与
//! Vue 主面已被 webui 直连 REST+SSE 完全覆盖，随 C-P0 归档到
//! `docs/archive/2026-08-04-c-p0-desktop-shell/`（BUG-3/4/5 随之消除）。

mod lifecycle;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::async_runtime::JoinHandle;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tracing_subscriber::EnvFilter;

/// 8765 与 deploy/windows-webui/Start-AIRP.cmd、deploy/linux-webui/start-airp.sh
/// 及 README 中面向用户的端口一致；壳读 settings.json 的 daemon_port 缺省时
/// 用此值，避免桌面入口（8765）与浏览器入口（cmd 硬编码 8765）端口错位
/// （审计 N-02）。
const DEFAULT_ENGINE_PORT: u16 = 8765;

#[derive(Debug, serde::Deserialize, Default)]
struct SidecarSettings {
    daemon_port: Option<u16>,
}

/// 一对一的已发布 sidecar 句柄与归属标识。
///
/// 发布和接管必须成对发生，避免 child 与 instance_id 暴露半发布状态。
struct PublishedSidecar<C> {
    child: Option<C>,
    instance_id: Option<String>,
}

impl<C> Default for PublishedSidecar<C> {
    fn default() -> Self {
        Self {
            child: None,
            instance_id: None,
        }
    }
}

impl<C> PublishedSidecar<C> {
    fn publish(&mut self, child: C, instance_id: String) {
        self.child = Some(child);
        self.instance_id = Some(instance_id);
    }

    fn take(&mut self) -> (Option<C>, Option<String>) {
        (self.child.take(), self.instance_id.take())
    }
}

/// 壳自启 sidecar 的生命周期状态。
///
/// `published`、shutdown flags 和启动任务必须由同一把锁协调：退出请求
/// 要么先把启动任务标为停止并等待它完成，要么在它完成发布后再接管句柄；
/// 不能让两个独立 Mutex 暴露出半发布状态。
struct SidecarState<C> {
    shutting_down: bool,
    shutdown_complete: bool,
    published: PublishedSidecar<C>,
    startup: Option<JoinHandle<()>>,
}

impl<C> Default for SidecarState<C> {
    fn default() -> Self {
        Self {
            shutting_down: false,
            shutdown_complete: false,
            published: PublishedSidecar::default(),
            startup: None,
        }
    }
}

impl<C> SidecarState<C> {
    fn can_spawn(&self) -> bool {
        !self.shutting_down && !self.shutdown_complete
    }

    fn request_shutdown(&mut self) -> ExitRequest {
        if self.shutdown_complete {
            ExitRequest::Allow
        } else if self.shutting_down {
            ExitRequest::Prevent
        } else {
            self.shutting_down = true;
            ExitRequest::Start(self.startup.take())
        }
    }

    fn publish(&mut self, child: C, instance_id: String) {
        self.published.publish(child, instance_id);
    }

    fn take_published(&mut self) -> (Option<C>, Option<String>) {
        self.published.take()
    }
}

enum SpawnTransaction<T, E> {
    Rejected,
    Published(T),
    Failed(E),
}

/// Execute the sidecar spawn/publish transaction under one state lock.
///
/// The action owns all work between the can-spawn check and publication.  A
/// caller cannot observe a successful child before its owner id is published,
/// nor can an exit request acquire the state lock between those operations.
fn run_spawn_transaction<C, T, E, F>(
    state: &Mutex<SidecarState<C>>,
    action: F,
) -> SpawnTransaction<T, E>
where
    F: FnOnce() -> Result<(C, String, T), E>,
{
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if !state.can_spawn() {
        return SpawnTransaction::Rejected;
    }
    match action() {
        Ok((child, instance_id, value)) => {
            state.publish(child, instance_id);
            SpawnTransaction::Published(value)
        }
        Err(error) => SpawnTransaction::Failed(error),
    }
}

enum ExitRequest {
    Allow,
    Prevent,
    Start(Option<JoinHandle<()>>),
}

/// 壳自启 sidecar 的状态与数据根目录。
/// data_root 供退出清理复用 setup 期解析结果（便携包体模式下不等于
/// %APPDATA%，退出清理必须使用同一路径才能清对 owner 记录）。
#[derive(Default)]
struct EngineSidecar {
    state: Mutex<SidecarState<CommandChild>>,
    data_root: Mutex<Option<PathBuf>>,
}

/// 捆绑 sidecar 启动序列所需的全部上下文（setup 同步段解析，
/// 异步段消费；所有字段不可变，跨线程安全）。
struct StartupContext {
    data_root: PathBuf,
    engine_url: String,
    port: u16,
    access_key: Option<String>,
    webui_dir: Option<PathBuf>,
}

fn main() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(EngineSidecar::default())
        .setup(|app| {
            let data_root = resolve_data_root(app.handle())?;
            std::fs::create_dir_all(&data_root)?;
            // 供 RunEvent::Exit 清理锁时复用同一数据根（便携包体模式下
            // 数据根在包内 data/，不在 %APPDATA%，不可重新推导）。
            *app.state::<EngineSidecar>()
                .data_root
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(data_root.clone());
            let (port, configured_access_key) = load_sidecar_settings(&data_root);
            let env_engine_url = std::env::var("AIRP_ENGINE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty());
            // 捆绑本地 sidecar 一律获得进程级随机 bearer。只有壳进程与
            // 子进程可见；绝不持久化到 settings.json。
            let access_key = if env_engine_url.is_none() {
                configured_access_key.or_else(|| Some(uuid::Uuid::new_v4().to_string()))
            } else {
                configured_access_key
            };
            let engine_url = env_engine_url
                .clone()
                .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
            let engine_url = engine_url.trim_end_matches('/').to_string();
            let webui_dir = resolve_webui_dir(app.handle());

            tracing::info!(
                data_root = %data_root.display(),
                engine_url = %engine_url,
                has_access_key = access_key.is_some(),
                webui_dir = webui_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "engine connection configured"
            );

            if let Some(env_url) = env_engine_url {
                // 外部 engine：不 spawn sidecar、不写锁（不归我们所有），
                // 但仍走同一套就绪→承载探测→导航链路（engine 未承载 webui
                // 时给出可操作错误而非白屏）。
                tracing::info!(engine_url = %env_url,
                    "AIRP_ENGINE_URL is set; skipping bundled sidecar spawn");
                spawn_readiness_and_navigate(app.handle().clone(), engine_url, access_key);
                return Ok(());
            }

            // 捆绑 sidecar 路径：防进程残留探测（锁文件 + 端口 + 承载探测）
            // 含网络/进程 I/O，放异步任务，避免阻塞 setup。
            let context = StartupContext {
                data_root,
                engine_url,
                port,
                access_key,
                webui_dir,
            };
            let handle = app.handle().clone();
            let startup = tauri::async_runtime::spawn(async move {
                run_startup_sequence(handle, context).await;
            });
            app.state::<EngineSidecar>()
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .startup = Some(startup);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building AIRP UI")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                let request = app
                    .state::<EngineSidecar>()
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .request_shutdown();
                match request {
                    ExitRequest::Allow => {}
                    ExitRequest::Prevent => api.prevent_exit(),
                    ExitRequest::Start(startup) => {
                        api.prevent_exit();
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            finish_shutdown(app, startup, code.unwrap_or(0)).await;
                        });
                    }
                }
            }
        });
}

fn init_tracing() {
    let filter = std::env::var("AIRP_UI_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn load_sidecar_settings(data_root: &Path) -> (u16, Option<String>) {
    let mut port = DEFAULT_ENGINE_PORT;
    let mut access_key = None;

    let settings_path = data_root.join("settings.json");
    if settings_path.exists() {
        match std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<SidecarSettings>(&raw).ok())
        {
            Some(settings) => {
                if let Some(value) = settings.daemon_port {
                    port = value;
                }
            }
            None => tracing::warn!(
                path = %settings_path.display(),
                "failed to parse sidecar settings; using defaults"
            ),
        }
    }

    if let Ok(value) = std::env::var("AIRP_DAEMON_PORT") {
        match value.parse::<u16>() {
            Ok(value) => port = value,
            Err(e) => tracing::warn!(err = %e, value = %value, "invalid AIRP_DAEMON_PORT"),
        }
    }
    if let Ok(value) = std::env::var("AIRP_ACCESS_KEY") {
        if !value.is_empty() {
            access_key = Some(value);
        }
    }

    (port, access_key)
}

/// 数据根目录解析（v0.0.4：桌面壳与 webui 便携包体共存、共用包内目录）。
///
/// 优先级：
/// 1. `AIRP_DATA_DIR` 环境变量（显式覆盖，与 webui 便携包 Start-AIRP.cmd
///    的包内数据语义同源）；
/// 2. 便携包体模式：exe 同目录同时存在 `airp-core.exe` 与 `webui/index.html`
///    （包体标记）时使用同目录 `data/`（airp-ui.exe 与 Start-AIRP.cmd
///    共用一个解压目录，角色卡/会话/密钥等用户数据从首次启动即两侧一致）；
/// 3. 默认 `%APPDATA%/<identifier>/data`（开发/安装场景，C-P0 原语义）。
fn resolve_data_root(app: &tauri::AppHandle) -> std::io::Result<PathBuf> {
    if let Ok(dir) = std::env::var("AIRP_DATA_DIR") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    if let Some(portable) = portable_data_dir() {
        return Ok(portable);
    }
    let dir = app.path().app_data_dir().map_err(std::io::Error::other)?;
    Ok(dir.join("data"))
}

/// 便携包体模式：可执行文件同目录同时存在捆绑 sidecar（`airp-core.exe`）
/// 与同源承载资产（`webui/index.html`）即视为与 webui 便携包共存，
/// 数据根使用包内 `data/`（目录由 setup 的 create_dir_all 补齐——全新
/// 解压包首次双击桌面端即命中，角色/会话与 webui 入口从第一次就共享）。
/// 判定只认包体标记，不认副作用目录（空目录/解压工具丢弃不可改变判定）。
fn portable_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    portable_data_dir_from(exe.parent()?)
}

fn portable_data_dir_from(exe_dir: &Path) -> Option<PathBuf> {
    let dir = exe_dir.join("data");
    if exe_dir.join("airp-core.exe").is_file() && exe_dir.join("webui").join("index.html").is_file()
    {
        Some(dir)
    } else {
        None
    }
}

/// C-P0: 定位 webui 资产目录（engine 同源承载的内容面）。
///
/// 优先级：
/// 1. `AIRP_WEBUI_DIR` 环境变量（显式覆盖）；
/// 2. 打包资源目录 `<resource_dir>/webui`（或 bundle-webui.ps1 暂存的
///    `<resource_dir>/webui-bundle`，见 tauri.conf.json bundle.resources；
///    dev 模式下 tauri-build 会把 resources 拷到 `target/<profile>/`，
///    resource_dir 即指向那里，内容与仓库 webui/ 等价）；
/// 3. 从可执行文件向上回溯（开发检出：target/debug|release 上 2~3 级即仓库根）。
///
/// 每个候选都要求包含 `index.html`，避免把错误目录交给 daemon。
fn resolve_webui_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let valid = |candidate: PathBuf| -> Option<PathBuf> {
        candidate.join("index.html").is_file().then_some(candidate)
    };

    if let Ok(dir) = std::env::var("AIRP_WEBUI_DIR") {
        if !dir.trim().is_empty() {
            if let Some(found) = valid(PathBuf::from(dir.trim())) {
                return Some(found);
            }
            tracing::warn!("AIRP_WEBUI_DIR is set but does not contain index.html");
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        for name in ["webui", "webui-bundle"] {
            if let Some(found) = valid(resource_dir.join(name)) {
                return Some(found);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(6) {
            if let Some(found) = valid(ancestor.join("webui")) {
                return Some(found);
            }
        }
    }
    tracing::warn!(
        "webui directory not found; engine will run API-only and the shell \
         cannot show the same-origin WebUI"
    );
    None
}

fn is_shutting_down(app: &tauri::AppHandle) -> bool {
    app.state::<EngineSidecar>()
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .shutting_down
}

fn clear_owner_after_successful_stop(
    lock_path: &Path,
    instance_id: Option<String>,
    kill_succeeded: bool,
) {
    if !kill_succeeded {
        return;
    }
    if let Some(instance_id) = instance_id {
        lifecycle::remove_lock_if_owned(lock_path, &instance_id);
    }
}

/// 等待脱离 setup 的启动任务完成，再接管已发布的 child/id。
///
/// 退出请求先阻止事件循环；startup 任务在返回前会释放生命周期锁，
/// 因此这里随后获取锁时不会遇到「启动任务仍持锁」的假失败。
async fn finish_shutdown(app: tauri::AppHandle, startup: Option<JoinHandle<()>>, exit_code: i32) {
    if let Some(startup) = startup {
        if let Err(error) = startup.await {
            tracing::warn!(%error, "engine startup task failed while shutting down");
        }
    }

    let (child, instance_id) = {
        let sidecar = app.state::<EngineSidecar>();
        let mut state = sidecar
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.take_published()
    };
    let data_root = app
        .state::<EngineSidecar>()
        .data_root
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();

    let killed = match child {
        Some(child) => {
            let pid = child.pid();
            match child.kill() {
                Ok(()) => {
                    tracing::info!(pid, "engine sidecar stopped with UI");
                    true
                }
                Err(error) => {
                    // Keep the durable owner record.  A later startup can still
                    // identify and retry this owned process instead of losing
                    // the only recovery handle.
                    tracing::warn!(pid, %error, "failed to stop engine sidecar; preserving owner record");
                    false
                }
            }
        }
        None => true,
    };

    if let Some(data_root) = data_root {
        clear_owner_after_successful_stop(
            &data_root.join(lifecycle::LOCK_FILE_NAME),
            instance_id,
            killed,
        );
    }

    app.state::<EngineSidecar>()
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .shutdown_complete = true;
    app.exit(exit_code);
}

/// 捆绑 sidecar 启动序列：防进程残留三分支探测 → 按决策执行。
///
/// 决策矩阵见 `lifecycle::decide_startup`；本函数负责 I/O 执行面：
/// - `SpawnFresh`：持有原子启动锁 → 拉起 sidecar → 写新锁；
/// - `KillOwnedEngineThenSpawn`：杀自启残留 → 等端口释放 → 拉起 → 写锁；
/// - `ReuseExternalHosting`：端口被外部承载 webui 的 engine 占用 →
///   不 spawn、不写锁，直接连接（无 access key，token 交换降级为无 bearer）；
/// - `ConflictExternalPort`：端口被占用且不承载 webui → 可操作提示；
/// - `AnotherShellRunning`：防双开 → 提示后退出。
async fn run_startup_sequence(app: tauri::AppHandle, context: StartupContext) {
    if is_shutting_down(&app) {
        return;
    }
    let lock_path = context.data_root.join(lifecycle::LOCK_FILE_NAME);
    // The OS lock must be acquired before *any* lifecycle probe.  Otherwise
    // two shells can both observe a stale/absent record and spawn sidecars
    // before either one writes its owner record (last-writer-wins).
    let mut lock_guard = match lifecycle::acquire_lock(&lock_path) {
        Ok(guard) => guard,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            let shell_pid = lifecycle::read_lock(&lock_path).map(|lock| lock.shell_pid);
            let message = match shell_pid {
                Some(shell_pid) => format!(
                    "AIRP UI is already running or starting (shell PID {shell_pid}). \
                     This second instance will now exit.\n\
                     已有 AIRP UI 实例正在运行或启动（壳进程 {shell_pid}），\
                     本窗口即将退出。"
                ),
                None => "AIRP UI is already starting in another process. This second instance \
                         will now exit.\n另一个进程正在启动 AIRP UI，本窗口即将退出。"
                    .to_string(),
            };
            show_engine_error_then_exit(&app, &message);
            return;
        }
        Err(error) => {
            show_engine_error(
                &app,
                &format!(
                    "AIRP UI could not acquire its startup lock: {error}\n\
                     AIRP UI 无法取得启动锁，请检查数据目录权限后重试。"
                ),
            );
            return;
        }
    };
    if is_shutting_down(&app) {
        drop(lock_guard);
        return;
    }
    let lock = lock_guard.read_lock();
    let port_occupied = lifecycle::is_port_occupied(context.port);
    let external_hosts_webui = if port_occupied {
        probe_hosts_webui(&context.engine_url).await
    } else {
        false
    };
    if is_shutting_down(&app) {
        drop(lock_guard);
        return;
    }
    let plan = lifecycle::decide_startup(
        lock.as_ref(),
        std::process::id(),
        &lifecycle::is_process_running,
        port_occupied,
        external_hosts_webui,
    );
    tracing::info!(
        has_lock = lock.is_some(),
        port_occupied,
        external_hosts_webui,
        plan = ?plan,
        "startup probe decision"
    );

    match plan {
        lifecycle::StartupPlan::AnotherShellRunning { shell_pid } => {
            // exit 必须挪进对话框关闭回调：非阻塞 show + 立即退出会让
            // 窗口一闪而过，用户看不到任何解释（双开是最需要解释的路径）。
            // 文案附锁文件位置，给 PID 复用等异常场景留自助出路。
            let message = format!(
                "AIRP UI is already running (shell PID {shell_pid}). \
                 This second instance will now exit. \
                 If no AIRP UI window is actually open, clear the stale owner \
                 record (or delete engine-instance.lock) under the app data \
                 directory and retry.\n\
                 已有 AIRP UI 实例在运行（壳进程 {shell_pid}），本窗口即将退出。\
                 若实际没有 AIRP UI 窗口，请删除数据目录下的残留锁文件 \
                 engine-instance.lock 后重试。"
            );
            if is_shutting_down(&app) {
                drop(lock_guard);
                return;
            }
            drop(lock_guard);
            show_engine_error_then_exit(&app, &message);
        }
        lifecycle::StartupPlan::KillOwnedEngineThenSpawn { engine_pid } => {
            if is_shutting_down(&app) {
                drop(lock_guard);
                return;
            }
            tracing::info!(engine_pid, "killing leftover owned engine before respawn");
            if !lifecycle::kill_pid(engine_pid) {
                tracing::warn!(
                    engine_pid,
                    "failed to kill leftover engine; port probe follows"
                );
            }
            // 等端口真正释放（最多 3 秒），否则落回冲突提示而不是硬撞。
            let mut freed = false;
            for _ in 0..30 {
                if is_shutting_down(&app) {
                    drop(lock_guard);
                    return;
                }
                if !lifecycle::is_port_occupied(context.port) {
                    freed = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if is_shutting_down(&app) {
                    drop(lock_guard);
                    return;
                }
            }
            if freed {
                // Keep the same locked inode while replacing the owner record;
                // unlinking here would let a POSIX peer bypass flock.
                spawn_sidecar(&app, &context, lock_guard).await;
            } else {
                // Preserve the old owner record for the next startup's
                // self-healing retry when the engine did not release the port.
                if is_shutting_down(&app) {
                    drop(lock_guard);
                    return;
                }
                drop(lock_guard);
                show_port_conflict_error(&app, context.port);
            }
        }
        lifecycle::StartupPlan::ReuseExternalHosting => {
            // 外部 engine 已在承载 webui：直接复用（不 spawn、不写锁——
            // 进程不归我们所有，退出时也不得 kill）。
            tracing::info!(
                port = context.port,
                "port occupied by an external engine hosting the WebUI; reusing it"
            );
            if is_shutting_down(&app) {
                drop(lock_guard);
                return;
            }
            if let Err(error) = lock_guard.clear() {
                tracing::warn!(%error, "failed to clear stale engine instance lock");
            }
            drop(lock_guard);
            spawn_readiness_and_navigate(app, context.engine_url, None);
        }
        lifecycle::StartupPlan::ConflictExternalPort => {
            if is_shutting_down(&app) {
                drop(lock_guard);
                return;
            }
            if let Err(error) = lock_guard.clear() {
                tracing::warn!(%error, "failed to clear stale engine instance lock");
            }
            drop(lock_guard);
            show_port_conflict_error(&app, context.port);
        }
        lifecycle::StartupPlan::SpawnFresh => {
            spawn_sidecar(&app, &context, lock_guard).await;
        }
    }
}

/// 承载探测：GET /runtime-config.js（local/desktop router 提供，纯 API 404）。
async fn probe_hosts_webui(engine_url: &str) -> bool {
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client
            .get(format!("{engine_url}/runtime-config.js"))
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// 拉起 engine sidecar 并在成功后写入归属锁。
///
/// sidecar 二进制位于 `binaries/airp-core-$TARGET_TRIPLE`（tauri.conf.json
/// externalBin；tauri-plugin-shell 按平台后缀解析）。
/// AIRP_DATA_DIR 让打包构建使用 per-user data root；
/// AIRP_DESKTOP_WEBUI_DIR（C-P0）让 daemon 以 desktop router 同源承载
/// webui（允许 access key 鉴权，与 CLI --webui-dir 互斥约束无关）。
async fn spawn_sidecar(
    app: &tauri::AppHandle,
    context: &StartupContext,
    lock_guard: lifecycle::LockGuard,
) {
    let port = context.port;
    let port_arg = port.to_string();
    let sidecar = app.state::<EngineSidecar>();
    let mut lock_guard = Some(lock_guard);
    let transaction = run_spawn_transaction(&sidecar.state, || {
        let mut lock_guard = lock_guard
            .take()
            .expect("spawn transaction must own its lifecycle lock");
        let mut cmd = match app.shell().sidecar("airp-core") {
            Ok(cmd) => cmd,
            Err(error) => {
                tracing::error!(err = %error,
                    "sidecar 'airp-core' not configured/found — packaging must build \
                     binaries/airp-core-$TARGET_TRIPLE first");
                if let Err(clear_error) = lock_guard.clear() {
                    tracing::warn!(%clear_error, "failed to clear engine instance lock after spawn failure");
                }
                drop(lock_guard);
                return Err(format!(
                    "Engine sidecar is missing. Run `ui/build-engine-sidecar.ps1` or start \
                     `cargo run -p airp-core -- daemon --port {port}` manually: {error}\n\
                     未找到引擎 sidecar。请先运行 ui/build-engine-sidecar.ps1，\
                     或手动启动引擎。"
                ));
            }
        };
        cmd = cmd
            .args(["daemon", "--port", port_arg.as_str()])
            .current_dir(&context.data_root)
            .env("AIRP_DATA_DIR", &context.data_root)
            .env("AIRP_ALLOW_LOCAL_PATH", "1");
        if let Some(ref access_key) = context.access_key {
            cmd = cmd.env("AIRP_ACCESS_KEY", access_key);
        }
        if let Some(ref webui_dir) = context.webui_dir {
            cmd = cmd.env("AIRP_DESKTOP_WEBUI_DIR", webui_dir);
        }

        let (rx, child) = match cmd.spawn() {
            Ok(result) => result,
            Err(error) => {
                tracing::error!(err = %error, "failed to spawn engine sidecar");
                if let Err(clear_error) = lock_guard.clear() {
                    tracing::warn!(%clear_error, "failed to clear engine instance lock after spawn failure");
                }
                drop(lock_guard);
                return Err(format!(
                    "Engine failed to start. Run \
                     `cargo run -p airp-core -- daemon --port {port}` manually or \
                     rebuild the sidecar: {error}\n\
                     引擎启动失败。可手动运行上述命令，或重建 sidecar。"
                ));
            }
        };
        let pid = child.pid();
        let instance_id = uuid::Uuid::new_v4().to_string();
        // 归属锁：记录 shell/engine pid + 端口 + 实例标识，供下次启动三分支探测。
        let lock = lifecycle::InstanceLock {
            shell_pid: std::process::id(),
            engine_pid: pid,
            port,
            instance_id,
        };
        if let Err(error) = lock_guard.write_lock(&lock) {
            tracing::error!(%error, "failed to write engine instance lock");
            if let Err(kill_error) = child.kill() {
                tracing::warn!(%kill_error, "failed to stop engine after lock write failure");
            }
            if let Err(clear_error) = lock_guard.clear() {
                tracing::warn!(%clear_error, "failed to clear engine instance lock after spawn failure");
            }
            drop(lock_guard);
            return Err(format!(
                "Engine started but AIRP UI could not persist its instance lock: {error}\n\
                 引擎已启动，但 AIRP UI 无法写入实例锁，已停止该引擎。"
            ));
        }

        // Release the OS lock before returning; run_spawn_transaction then
        // publishes child/id while its state mutex is still held.
        let instance_id = lock.instance_id.clone();
        drop(lock_guard);
        Ok((child, instance_id, (rx, pid)))
    });
    if let Some(lock_guard) = lock_guard {
        // Rejected transactions never invoke the action; release the
        // lifecycle lock after the state mutex has been released.
        drop(lock_guard);
    }

    let (mut rx, pid) = match transaction {
        SpawnTransaction::Rejected => return,
        SpawnTransaction::Failed(message) => {
            show_engine_error(app, &message);
            return;
        }
        SpawnTransaction::Published((rx, pid)) => (rx, pid),
    };

    // Single receiver yields all CommandEvent variants (Stdout/Stderr/
    // Terminated/...). Log each for debuggability.
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                CommandEvent::Stdout(b) => tracing::info!(
                    target: "airp-core",
                    "engine: {}", String::from_utf8_lossy(&b).trim_end()),
                CommandEvent::Stderr(b) => tracing::warn!(
                    target: "airp-core",
                    "engine err: {}", String::from_utf8_lossy(&b).trim_end()),
                CommandEvent::Terminated(p) => tracing::warn!(
                    target: "airp-core",
                    "engine sidecar terminated: {:?}", p),
                _ => {}
            }
        }
    });
    tracing::info!(
        port = port,
        pid,
        data_root = %context.data_root.display(),
        "engine sidecar spawned"
    );
    if !is_shutting_down(app) {
        spawn_readiness_and_navigate(
            app.clone(),
            context.engine_url.clone(),
            context.access_key.clone(),
        );
    }
}

/// engine 就绪 → webui 承载探测 → bearer 交换 → 首屏导航。
///
/// 任何一步失败都给出明确的原生错误提示，而不是让 webview 停在白屏：
/// - 就绪超时：sidecar 未起来；
/// - 承载探测失败（GET /runtime-config.js 非 200）：engine 是 API-only
///   （webui 目录缺失，或连上了未开启承载的外部/遗留 engine 实例）；
/// - token 交换失败：仍导航，但不带 fragment——webui 设置屏的连接卡可
///   手动补 bearer（短时效 token 机制见 engine `daemon::desktop_session`）。
fn spawn_readiness_and_navigate(
    app: tauri::AppHandle,
    engine_url: String,
    access_key: Option<String>,
) {
    tauri::async_runtime::spawn(async move {
        if is_shutting_down(&app) {
            return;
        }
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        // 1. 就绪探针（50 × 100ms = 5s）。
        let mut ready = false;
        for _ in 0..50 {
            if is_shutting_down(&app) {
                return;
            }
            if client
                .get(format!("{engine_url}/version"))
                .send()
                .await
                .map(|resp| resp.status().is_success())
                .unwrap_or(false)
            {
                ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if is_shutting_down(&app) {
                return;
            }
        }
        if !ready {
            tracing::error!(engine_url = %engine_url, "engine did not become ready");
            show_engine_error(
                &app,
                &format!(
                    "Engine did not become ready at {engine_url} within 5 seconds. \
                     Check the engine logs, or whether another program is holding the port. \
                     引擎 5 秒内未就绪：请查看引擎日志，或检查端口是否被其他程序占用。"
                ),
            );
            return;
        }
        tracing::info!(engine_url = %engine_url, "engine ready");
        if is_shutting_down(&app) {
            return;
        }

        // 2. 承载探测：engine 必须同源承载 webui（local/desktop router 都提供
        //    /runtime-config.js；纯 API router 返回 404）。
        let hosted = client
            .get(format!("{engine_url}/runtime-config.js"))
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false);
        if is_shutting_down(&app) {
            return;
        }
        if !hosted {
            show_engine_error(
                &app,
                &format!(
                    "The engine at {engine_url} is running, but it is not serving the AIRP \
                     WebUI (it responds API-only). This usually means an older or externally \
                     started engine instance is occupying this port.\n\
                     Next steps (pick one):\n\
                     1. Close the other engine instance and reopen AIRP UI;\n\
                     2. Or set the AIRP_DAEMON_PORT environment variable to a free port \
                     and reopen AIRP UI;\n\
                     3. Or start the engine with WebUI hosting yourself, e.g. \
                     `cargo run -p airp-core -- daemon --webui-dir <repo>/webui`.\n\n\
                     该端口的引擎未承载 WebUI（API-only 模式），通常是旧版或外部启动的\
                     引擎实例占用了端口。处理：关闭该实例后重开 AIRP UI；或设置 \
                     AIRP_DAEMON_PORT 换端口；或手动以 --webui-dir 承载模式启动引擎。"
                ),
            );
            return;
        }

        let blueprint_requested =
            desktop_ui_path(std::env::var("AIRP_DESKTOP_UI").ok().as_deref()) == "/desktop/";
        let blueprint_available = if blueprint_requested {
            client
                .get(format!("{engine_url}/desktop/"))
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false)
        } else {
            false
        };
        let entry_path = if blueprint_requested && blueprint_available {
            "/desktop/"
        } else {
            if blueprint_requested {
                tracing::warn!("Vue desktop bundle unavailable; falling back to WebUI");
                show_engine_error(
                    &app,
                    "The Blueprint desktop UI was requested but its bundle could not be opened. \
                     AIRP is falling back to the supported WebUI.\n\n\
                     Blueprint 桌面界面缺失或启动失败，已回退到 WebUI。",
                );
            }
            "/"
        };

        // 3. bearer 注入通道：进程互信（access key）换短时效 UI token。
        let exchanged =
            exchange_desktop_token(&client, &engine_url, access_key.as_deref(), true).await;
        if is_shutting_down(&app) {
            return;
        }
        let token = exchanged.as_ref().map(|(token, _)| token.clone());

        // 4. 导航首屏。fragment 不发送到服务端；entry.js 承接写入 sessionStorage。
        let mut target = format!("{engine_url}{entry_path}");
        if let Some(ref token) = token {
            target.push_str("#airp-token=");
            target.push_str(token);
        }
        match target.parse::<url::Url>() {
            Ok(url) => {
                if let Some(window) = app.get_webview_window("main") {
                    if let Err(error) = window.navigate(url) {
                        tracing::error!(%error, "failed to navigate main window to WebUI");
                        show_engine_error(&app, &format!("Failed to open the WebUI: {error}"));
                    } else {
                        // C-P2：导航成功后启动 token 续期循环，避免 8h TTL
                        // 边界会话锁死（C-P1 交接项 / issue #479）。
                        if let Some((_token, expires_in)) = exchanged {
                            spawn_token_renewal_loop(
                                app.clone(),
                                engine_url.clone(),
                                access_key.clone(),
                                expires_in,
                            );
                        }
                    }
                } else {
                    tracing::error!("main window disappeared before WebUI navigation");
                }
            }
            Err(error) => {
                tracing::error!(%error, url = %target, "invalid WebUI navigation URL");
                show_engine_error(&app, &format!("Invalid WebUI URL: {error}"));
            }
        }
    });
}

fn desktop_ui_path(value: Option<&str>) -> &'static str {
    if value == Some("blueprint") {
        "/desktop/"
    } else {
        "/"
    }
}

/// C-P2：desktop session token 主动续期循环。
///
/// 策略：按返回的 `expires_in / 2` 调度重新交换（access key 换新 token）。
/// 注意：壳这里故意用 exchange（只增不撤）而非 renew（rotation）——
/// 若壳做 rotation，webui 尚持有的旧 token 会被立即撤销，两边互踢；
/// 代价是旧 token 在自身 TTL 内仍有效（取舍见 C-P2 审查 S2）。
/// 成功后经 `webview.eval()` 推送新 bearer 到 `sessionStorage.airp_bearer`
/// 并 dispatch `airp-bearer-renewed`——api-client 的函数形态 bearer 在
/// 下次请求即取新值。
///
/// 为何不走 Tauri IPC：webview 运行在 `http://127.0.0.1:<port>` 远程 URL
/// （engine 同源承载），无 Tauri IPC 通道；动态端口也无法枚举进
/// dangerousRemoteUrlIpcAccess。eval 注入是同进程单向推送，不引入新攻击面。
///
/// 失败处理（W2）：交换失败置 `failed_fast` 标志，下轮等待切 60s 短间隔
/// （而非先睡 60s 再回循环顶等半个 TTL——恰在循环该发挥作用的重试
/// 时刻失灵）；成功后恢复 expires_in/2 节奏。webui 侧撞 401 另有
/// `POST /v1/desktop-session/renew` 兜底（rotation），双保险。
/// 失败退避（T1）：连续失败按 60s 起指数退避（60s · 2^(n-1)，封顶 15min）；
/// 日志降级——仅首次失败 warn，连续失败期间 debug，防止 engine 长时间
/// 宕机时每轮一条 warn 刷满日志。
fn spawn_token_renewal_loop(
    app: tauri::AppHandle,
    engine_url: String,
    access_key: Option<String>,
    mut expires_in: u64,
) {
    if access_key.is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        // W2：failed_fast —— 上一轮交换失败时下轮只等 60s 短间隔。
        let mut failed_fast = false;
        // T1（#485）：连续失败计数 —— 退避 60s * 2^(n-1) 封顶 15min；日志仅
        // 首次失败 warn，连续失败期间降级 debug（engine 宕机时不刷 warn）。
        let mut consecutive_failures: u32 = 0;
        let retry_wait =
            |failures: u32| -> u64 { (60u64 << failures.saturating_sub(1).min(4)).min(900) };
        loop {
            if is_shutting_down(&app) {
                return;
            }
            // TTL 过半即续期；下限 5s 防短 TTL 冒烟时热循环，上限 4h 防
            // engine 返回异常大值时续期完全停摆；失败后按连续次数指数退避。
            let wait_secs = if failed_fast {
                retry_wait(consecutive_failures)
            } else {
                (expires_in / 2).clamp(5, 4 * 3600)
            };
            tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
            if is_shutting_down(&app) {
                return;
            }
            match exchange_desktop_token(
                &client,
                &engine_url,
                access_key.as_deref(),
                consecutive_failures == 0,
            )
            .await
            {
                Some((new_token, new_expires_in)) => {
                    if is_shutting_down(&app) {
                        return;
                    }
                    failed_fast = false;
                    consecutive_failures = 0;
                    if let Some(window) = app.get_webview_window("main") {
                        // JSON.stringify 转义 token（uuid hex 本无特殊字符，
                        // 防御性处理）；eval 失败（窗口已销毁/导航中）仅留痕。
                        let script = format!(
                            "try {{ sessionStorage.setItem('airp_bearer', {}); \
                             window.dispatchEvent(new CustomEvent('airp-bearer-renewed', \
                             {{ detail: {{ expires_in: {new_expires_in} }} }})); }} catch (e) {{}}",
                            serde_json::to_string(&new_token).unwrap_or_else(|_| "''".to_string())
                        );
                        if let Err(error) = window.eval(&script) {
                            tracing::warn!(%error, "failed to push renewed bearer into webview");
                        } else {
                            tracing::info!(
                                expires_in = new_expires_in,
                                "desktop session token renewed and pushed to webview"
                            );
                        }
                    } else {
                        tracing::warn!("main window gone; stopping token renewal loop");
                        return;
                    }
                    expires_in = new_expires_in;
                }
                None => {
                    // 交换失败（engine 重启/网络抖动）：置标志，下轮按连续失败
                    // 次数指数退避（60s 起，封顶 15min）；webui 侧 401 另有
                    // renew 端点兜底。日志降级：仅首次失败 warn，连续期间 debug。
                    failed_fast = true;
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let next_wait = retry_wait(consecutive_failures);
                    if consecutive_failures == 1 {
                        tracing::warn!(
                            retry_in_secs = next_wait,
                            "desktop session renewal exchange failed; will retry"
                        );
                    } else {
                        tracing::debug!(
                            retries = consecutive_failures,
                            retry_in_secs = next_wait,
                            "desktop session renewal exchange still failing"
                        );
                    }
                }
            }
        }
    });
}

/// 端口冲突（外部进程占用且不承载 webui）的可操作提示。
fn show_port_conflict_error(app: &tauri::AppHandle, port: u16) {
    show_engine_error(
        app,
        &format!(
            "Port {port} is already in use by another program, and that program is not \
             serving the AIRP WebUI.\n\
             Next steps (pick one):\n\
             1. Close the program occupying port {port} (it may be a leftover AIRP engine \
             from a previous session), then reopen AIRP UI;\n\
             2. Or set the AIRP_DAEMON_PORT environment variable to a free port and \
             reopen AIRP UI.\n\n\
             端口 {port} 已被其他程序占用且未承载 AIRP WebUI。处理：关闭占用端口的程序\
             （可能是上次会话遗留的 AIRP 引擎）后重开；或设置 AIRP_DAEMON_PORT 换端口。"
        ),
    );
}

/// 持 access key 调 `POST /v1/desktop-session` 换短时效 UI token。
/// 返回 `(token, expires_in_secs)`；失败（无 key / engine 不支持 / 网络）
/// 时返回 None：导航降级为无 bearer，webui 连接卡可手动输入（不阻断桌面
/// 可用性）。expires_in 缺失时保守按 60s（宁可多续一次，不长期裸奔）。
/// `log_failure_at_warn`：续期循环按连续失败次数降级日志（首次失败 warn、
/// 后续 debug），避免 engine 长时间宕机时每次重试刷多条 warn（审计 #518）。
async fn exchange_desktop_token(
    client: &reqwest::Client,
    engine_url: &str,
    access_key: Option<&str>,
    log_failure_at_warn: bool,
) -> Option<(String, u64)> {
    let key = access_key?;
    let response = client
        .post(format!("{engine_url}/v1/desktop-session"))
        .bearer_auth(key)
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(body) => {
                let token = body
                    .get("token")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                match token {
                    Some(token) => {
                        let expires_in = body
                            .get("expires_in")
                            .and_then(|v| v.as_u64())
                            .filter(|v| *v > 0)
                            .unwrap_or(60);
                        tracing::info!(expires_in, "desktop session token exchanged");
                        Some((token, expires_in))
                    }
                    None => {
                        if log_failure_at_warn {
                            tracing::warn!("desktop-session response missing token field");
                        } else {
                            tracing::debug!("desktop-session response missing token field");
                        }
                        None
                    }
                }
            }
            Err(error) => {
                if log_failure_at_warn {
                    tracing::warn!(%error, "failed to parse desktop-session response");
                } else {
                    tracing::debug!(%error, "failed to parse desktop-session response");
                }
                None
            }
        },
        Ok(resp) => {
            if log_failure_at_warn {
                tracing::warn!(status = %resp.status(), "desktop-session exchange rejected");
            } else {
                tracing::debug!(status = %resp.status(), "desktop-session exchange rejected");
            }
            None
        }
        Err(error) => {
            if log_failure_at_warn {
                tracing::warn!(%error, "desktop-session exchange failed");
            } else {
                tracing::debug!(%error, "desktop-session exchange failed");
            }
            None
        }
    }
}

fn show_engine_error(app: &tauri::AppHandle, message: &str) {
    // 原生对话框（tauri-plugin-dialog Rust API）：不依赖 webview 内容面，
    // 启动失败路径上也能把原因说清楚（透明取向）。文案要求可操作：
    // 每条失败路径都给出明确的下一步（见各调用点）。
    app.dialog()
        .message(message.to_string())
        .title("AIRP engine error")
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

/// 展示错误对话框并在用户关闭后才退出进程（防双开路径专用）。
/// 对话框属于本进程，先 exit 再 show 会直接销毁窗口；exit 必须放在
/// 关闭回调里，用户才能读到解释。
fn show_engine_error_then_exit(app: &tauri::AppHandle, message: &str) {
    let app = app.clone();
    app.dialog()
        .message(message.to_string())
        .title("AIRP UI")
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(move |_| {
            app.exit(0);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_mutex_serializes_publish_before_shutdown() {
        use std::sync::{Arc, Barrier, Condvar, Mutex, TryLockError};

        let state = Arc::new(Mutex::new(SidecarState::<u32>::default()));
        let entered_publish = Arc::new(Barrier::new(2));
        let release_gate = Arc::new((Mutex::new(false), Condvar::new()));

        let startup_state = Arc::clone(&state);
        let startup_barrier = Arc::clone(&entered_publish);
        let startup_gate = Arc::clone(&release_gate);
        let startup = std::thread::spawn(move || {
            let transaction = run_spawn_transaction(&startup_state, || {
                startup_barrier.wait();
                let (gate, wake) = &*startup_gate;
                let mut released = gate.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
                Ok::<_, ()>((7, "instance-7".to_string(), ()))
            });
            assert!(matches!(transaction, SpawnTransaction::Published(())));
        });

        entered_publish.wait();
        assert!(matches!(state.try_lock(), Err(TryLockError::WouldBlock)));

        let exit_state = Arc::clone(&state);
        let exit = std::thread::spawn(move || {
            let mut state = exit_state.lock().unwrap();
            let first = state.request_shutdown();
            let taken = state.take_published();
            let second = state.request_shutdown();
            (first, taken, second, state.can_spawn())
        });

        let (gate, wake) = &*release_gate;
        *gate.lock().unwrap() = true;
        wake.notify_one();
        startup.join().unwrap();

        let (first, (child, instance_id), second, can_spawn) = exit.join().unwrap();
        assert!(matches!(first, ExitRequest::Start(None)));
        assert_eq!(child, Some(7));
        assert_eq!(instance_id.as_deref(), Some("instance-7"));
        assert!(matches!(second, ExitRequest::Prevent));
        assert!(!can_spawn);
    }

    #[test]
    fn exit_first_rejects_later_spawn_on_same_state_mutex() {
        use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Barrier, Mutex};

        let state = Arc::new(Mutex::new(SidecarState::<u32>::default()));
        let ready = Arc::new(Barrier::new(2));
        let exit_state = Arc::clone(&state);
        let exit_ready = Arc::clone(&ready);
        let exit = std::thread::spawn(move || {
            let mut state = exit_state.lock().unwrap();
            assert!(state.can_spawn());
            exit_ready.wait();
            assert!(matches!(state.request_shutdown(), ExitRequest::Start(None)));
            assert!(!state.can_spawn());
        });
        ready.wait();
        exit.join().unwrap();

        let spawn_state = Arc::clone(&state);
        let spawn = std::thread::spawn(move || {
            let state = spawn_state.lock().unwrap();
            state.can_spawn()
        });
        assert!(!spawn.join().unwrap());

        let closure_executed = Arc::new(AtomicBool::new(false));
        let closure_flag = Arc::clone(&closure_executed);
        let transaction = run_spawn_transaction(&state, move || {
            closure_flag.store(true, Ordering::SeqCst);
            Ok::<_, ()>((1, "must-not-publish".to_string(), ()))
        });
        assert!(matches!(transaction, SpawnTransaction::Rejected));
        assert!(!closure_executed.load(Ordering::SeqCst));
    }

    #[test]
    fn owner_cleanup_only_clears_after_successful_stop() {
        let root = temp_data_root("owner-cleanup");
        let path = root.join(lifecycle::LOCK_FILE_NAME);
        let owner = lifecycle::InstanceLock {
            shell_pid: 100,
            engine_pid: 200,
            port: 8000,
            instance_id: "instance-1".to_string(),
        };
        let mut guard = lifecycle::acquire_lock(&path).unwrap();
        guard.write_lock(&owner).unwrap();
        drop(guard);

        clear_owner_after_successful_stop(&path, Some(owner.instance_id.clone()), false);
        assert_eq!(lifecycle::read_lock(&path), Some(owner.clone()));

        clear_owner_after_successful_stop(&path, Some(owner.instance_id.clone()), true);
        assert!(lifecycle::read_lock(&path).is_none());
        assert!(path.exists(), "owner cleanup must retain the lock inode");
        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_data_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("airp-ui-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn sidecar_settings_default_when_missing() {
        let root = temp_data_root("missing-settings");
        let (port, access_key) = load_sidecar_settings(&root);
        assert_eq!(port, DEFAULT_ENGINE_PORT);
        assert_eq!(access_key, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sidecar_settings_reads_port_but_not_plaintext_access_key() {
        let root = temp_data_root("settings");
        std::fs::write(
            root.join("settings.json"),
            r#"{"daemon_port": 8123, "access_api_key": "secret"}"#,
        )
        .unwrap();
        let (port, access_key) = load_sidecar_settings(&root);
        assert_eq!(port, 8123);
        assert_eq!(access_key, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn portable_data_dir_requires_package_markers() {
        let root = temp_data_root("portable");
        // 无包体标记 → 非便携。
        assert_eq!(portable_data_dir_from(&root), None);
        // 只有 data/ 目录 → 仍非便携（不能凭副作用目录判定，
        // 否则全新解压包首次启动会回落 %APPDATA%，与共享数据目标冲突）。
        std::fs::create_dir_all(root.join("data")).unwrap();
        assert_eq!(portable_data_dir_from(&root), None);
        // 包体标记齐备（airp-core.exe + webui/index.html）→ 便携；
        // data/ 由 setup 的 create_dir_all 补齐。
        std::fs::write(root.join("airp-core.exe"), b"x").unwrap();
        std::fs::create_dir_all(root.join("webui")).unwrap();
        std::fs::write(root.join("webui").join("index.html"), b"x").unwrap();
        assert_eq!(portable_data_dir_from(&root), Some(root.join("data")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn webui_dir_env_override_requires_index_html() {
        // 无 index.html 的目录不可作为承载面（resolve_webui_dir 的 env 分支
        // 与同一 valid 判定；AppHandle 分支在集成冒烟覆盖，此处测纯函数边界）。
        let root = temp_data_root("webui-invalid");
        std::fs::write(root.join("not-index.txt"), "x").unwrap();
        assert!(!root.join("index.html").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_ui_switch_is_explicit_and_defaults_to_webui() {
        assert_eq!(desktop_ui_path(None), "/");
        assert_eq!(desktop_ui_path(Some("")), "/");
        assert_eq!(desktop_ui_path(Some("unknown")), "/");
        assert_eq!(desktop_ui_path(Some("blueprint")), "/desktop/");
    }
}
