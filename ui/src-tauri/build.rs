//! Dev-mode 兜底钩子 + 构建自洽（tauri_build::build() 之前执行）。
//!
//! 1. 构建自洽（B1 修复）：tauri.conf.json 的 `resources: ["webui-bundle/**/*"]`
//!    要求 `src-tauri/webui-bundle/` 存在且非空（glob crate 的 `dir/**` 只匹配
//!    目录、tauri-utils 对空目录报 GlobPathNotFound）。此处缺失时自行从仓库
//!    根 `webui/` 暂存（与 ui/bundle-webui.ps1 同一语义），全新 CI runner 上
//!    `cargo test/clippy/doc` 编译 airp-ui 不再依赖外部前置步骤。
//!
//! 2. 残留清理（仅 dev 构建相关）：壳崩溃后 engine 残留，而残留进程正是
//!    `target/<profile>/airp-core.exe`（tauri dev 经 cargo run 构建）。
//!    tauri-build 每次构建都会把 sidecar 拷进 target 目录，残留进程锁住
//!    该文件会导致构建失败（PermissionDenied）——自愈逻辑在壳里，但壳
//!    构建不出来，形成死锁。此钩子在构建前清掉残留，让构建得以继续；
//!    壳启动后的三分支探测仍是权威决策。
//!
//!    杀判定三重保守（W1/W2 修复）：
//!    - 锁中 shell_pid 活着 → 有实例正在运行，绝不是残留，直接返回
//!      （否则开发者开着应用时任何编译都会杀掉运行中的 engine）；
//!    - engine_pid 必须通过身份探测（映像名 airp-core 前缀）才杀，
//!      PID 被无关进程复用时视为陈旧锁，绝不误杀；
//!    - 探测失败一律不杀（宁可构建时报 sharing violation，不误杀进程）。
//!
//! 打包/安装版不受残留清理影响：残留锁住的是安装目录 exe，二次打开无需重建。

fn main() {
    stage_webui_bundle();
    kill_leftover_engine_from_lock();
    wait_for_sidecar_unlock();
    tauri_build::build();
}

// ---------------------------------------------------------------------------
// 构建自洽：确保 webui-bundle 暂存目录存在（缺失时从仓库 webui/ 拷贝）
// ---------------------------------------------------------------------------

fn stage_webui_bundle() {
    let manifest_dir = match std::env::var_os("CARGO_MANIFEST_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => return,
    };
    let bundle_dir = manifest_dir.join("webui-bundle");
    // 已存在且非空：视为已暂存（打包流水线的 beforeBuildCommand 会用
    // bundle-webui.ps1 刷新；构建钩子不重复拷贝，避免拖慢增量编译）。
    if dir_has_files(&bundle_dir) {
        return;
    }
    let source = manifest_dir
        .parent() // ui/
        .and_then(|ui_dir| ui_dir.parent()) // 仓库根
        .map(|repo_root| repo_root.join("webui"));
    let source = match source {
        Some(source) if source.join("index.html").is_file() => source,
        _ => {
            // 无仓库 webui 源（如独立发布构建）：建空目录并告警。
            // tauri-build 对空目录会报 GlobPathNotFound——该场景下打包
            // 必须自行暂存，这里至少把「目录缺失」收敛为明确告警。
            let _ = std::fs::create_dir_all(&bundle_dir);
            println!(
                "cargo:warning=webui source not found; created empty webui-bundle/ \
                 staging dir. Packaging must stage real assets via ui/bundle-webui.ps1."
            );
            return;
        }
    };
    match copy_dir_recursive(&source, &bundle_dir) {
        Ok(count) => println!(
            "cargo:warning=staged webui bundle for tauri resources ({} files) -> {}",
            count,
            bundle_dir.display()
        ),
        Err(error) => {
            let _ = std::fs::create_dir_all(&bundle_dir);
            println!(
                "cargo:warning=failed to stage webui bundle: {error}; \
                 created empty dir (tauri-build may fail on the resources glob)"
            );
        }
    }
}

fn dir_has_files(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        })
        .unwrap_or(false)
}

fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut count = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if file_type.is_dir() {
            count += copy_dir_recursive(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dest)?;
            count += 1;
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// 残留清理：锁文件驱动的 dev 兜底（壳活着绝不杀 + 身份探测）
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Lock {
    shell_pid: u32,
    engine_pid: u32,
}

/// 纯判定函数：是否应杀掉锁记录的 engine（I/O 探测结果注入，可单测）。
///
/// 语义：壳活着 = 有实例在运行 = 非残留，绝不杀（W1）；
/// engine 必须「活着且身份匹配 airp-core」才杀（W2，PID 复用不误杀）；
/// 探测失败（工具缺失返回 false）宁可让构建报 sharing violation，不误杀。
fn should_kill_engine(
    shell_pid: u32,
    engine_pid: u32,
    is_running: &dyn Fn(u32, &str) -> bool,
) -> bool {
    if is_running(shell_pid, "airp-ui") {
        return false;
    }
    is_running(engine_pid, "airp-core")
}

fn kill_leftover_engine_from_lock() {
    // dirs::data_dir() 在 Windows 上是 %APPDATA%（Roaming），与 tauri 的
    // app_data_dir 一致（冒烟实证：壳写入的锁位于 Roaming\com.airp.ui\data）。
    let data_root = match dirs::data_dir() {
        Some(root) => root.join("com.airp.ui").join("data"),
        None => return,
    };
    let lock_path = data_root.join("engine-instance.lock");
    let raw = match std::fs::read_to_string(&lock_path) {
        Ok(raw) => raw,
        Err(_) => return,
    };
    let lock: Lock = match serde_json::from_str(&raw) {
        Ok(lock) => lock,
        Err(_) => return,
    };
    if !should_kill_engine(lock.shell_pid, lock.engine_pid, &is_process_running) {
        return;
    }
    println!(
        "cargo:warning=dev-build hook: killing leftover engine pid {} \
         (shell pid {} is not running; lock file)",
        lock.engine_pid, lock.shell_pid
    );
    kill_pid(lock.engine_pid);
}

/// 进程身份探测：PID 存活 + 映像名匹配（前缀，大小写不敏感）。
/// Windows 用 tasklist CSV，POSIX 读 /proc/<pid>/comm；探测失败返回 false。
fn is_process_running(pid: u32, expected_prefix: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().any(|line| {
                    line.contains(&format!("\"{pid}\"")) && image_matches(line, expected_prefix)
                })
            }
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            Ok(comm) => comm.trim().starts_with(expected_prefix),
            Err(_) => std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|status| status.success())
                .unwrap_or(false),
        }
    }
}

/// CSV 行的映像名段是否匹配期望前缀（与 lifecycle.rs 同语义）。
#[cfg(target_os = "windows")]
fn image_matches(line: &str, expected: &str) -> bool {
    line.split(',').next().is_some_and(|field| {
        let name = field.trim_matches('"').to_ascii_lowercase();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        stem.starts_with(&expected.to_ascii_lowercase())
    })
}

/// Windows 下进程退出后文件锁释放需要时间；循环重试直到 sidecar 目标
/// 可写（或超时），避免 tauri-build 拷贝时的 sharing violation。
fn wait_for_sidecar_unlock() {
    let manifest_dir = match std::env::var_os("CARGO_MANIFEST_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => return,
    };
    let triple = match std::env::var_os("TARGET") {
        Some(triple) => triple.to_string_lossy().into_owned(),
        None => return,
    };
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let file_name = format!("airp-core-{triple}{ext}");
    let mut candidates = vec![manifest_dir.join("binaries").join(&file_name)];
    // tauri-build 会把 sidecar 拷到 target/<profile>/（dest 也可能被锁）；
    // dev 模式 sidecar 实际运行的也可能是 workspace 的 airp-core 产物。
    // OUT_DIR = target/<profile>/build/<pkg>/out，上三级即 target/<profile>。
    if let Some(out_dir) = std::env::var_os("OUT_DIR") {
        let target_dir = std::path::Path::new(&out_dir)
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent());
        if let Some(target_dir) = target_dir {
            candidates.push(target_dir.join(&file_name));
            candidates.push(target_dir.join(format!("airp-core{ext}")));
        }
    }
    for _ in 0..40 {
        if candidates.iter().all(|path| try_open_write(path)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    println!(
        "cargo:warning=dev-build hook: sidecar file still locked after waiting: {}",
        candidates
            .iter()
            .filter(|path| !try_open_write(path))
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn try_open_write(path: &std::path::Path) -> bool {
    if !path.exists() {
        return true;
    }
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

fn kill_pid(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
}

// 注：cargo 不会执行 build.rs 内的单元测试（build script 不是测试目标），
// 故 should_kill_engine / image_matches 的决策语义在 lifecycle.rs 的同名
// 逻辑处有单测覆盖（is_process_running / decide_startup / csv_image_name_matching），
// 本文件的实装面通过 W1/W2 冒烟取证（构建日志断言不出现 kill warning）。
