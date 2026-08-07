//! Trusted Plugin 生命周期（#498 §6.3）：spawn 与终止。
//!
//! daemon 启动时 spawn 全部合法 manifest 的子进程；daemon 退出时终止
//! （Unix：SIGTERM → 等 5s → SIGKILL；Windows 无 SIGTERM 语义，直接强杀）。
//! 不做自动重启（§6.7：崩溃 = 用户重启 daemon）。

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

use tokio::process::Child;

use super::{resolve_args, resolve_command, TrustedPluginManifest};

/// 逐个 spawn 全部 manifest；单个失败（目录缺失 / 命令越界 / 端口冲突等）
/// warn 并跳过，不阻塞其余插件（§6.5 端口冲突在 spawn 时暴露）。
pub async fn spawn_all(
    manifests: &[TrustedPluginManifest],
    data_root: &Path,
) -> HashMap<String, Child> {
    let mut children = HashMap::new();
    for manifest in manifests {
        match spawn_one(manifest, data_root).await {
            Ok(child) => {
                tracing::info!(
                    id = %manifest.id,
                    port = manifest.port,
                    "trusted plugin spawned"
                );
                children.insert(manifest.id.clone(), child);
            }
            Err(e) => tracing::warn!(id = %manifest.id, %e, "trusted plugin spawn failed, skipped"),
        }
    }
    children
}

async fn spawn_one(manifest: &TrustedPluginManifest, data_root: &Path) -> Result<Child, String> {
    let command = resolve_command(data_root, manifest)?;
    let args = resolve_args(manifest);
    let plugin_dir = data_root.join("plugins").join(&manifest.id);
    let mut cmd = tokio::process::Command::new(&command);
    cmd.args(&args)
        // 子进程 cwd = 插件目录：args 里的相对路径（如 "server.js"）
        // 以插件目录为基准，而不是 daemon 的 cwd。
        .current_dir(&plugin_dir)
        // 不 env_clear：trusted plugin 是用户显式装的，允许它读自己的环境
        // （区别于 plugin_tool 的零信任脚本，见 #498 §6.3）。
        .env("AIRP_PLUGIN_PORT", manifest.port.to_string())
        .env("AIRP_DATA_ROOT", data_root)
        .env("AIRP_PLUGIN_ID", &manifest.id)
        // 长跑服务：stdout/stderr 直接进 daemon 终端（piped 会因缓冲满
        // 阻塞子进程写日志）。
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    cmd.spawn()
        .map_err(|e| format!("spawn `{}` failed: {e}", command.display()))
}

/// daemon 退出时终止全部子进程。
pub async fn terminate_all(children: &mut HashMap<String, Child>) {
    for (id, child) in children.iter_mut() {
        terminate_one(id, child).await;
    }
}

async fn terminate_one(_id: &str, child: &mut Child) {
    #[cfg(unix)]
    {
        // 先发 SIGTERM 请求优雅退出，5s 内未退出再 SIGKILL。
        if let Some(pid) = child.id() {
            // SAFETY: pid 是自有子进程，kill 语义（发信号）无内存安全风险。
            let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(Ok(_)) => return,
            Ok(Err(e)) => {
                tracing::warn!(id = %_id, %e, "trusted plugin wait after SIGTERM failed");
                return;
            }
            Err(_) => {
                tracing::warn!(id = %_id, "trusted plugin ignored SIGTERM, sending SIGKILL");
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }
    #[cfg(windows)]
    {
        // Windows 无 SIGTERM 语义：直接 TerminateProcess。
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}
