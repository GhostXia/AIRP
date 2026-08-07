//! Trusted Plugin 生命周期（#498 §6.3）：spawn 与终止。
//!
//! daemon 启动时 spawn 全部合法 manifest 的子进程；daemon 退出时终止
//! （Unix：SIGTERM → 等 5s → SIGKILL；Windows 无 SIGTERM 语义，直接强杀）。
//! 不做自动重启（§6.7：崩溃 = 用户重启 daemon）。

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

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
    cmd.env_clear()
        // 审计 A4 修复：env_clear + 最小白名单——daemon 凭据（如
        // AIRP_ACCESS_KEY）不得继承给插件子进程。trusted plugin 虽为
        // 用户显式安装，但环境继承面仍应最小化（#498 §6.3 原文「允许读
        // 自己的环境」修订为白名单语义，见 docs/TRUSTED-PLUGINS.md）。
        // PATH：插件可能派生工具；TEMP/TMP/SYSTEMROOT：Windows 惯例。
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    #[cfg(windows)]
    {
        cmd.env(
            "SYSTEMROOT",
            std::env::var_os("SYSTEMROOT").unwrap_or_default(),
        )
        .env("TEMP", std::env::var_os("TEMP").unwrap_or_default())
        .env("TMP", std::env::var_os("TMP").unwrap_or_default());
    }
    cmd.args(&args)
        // 子进程 cwd = 插件目录：args 里的相对路径（如 "server.js"）
        // 以插件目录为基准，而不是 daemon 的 cwd。
        .current_dir(&plugin_dir)
        // AIRP_* 为 engine 注入的受控环境（env_clear 后不受宿主影响）。
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

/// daemon 退出时终止全部子进程（并发：全部先发 SIGTERM，5s 宽限后
/// 未退出的再 SIGKILL——多个插件不串行等待，审计 S5/CodeRabbit）。
pub async fn terminate_all(children: &mut HashMap<String, Child>) {
    let futures: Vec<_> = children
        .iter_mut()
        .map(|(id, child)| terminate_graceful(id, child))
        .collect();
    futures_util::future::join_all(futures).await;
}

async fn terminate_graceful(_id: &str, child: &mut Child) {
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

/// 崩溃监控（审计 W4，§6.7）：轮询子进程退出状态，退出即从 children 表
/// 移除并 warn 留痕（`/v1/plugins` 状态立即失真纠正为 stopped；**不自动
/// 重启**——§6.7 语义：崩溃 = 用户重启 daemon）。锁内仅做非阻塞 try_wait，
/// 不长时间占锁；全部子进程退出后任务自行结束。
///
/// 注意：子进程正常退出（自己 exit）与崩溃同样处理——engine 无法区分，
/// 统一按「进程已退出」从表中移除。
pub fn monitor_children(children: Arc<tokio::sync::Mutex<HashMap<String, Child>>>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let mut exited: Vec<(String, std::process::ExitStatus)> = Vec::new();
            {
                let mut guard = children.lock().await;
                for (id, child) in guard.iter_mut() {
                    if let Ok(Some(status)) = child.try_wait() {
                        exited.push((id.clone(), status));
                    }
                }
                for (id, _) in &exited {
                    guard.remove(id);
                }
            }
            for (id, status) in &exited {
                tracing::warn!(
                    id = %id,
                    %status,
                    "trusted plugin process exited, removed from children map (no auto-restart)"
                );
            }
            if children.lock().await.is_empty() {
                break;
            }
        }
    });
}
