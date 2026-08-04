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
//! 退出清理：壳退出时 kill sidecar 并删除归属本实例的锁文件。

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// 锁文件名（位于 data root 下）。
pub const LOCK_FILE_NAME: &str = "engine-instance.lock";

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
/// 注意：返回 `SpawnFresh` 时，调用方仍需 best-effort 删除陈旧锁文件
/// （分支 b 与无锁情形共用该出口）。
pub fn decide_startup(
    lock: Option<&InstanceLock>,
    current_shell_pid: u32,
    pid_alive: &dyn Fn(u32) -> bool,
    port_occupied: bool,
    external_hosts_webui: bool,
) -> StartupPlan {
    if let Some(lock) = lock {
        // 防双开优先于一切：另一个壳活着则本实例退出。
        if lock.shell_pid != current_shell_pid && pid_alive(lock.shell_pid) {
            return StartupPlan::AnotherShellRunning {
                shell_pid: lock.shell_pid,
            };
        }
        // 分支 a：锁归属的 engine 还活着（壳崩溃/被强杀后的残留）——
        // CommandChild 句柄已随旧壳丢失，无法接管，先杀再拉是唯一自愈路径。
        if pid_alive(lock.engine_pid) {
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

/// 写入锁文件（原子性要求不高：单写者，且读取侧容忍损坏）。
pub fn write_lock(path: &Path, lock: &InstanceLock) -> io::Result<()> {
    let raw = serde_json::to_string(lock).map_err(io::Error::other)?;
    std::fs::write(path, raw)
}

/// Best-effort 删除锁文件（不存在不算错误）。
pub fn remove_lock(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(%error, "failed to remove engine instance lock"),
    }
}

/// 仅当锁文件归属给定 instance_id 时删除（退出清理防误删他人锁）。
pub fn remove_lock_if_owned(path: &Path, instance_id: &str) {
    let owned = read_lock(path)
        .map(|lock| lock.instance_id == instance_id)
        .unwrap_or(false);
    if owned {
        remove_lock(path);
    }
}

/// 进程存活探测。Windows 用 tasklist，POSIX 用 `kill -0`。
/// 探测失败（工具不可用等）保守返回 false：宁可多拉一次，不误判双开。
pub fn is_pid_alive(pid: u32) -> bool {
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

    /// 构造 pid_alive 闭包：仅指定集合内的 pid 视为活着。
    fn alive_set(pids: &'static [u32]) -> Box<dyn Fn(u32) -> bool> {
        Box::new(move |pid| pids.contains(&pid))
    }

    #[test]
    fn no_lock_free_port_spawns_fresh() {
        let plan = decide_startup(None, 100, &alive_set(&[]), false, false);
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn branch_a_owned_engine_alive_is_killed_then_spawned() {
        let lock = lock(100, 200, 8000);
        // 旧壳 100 已死，engine 200 残留活着。
        let plan = decide_startup(Some(&lock), 300, &alive_set(&[200]), false, false);
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
        let plan = decide_startup(Some(&lock), 300, &alive_set(&[200]), true, false);
        assert_eq!(
            plan,
            StartupPlan::KillOwnedEngineThenSpawn { engine_pid: 200 }
        );
    }

    #[test]
    fn branch_b_stale_lock_falls_through_to_port_probe() {
        let lock = lock(100, 200, 8000);
        // 壳与 engine 都死了，端口空闲 → 直接拉起。
        let plan = decide_startup(Some(&lock), 300, &alive_set(&[]), false, false);
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn branch_c_external_port_hosting_webui_is_reused() {
        let plan = decide_startup(None, 100, &alive_set(&[]), true, true);
        assert_eq!(plan, StartupPlan::ReuseExternalHosting);
    }

    #[test]
    fn branch_c_external_port_not_hosting_is_conflict() {
        let plan = decide_startup(None, 100, &alive_set(&[]), true, false);
        assert_eq!(plan, StartupPlan::ConflictExternalPort);
    }

    #[test]
    fn double_launch_detected_when_other_shell_alive() {
        let lock = lock(100, 200, 8000);
        let plan = decide_startup(Some(&lock), 300, &alive_set(&[100, 200]), false, false);
        assert_eq!(plan, StartupPlan::AnotherShellRunning { shell_pid: 100 });
    }

    #[test]
    fn own_lock_entry_does_not_trigger_double_launch() {
        // 锁里的 shell_pid 就是本进程（异常场景：重启后锁未清理但 pid 复用）
        // 不应误判双开；engine 死了则走分支 b。
        let lock = lock(100, 200, 8000);
        let plan = decide_startup(Some(&lock), 100, &alive_set(&[100]), false, false);
        assert_eq!(plan, StartupPlan::SpawnFresh);
    }

    #[test]
    fn lock_roundtrip_and_owned_removal() {
        let dir = std::env::temp_dir().join(format!("airp-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOCK_FILE_NAME);
        let lock = lock(100, 200, 8000);

        assert!(read_lock(&path).is_none());
        write_lock(&path, &lock).unwrap();
        assert_eq!(read_lock(&path), Some(lock.clone()));

        // 非归属实例不删除。
        remove_lock_if_owned(&path, "someone-else");
        assert!(read_lock(&path).is_some());
        // 归属实例删除。
        remove_lock_if_owned(&path, &lock.instance_id);
        assert!(read_lock(&path).is_none());
        // 重复删除不报错。
        remove_lock(&path);

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
}
