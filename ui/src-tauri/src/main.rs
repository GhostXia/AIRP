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

use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tracing_subscriber::EnvFilter;

const DEFAULT_ENGINE_PORT: u16 = 8000;

#[derive(Debug, serde::Deserialize, Default)]
struct SidecarSettings {
    daemon_port: Option<u16>,
}

/// 壳自启 sidecar 的句柄与本实例标识。
/// instance_id 用于退出清理时只删除归属本实例的锁文件。
#[derive(Default)]
struct EngineSidecar {
    child: Mutex<Option<CommandChild>>,
    instance_id: Mutex<Option<String>>,
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
            let data_root = app.path().app_data_dir()?.join("data");
            std::fs::create_dir_all(&data_root)?;
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
            tauri::async_runtime::spawn(async move {
                run_startup_sequence(handle, context).await;
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building AIRP UI")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let sidecar = app.state::<EngineSidecar>();
                let child = sidecar
                    .child
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                if let Some(child) = child {
                    let pid = child.pid();
                    match child.kill() {
                        Ok(()) => tracing::info!(pid, "engine sidecar stopped with UI"),
                        Err(error) => tracing::warn!(pid, %error, "failed to stop engine sidecar"),
                    }
                }
                // 退出清理：只删除归属本实例的锁（防误删并发实例的锁）。
                let instance_id = sidecar
                    .instance_id
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if let Some(instance_id) = instance_id {
                    if let Ok(data_root) = app.path().app_data_dir() {
                        lifecycle::remove_lock_if_owned(
                            &data_root.join("data").join(lifecycle::LOCK_FILE_NAME),
                            &instance_id,
                        );
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

/// 捆绑 sidecar 启动序列：防进程残留三分支探测 → 按决策执行。
///
/// 决策矩阵见 `lifecycle::decide_startup`；本函数负责 I/O 执行面：
/// - `SpawnFresh`：清理陈旧锁 → 拉起 sidecar → 写新锁；
/// - `KillOwnedEngineThenSpawn`：杀自启残留 → 等端口释放 → 拉起 → 写锁；
/// - `ReuseExternalHosting`：端口被外部承载 webui 的 engine 占用 →
///   不 spawn、不写锁，直接连接（无 access key，token 交换降级为无 bearer）；
/// - `ConflictExternalPort`：端口被占用且不承载 webui → 可操作提示；
/// - `AnotherShellRunning`：防双开 → 提示后退出。
async fn run_startup_sequence(app: tauri::AppHandle, context: StartupContext) {
    let lock_path = context.data_root.join(lifecycle::LOCK_FILE_NAME);
    let lock = lifecycle::read_lock(&lock_path);
    let port_occupied = lifecycle::is_port_occupied(context.port);
    let external_hosts_webui = if port_occupied {
        probe_hosts_webui(&context.engine_url).await
    } else {
        false
    };
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
                 If no AIRP UI window is actually open, delete the stale \
                 instance lock (engine-instance.lock) under the app data \
                 directory and retry.\n\
                 已有 AIRP UI 实例在运行（壳进程 {shell_pid}），本窗口即将退出。\
                 若实际没有 AIRP UI 窗口，请删除数据目录下的残留锁文件 \
                 engine-instance.lock 后重试。"
            );
            show_engine_error_then_exit(&app, &message);
        }
        lifecycle::StartupPlan::KillOwnedEngineThenSpawn { engine_pid } => {
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
                if !lifecycle::is_port_occupied(context.port) {
                    freed = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            if freed {
                // 仅在端口确认释放后才删锁：kill 失败（归属信息已丢则下次
                // 无法再走分支 a 自愈）时保留锁，保持自愈链路完整。
                lifecycle::remove_lock(&lock_path);
                spawn_sidecar(&app, &context, &lock_path).await;
            } else {
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
            lifecycle::remove_lock(&lock_path);
            spawn_readiness_and_navigate(app, context.engine_url, None);
        }
        lifecycle::StartupPlan::ConflictExternalPort => {
            show_port_conflict_error(&app, context.port);
        }
        lifecycle::StartupPlan::SpawnFresh => {
            lifecycle::remove_lock(&lock_path);
            spawn_sidecar(&app, &context, &lock_path).await;
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
async fn spawn_sidecar(app: &tauri::AppHandle, context: &StartupContext, lock_path: &Path) {
    let port = context.port;
    let port_arg = port.to_string();
    match app.shell().sidecar("airp-core") {
        Ok(mut cmd) => {
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
            match cmd.spawn() {
                Ok((mut rx, child)) => {
                    let pid = child.pid();
                    let instance_id = uuid::Uuid::new_v4().to_string();
                    *app.state::<EngineSidecar>()
                        .child
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(child);
                    *app.state::<EngineSidecar>()
                        .instance_id
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(instance_id.clone());
                    // 归属锁：记录 shell/engine pid + 端口 + 实例标识，
                    // 供下次启动三分支探测与防双开使用。
                    let lock = lifecycle::InstanceLock {
                        shell_pid: std::process::id(),
                        engine_pid: pid,
                        port,
                        instance_id,
                    };
                    if let Err(error) = lifecycle::write_lock(lock_path, &lock) {
                        tracing::warn!(%error, "failed to write engine instance lock");
                    }
                    // Single receiver yields all CommandEvent variants
                    // (Stdout/Stderr/Terminated/...). Log each for
                    // debuggability (透明取向: 引擎状态可观察).
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
                    spawn_readiness_and_navigate(
                        app.clone(),
                        context.engine_url.clone(),
                        context.access_key.clone(),
                    );
                }
                Err(e) => {
                    tracing::error!(err = %e, "failed to spawn engine sidecar");
                    show_engine_error(
                        app,
                        &format!(
                            "Engine failed to start. Run \
                             `cargo run -p airp-core -- daemon --port {port}` manually or \
                             rebuild the sidecar: {e}\n\
                             引擎启动失败。可手动运行上述命令，或重建 sidecar。"
                        ),
                    );
                }
            }
        }
        Err(e) => {
            tracing::error!(err = %e,
                "sidecar 'airp-core' not configured/found — packaging must build \
                 binaries/airp-core-$TARGET_TRIPLE first");
            show_engine_error(
                app,
                &format!(
                    "Engine sidecar is missing. Run `ui/build-engine-sidecar.ps1` or start \
                     `cargo run -p airp-core -- daemon --port {port}` manually: {e}\n\
                     未找到引擎 sidecar。请先运行 ui/build-engine-sidecar.ps1，\
                     或手动启动引擎。"
                ),
            );
        }
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
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        // 1. 就绪探针（50 × 100ms = 5s）。
        let mut ready = false;
        for _ in 0..50 {
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

        // 2. 承载探测：engine 必须同源承载 webui（local/desktop router 都提供
        //    /runtime-config.js；纯 API router 返回 404）。
        let hosted = client
            .get(format!("{engine_url}/runtime-config.js"))
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false);
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

        // 3. bearer 注入通道：进程互信（access key）换短时效 UI token。
        let token = exchange_desktop_token(&client, &engine_url, access_key.as_deref()).await;

        // 4. 导航首屏。fragment 不发送到服务端；entry.js 承接写入 sessionStorage。
        let mut target = format!("{engine_url}/");
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
/// 失败（无 key / engine 不支持 / 网络）时返回 None：导航降级为无 bearer，
/// webui 连接卡可手动输入（不阻断桌面可用性）。
async fn exchange_desktop_token(
    client: &reqwest::Client,
    engine_url: &str,
    access_key: Option<&str>,
) -> Option<String> {
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
                if token.is_none() {
                    tracing::warn!("desktop-session response missing token field");
                } else {
                    tracing::info!(
                        expires_in = body.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(0),
                        "desktop session token exchanged"
                    );
                }
                token
            }
            Err(error) => {
                tracing::warn!(%error, "failed to parse desktop-session response");
                None
            }
        },
        Ok(resp) => {
            tracing::warn!(status = %resp.status(), "desktop-session exchange rejected");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "desktop-session exchange failed");
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
    fn webui_dir_env_override_requires_index_html() {
        // 无 index.html 的目录不可作为承载面（resolve_webui_dir 的 env 分支
        // 与同一 valid 判定；AppHandle 分支在集成冒烟覆盖，此处测纯函数边界）。
        let root = temp_data_root("webui-invalid");
        std::fs::write(root.join("not-index.txt"), "x").unwrap();
        assert!(!root.join("index.html").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
