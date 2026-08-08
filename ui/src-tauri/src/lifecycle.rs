//! 桌面壳生命周期：防进程残留（C-P0）。
//!
//! 背景：壳以 sidecar 方式拉起 engine。若壳崩溃/被强杀而 engine 残留，
//! 二次打开会撞端口；若残留的是外部 engine（API-only），壳会报
//! "not hosting the WebUI"。本模块用锁文件 + 三分支探测实现自愈：
//!
//! 锁文件 `<data_root>/engine-instance.lock`（JSON，仅本壳写入）：
//! 记录 shell_pid / engine_pid / port / instance_id 的归属关系。
//!
//! 启动探测三分支（纯函数 [`decide_startup`]，所有 I/O 结果注入）：
//! - a) 锁存在且 engine 进程活着 → 我们自启的残留：先杀再拉；
//! - b) 锁存在但进程已死 → 崩溃残留：清理锁后直接拉起；
//! - c) 端口被外部进程占用 → 承载 webui 则复用；否则给可操作提示。
//!
//! 防双开：锁中 shell_pid 活着（且不是本进程）→ 第二实例提示后退出。
//! 退出清理：壳退出时 kill sidecar 并清空归属本实例的锁记录；锁 inode 保留。
//!
//! PID 判定一律走身份探测（[`is_process_running`]）：存活 + 映像名匹配
//! （壳须 airp-ui、engine 须 airp-core），Windows PID 回绕复用不会导致
//! 误杀无关进程或双开误判进入不可启动状态；身份不符视为锁陈旧。

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// 锁文件名（位于 data root 下）。
pub const LOCK_FILE_NAME: &str = "engine-instance.lock";

/// 锁文件记录的两类进程身份（身份探测的期望映像名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRole {
    /// 桌面壳进程（映像名须为 airp-ui）。
    Shell,
    /// engine 进程（映像名须为 airp-core 前缀）。
    Engine,
}

/// 锁文件内容：一次壳实例与其自启 engine 的归属记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceLock {
    /// 壳进程 PID（用于防双开判定）。
    pub shell_pid: u32,
    /// 壳拉起的 engine sidecar PID。
    pub engine_pid: u32,
    /// engine 监听端口。
    pub port: u16,
    /// 壳实例随机标识（退出清理时比对，避免误删他人锁）。
    pub instance_id: String,
}

/// Exclusive startup lock held for one launch sequence.
///
/// The lock is an OS-level exclusive lock on `engine-instance.lock` (flock on
/// POSIX and LockFileEx on Windows, via `std::fs::File::try_lock`).  The file
/// itself remains in place while the guard is held; unlinking a locked path is
/// not portable (on POSIX an unlink would otherwise let a second process create
/// a new inode and bypass the lock).
#[derive(Debug)]
pub struct LockGuard {
    file: File,
}

impl LockGuard {
    /// Read the current owner record while the exclusive lock is held.
    pub fn read_lock(&mut self) -> Option<InstanceLock> {
        read_lock_from_file(&mut self.file)
    }

    /// Replace the owner record in-place and flush it before releasing the
    /// guard.  Keeping the same inode is required for cross-platform locking.
    pub fn write_lock(&mut self, lock: &InstanceLock) -> io::Result<()> {
        let raw = serde_json::to_vec(lock).map_err(io::Error::other)?;
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&raw)?;
        self.file.sync_data()
    }

    /// Clear the owner record while retaining the lock file inode.
    pub fn clear(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.sync_data()
    }
}

/// Acquire the per-data-root startup lock.
///
/// `ErrorKind::WouldBlock` means another shell currently owns the lock.  The
/// caller must keep the returned guard alive through the read → probe → spawn
/// → write sequence; after a successful spawn the durable owner record and PID
/// identity probe continue to reject concurrent launches.
pub fn acquire_lock(path: &Path) -> io::Result<LockGuard> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "engine instance lock is already held",
            ));
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
    Ok(LockGuard { file })
}

/// 启动探测的决策结果。
#[derive(Debug, PartialEq)]
pub enum StartupPlan {
    /// 无锁（或陈旧锁已视为无效）且端口空闲：拉起新 sidecar。
    SpawnFresh,
    /// 分支 a：自启 engine 残留且活着——先杀再拉。
    KillOwnedEngineThenSpawn { engine_pid: u32 },
    /// 分支 c（复用）：端口被外部 engine 占用且其承载 webui——直接连接。
    ReuseExternalHosting,
    /// 分支 c（冲突）：端口被占用但不承载 webui——需要用户介入。
    ConflictExternalPort,
    /// 防双开：另一个壳实例正在运行。
    AnotherShellRunning { shell_pid: u32 },
}

/// 纯决策函数：I/O 探测结果全部由参数注入，便于单元测试。
///
/// `process_running` 是身份探测而非存活探测：实现方须同时校验 PID 存活
/// 与映像名匹配（见 [`is_process_running`]），不匹配视为不存在。
///
/// 注意：返回 `SpawnFresh` 时，调用方仍需 best-effort 清空陈旧锁记录
/// （分支 b 与无锁情形共用该出口；锁 inode 不应被删除）。
pub fn decide_startup(
    lock: Option<&InstanceLock>,
    current_shell_pid: u32,
    process_running: &dyn Fn(u32, ProcessRole) -> bool,
    port_occupied: bool,
    external_hosts_webui: bool,
) -> StartupPlan {
    if let Some(lock) = lock {
        // 防双开优先于一切：另一个壳活着则本实例退出。
        if lock.shell_pid != current_shell_pid
            && process_running(lock.shell_pid, ProcessRole::Shell)
        {
            return StartupPlan::AnotherShellRunning {
                shell_pid: lock.shell_pid,
            };
        }
        // 分支 a：锁归属的 engine 还活着（壳崩溃/被强杀后的残留）——
        // CommandChild 句柄已随旧壳丢失，无法接管，先杀再拉是唯一自愈路径。
        if process_running(lock.engine_pid, ProcessRole::Engine) {
            return StartupPlan::KillOwnedEngineThenSpawn {
                engine_pid: lock.engine_pid,
            };
        }
        // 分支 b：进程都死了，锁是崩溃残留——落到端口探测。
    }
    if port_occupied {
        if external_hosts_webui {
            StartupPlan::ReuseExternalHosting
        } else {
            StartupPlan::ConflictExternalPort
        }
    } else {
        StartupPlan::SpawnFresh
    }
}

/// 读取锁文件；缺失或损坏都视为无锁（损坏锁不应阻断启动）。
pub fn read_lock(path: &Path) -> Option<InstanceLock> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

fn read_lock_from_file(file: &mut File) -> Option<InstanceLock> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut raw = String::new();
    file.read_to_string(&mut raw).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 仅当锁文件归属给定 instance_id 时清空（退出清理防误删他人锁）。
///
/// The same OS lock used during startup is acquired before checking ownership;
/// this closes the read→remove race.  The inode is retained so a POSIX peer
/// cannot unlink-and-recreate the path while the guard is held.
pub fn remove_lock_if_owned(path: &Path, instance_id: &str) {
    if !path.exists() {
        return;
    }
    let mut guard = match acquire_lock(path) {
        Ok(guard) => guard,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
        Err(error) => {
            tracing::warn!(%error, "failed to acquire engine instance lock for cleanup");
            return;
        }
    };
    let owned = guard
        .read_lock()
        .map(|lock| lock.instance_id == instance_id)
        .unwrap_or(false);
    if owned {
        if let Err(error) = guard.clear() {
            tracing::warn!(%error, "failed to clear engine instance lock");
        }
    }
}

/// 进程身份探测：存活 + 映像名匹配，二者缺一即视为不存在。
///
/// 为什么必须核验身份：Windows PID 回绕复用是常态，陈旧锁 + PID 复用
/// 会导致误杀无关进程（engine_pid 被复用）或双开误判进入不可启动
/// 状态（shell_pid 被复用）。映像名不符（如 PID 被无关进程复用）视为
/// 锁陈旧，决策层自然落到自愈路径。
///
/// Windows 用 `tasklist /FI "PID eq N" /FO CSV /NH` 解析映像名；
/// POSIX 读 `/proc/<pid>/comm`（macOS 无 procfs 时退化为仅存活判定）。
pub fn is_process_running(pid: u32, role: ProcessRole) -> bool {
    let expected: &str = match role {
        ProcessRole::Shell => "airp-ui",
        ProcessRole::Engine => "airp-core",
    };
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        match output {
            // CSV 行形如 "Image Name","PID","Session Name",...；无匹配时
            // 输出 "INFO: No tasks are running..."（不含目标 PID）。
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().any(|line| {
                    line.contains(&format!("\"{pid}\"")) && image_matches(line, expected)
                })
            }
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            // comm 为可执行名（截断至 15 字符），前缀匹配 airp-core/airp-ui。
            Ok(comm) => comm.trim().starts_with(expected),
            // 无 procfs（macOS）退化为仅存活判定：宁可保守误判存活，
            // 不因探测工具缺失而阻断启动。
            Err(_) => std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|status| status.success())
                .unwrap_or(false),
        }
    }
}

/// CSV 行的映像名段是否匹配期望前缀（大小写不敏感，容忍 .exe 后缀）。
#[cfg(target_os = "windows")]
fn image_matches(line: &str, expected: &str) -> bool {
    // CSV 第一字段是带引号的映像名，如 "airp-core.exe"。
    line.split(',').next().is_some_and(|field| {
        let name = field.trim_matches('"').to_ascii_lowercase();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        stem.starts_with(&expected.to_ascii_lowercase())
    })
}

/// 终止残留 engine（分支 a 自愈）。归属明确（锁记录），直接强杀。
pub fn kill_pid(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

/// 端口占用探测（loopback 短超时 connect）。
pub fn is_port_occupied(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(300),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(shell_pid: u32, engine_pid: u32, port: u16) -> InstanceLock {
        InstanceLock {
            shell_pid,
            engine_pid,
            port,
            instance_id: format!("test-{shell_pid}-{engine_pid}"),
        }
    }

    /// 构造身份探测闭包：集合内 (pid, role) 视为「活着且身份匹配」。
    /// 模拟真实语义：PID 活着但映像名不符的不在集合内，视为陈旧。
    fn running_set(
        entries: &'static [(u32, ProcessRole)],
    ) -> Box<dyn Fn(u32, ProcessRole) -> bool> {
        Box::new(move |pid, role| entries.contains(&(pid, role)))
    }

    const NONE: &[(u32, ProcessRole)] = &[];

    #[test]
    fn no_lock_free_port_spawns_fresh() {
        let plan = decide_startup(None, 100, &running_set(NONE), false, false);
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn branch_a_owned_engine_alive_is_killed_then_spawned() {
        let lock = lock(100, 200, 8000);
        // 旧壳 100 已死，engine 200（airp-core）残留活着。
        let plan = decide_startup(
            Some(&lock),
            300,
            &running_set(&[(200, ProcessRole::Engine)]),
            false,
            false,
        );
        assert_eq!(
            plan,
            StartupPlan::KillOwnedEngineThenSpawn { engine_pid: 200 }
        );
    }

    #[test]
    fn branch_a_takes_precedence_over_port_state() {
        // engine 残留活着时端口必然被它占用；决策仍走先杀再拉，
        // 不误判为外部冲突。
        let lock = lock(100, 200, 8000);
        let plan = decide_startup(
            Some(&lock),
            300,
            &running_set(&[(200, ProcessRole::Engine)]),
            true,
            false,
        );
        assert_eq!(
            plan,
            StartupPlan::KillOwnedEngineThenSpawn { engine_pid: 200 }
        );
    }

    /// PID 复用场景：engine_pid 活着但映像名不是 airp-core（被无关进程
    /// 复用）→ 视为锁陈旧，不得走先杀再拉（否则误杀无关进程），
    /// 落到端口探测自愈路径。
    #[test]
    fn pid_alive_but_wrong_identity_is_treated_as_stale() {
        let lock = lock(100, 200, 8000);
        // 200 活着但身份不是 Engine（identity 探测返回 false）。
        let plan = decide_startup(Some(&lock), 300, &running_set(NONE), false, false);
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    /// PID 复用场景：shell_pid 被无关进程复用 → 不得误判双开
    /// （否则应用进入不可启动状态），同样落到自愈路径。
    #[test]
    fn shell_pid_reused_by_unrelated_process_does_not_block_startup() {
        let lock = lock(100, 200, 8000);
        // 100 活着但身份不是 Shell：防双开不触发；200 已死：走分支 b。
        let plan = decide_startup(
            Some(&lock),
            300,
            &running_set(&[(100, ProcessRole::Engine)]),
            false,
            false,
        );
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn branch_b_stale_lock_falls_through_to_port_probe() {
        let lock = lock(100, 200, 8000);
        // 壳与 engine 都死了，端口空闲 → 直接拉起。
        let plan = decide_startup(Some(&lock), 300, &running_set(NONE), false, false);
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn branch_c_external_port_hosting_webui_is_reused() {
        let plan = decide_startup(None, 100, &running_set(NONE), true, true);
        assert_eq!(plan, StartupPlan::ReuseExternalHosting);
    }

    #[test]
    fn branch_c_external_port_not_hosting_is_conflict() {
        let plan = decide_startup(None, 100, &running_set(NONE), true, false);
        assert_eq!(plan, StartupPlan::ConflictExternalPort);
    }

    #[test]
    fn double_launch_detected_when_other_shell_alive() {
        let lock = lock(100, 200, 8000);
        let plan = decide_startup(
            Some(&lock),
            300,
            &running_set(&[(100, ProcessRole::Shell), (200, ProcessRole::Engine)]),
            false,
            false,
        );
        assert_eq!(plan, StartupPlan::AnotherShellRunning { shell_pid: 100 });
    }

    #[test]
    fn own_lock_entry_does_not_trigger_double_launch() {
        // 锁里的 shell_pid 就是本进程（异常场景：重启后锁未清理但 pid 复用）
        // 不应误判双开；engine 死了则走分支 b。
        let lock = lock(100, 200, 8000);
        let plan = decide_startup(
            Some(&lock),
            100,
            &running_set(&[(100, ProcessRole::Shell)]),
            false,
            false,
        );
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn lock_roundtrip_and_owned_removal() {
        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);
        let lock = lock(100, 200, 8000);

        assert!(read_lock(&path).is_none());
        let mut guard = acquire_lock(&path).unwrap();
        guard.write_lock(&lock).unwrap();
        assert_eq!(guard.read_lock(), Some(lock.clone()));
        drop(guard);
        assert_eq!(read_lock(&path), Some(lock.clone()));

        // 非归属实例不删除。
        remove_lock_if_owned(&path, "someone-else");
        assert!(read_lock(&path).is_some());
        // 归属实例删除。
        remove_lock_if_owned(&path, &lock.instance_id);
        assert!(read_lock(&path).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn startup_lock_serializes_concurrent_process_claims() {
        let path = std::env::var_os("AIRP_LIFECYCLE_CHILD_LOCK_PATH").map(std::path::PathBuf::from);
        if let Some(path) = path {
            let ready = std::env::var_os("AIRP_LIFECYCLE_CHILD_READY")
                .map(std::path::PathBuf::from)
                .expect("child ready path");
            let release = std::env::var_os("AIRP_LIFECYCLE_CHILD_RELEASE")
                .map(std::path::PathBuf::from)
                .expect("child release path");
            let _guard = acquire_lock(&path).expect("child must acquire lifecycle lock");
            std::fs::write(ready, b"ready").unwrap();
            for _ in 0..250 {
                if release.exists() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("parent did not release child lifecycle lock within timeout");
        }

        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);
        let ready = dir.join("child.ready");
        let release = dir.join("child.release");
        let test_name = "lifecycle::tests::startup_lock_serializes_concurrent_process_claims";
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env("AIRP_LIFECYCLE_CHILD_LOCK_PATH", &path)
            .env("AIRP_LIFECYCLE_CHILD_READY", &ready)
            .env("AIRP_LIFECYCLE_CHILD_RELEASE", &release)
            .spawn()
            .unwrap();

        let mut ready_seen = false;
        for _ in 0..250 {
            if ready.exists() {
                ready_seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !ready_seen {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not publish lock-held readiness within timeout");
        }
        let blocked_kind = match acquire_lock(&path) {
            Err(error) => error.kind(),
            Ok(guard) => {
                drop(guard);
                io::ErrorKind::Other
            }
        };
        if blocked_kind != io::ErrorKind::WouldBlock {
            let _ = std::fs::write(&release, b"release");
            let _ = child.wait();
            panic!("parent was not blocked by child lifecycle lock: {blocked_kind:?}");
        }
        std::fs::write(&release, b"release").unwrap();
        let child_status = child.wait().unwrap();
        assert!(
            child_status.success(),
            "child failed while holding lifecycle lock"
        );

        let reclaimed = acquire_lock(&path).expect("parent reclaims lock after child exits");
        drop(reclaimed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lock_guard_keeps_owner_record_on_same_inode() {
        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);
        let mut guard = acquire_lock(&path).unwrap();
        let owner = lock(100, 200, 8000);
        guard.write_lock(&owner).unwrap();

        assert_eq!(guard.read_lock(), Some(owner.clone()));
        let second = acquire_lock(&path).expect_err("owner record must remain locked");
        assert_eq!(second.kind(), io::ErrorKind::WouldBlock);

        guard.clear().unwrap();
        assert!(guard.read_lock().is_none());
        drop(guard);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn owned_cleanup_retains_lock_inode_for_reuse() {
        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);
        let identity_link = dir.join("engine-instance.identity-link");
        let owner = lock(100, 200, 8000);

        let mut guard = acquire_lock(&path).unwrap();
        guard.write_lock(&owner).unwrap();
        drop(guard);
        // A hard link observes the original inode.  If cleanup unlinked and
        // recreated the path, subsequent writes would diverge from this link.
        std::fs::hard_link(&path, &identity_link).unwrap();

        remove_lock_if_owned(&path, &owner.instance_id);
        assert!(path.exists(), "cleanup must retain the durable lock inode");
        assert!(read_lock(&path).is_none());

        let replacement = lock(300, 400, 8001);
        let mut guard = acquire_lock(&path).unwrap();
        guard.write_lock(&replacement).unwrap();
        drop(guard);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            std::fs::read_to_string(&identity_link).unwrap(),
            "writes through the path must remain visible through the original inode"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removing_owned_lock_for_missing_path_is_a_noop() {
        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);

        remove_lock_if_owned(&path, "missing");
        assert!(!path.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupted_lock_is_treated_as_absent() {
        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);
        std::fs::write(&path, "not json {").unwrap();
        assert!(read_lock(&path).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// tasklist CSV 行的映像名解析：前缀匹配、大小写不敏感、
    /// 容忍 .exe 后缀；不符即视为陈旧。
    #[cfg(target_os = "windows")]
    #[test]
    fn csv_image_name_matching() {
        let engine_line = "\"airp-core.exe\",\"200\",\"Console\",\"1\",\"1,234 K\"";
        assert!(image_matches(engine_line, "airp-core"));
        assert!(!image_matches(engine_line, "airp-ui"));

        let shell_line = "\"AIRP-UI.EXE\",\"100\",\"Console\",\"1\",\"1,234 K\"";
        assert!(image_matches(shell_line, "airp-ui"));

        // PID 被无关进程复用：映像名不符 → 陈旧。
        let unrelated = "\"chrome.exe\",\"200\",\"Console\",\"1\",\"1,234 K\"";
        assert!(!image_matches(unrelated, "airp-core"));

        // engine 的 triple 后缀产物（airp-core-x86_64...）同样命中前缀。
        let triple_line = "\"airp-core-x86_64-pc-windows-msvc.exe\",\"200\",\"0\",\"1\",\"1 K\"";
        assert!(image_matches(triple_line, "airp-core"));
    }
}
