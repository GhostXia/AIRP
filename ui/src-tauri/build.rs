//! Dev-mode 兜底：杀掉锁文件记录的残留 engine。
//!
//! 场景（仅 dev 构建存在）：壳崩溃后 engine 残留，而残留进程正是
//! `target/<profile>/airp-core.exe`（tauri dev 经 cargo run 构建）。
//! tauri-build 每次构建都会把 sidecar 拷进 target 目录，残留进程锁住
//! 该文件会导致构建失败（PermissionDenied）——自愈逻辑在壳里，但壳
//! 构建不出来，形成死锁。此钩子在 tauri_build::build() 之前清掉残留，
//! 让构建得以继续；壳启动后的三分支探测仍是权威决策。
//!
//! 打包/安装版不受影响：残留锁住的是安装目录 exe，二次打开无需重建。

fn main() {
    kill_leftover_engine_from_lock();
    wait_for_sidecar_unlock();
    tauri_build::build();
}

fn kill_leftover_engine_from_lock() {
    let data_root = match dirs::data_dir() {
        Some(root) => root.join("com.airp.ui").join("data"),
        None => return,
    };
    let lock_path = data_root.join("engine-instance.lock");
    let raw = match std::fs::read_to_string(&lock_path) {
        Ok(raw) => raw,
        Err(_) => return,
    };
    #[derive(serde::Deserialize)]
    struct Lock {
        engine_pid: u32,
    }
    let lock: Lock = match serde_json::from_str(&raw) {
        Ok(lock) => lock,
        Err(_) => return,
    };
    if !is_pid_alive(lock.engine_pid) {
        return;
    }
    println!(
        "cargo:warning=dev-build hook: killing leftover engine pid {} (lock file)",
        lock.engine_pid
    );
    kill_pid(lock.engine_pid);
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

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/nh", "/FI", &format!("PID eq {pid}")])
            .output();
        match output {
            Ok(output) => String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
