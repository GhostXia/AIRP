//! Trusted Plugin 生命周期（#498 §6.3）：spawn 与终止。
//!
//! daemon 启动时 spawn 全部合法 manifest 的子进程；daemon 退出时终止
//! （Unix：SIGTERM → 等 5s → SIGKILL；Windows 无 SIGTERM 语义，直接强杀）。
//! 不做自动重启（§6.7：崩溃 = 用户重启 daemon）。
//!
//! 级联 kill（审计 A2/B2）：trusted plugin 可能自己 spawn 子进程（如 TTS
//! 插件调 ffmpeg）。只 kill 直接子会让孙进程变孤儿，Windows 下端口仍占。
//! - Unix：`process_group(0)` 让子进程成为新进程组组长（PGID = PID），
//!   `killpg` 终止整个组（含孙进程）。
//! - Windows：`taskkill /T /F` 终止整个进程树（内部用 Job Object 实现）。
//! - panic/SIGKILL：`kill_on_drop(true)` 保证直接子在 Child drop 时被 kill；
//!   孙进程在 panic 路径仍可能变孤儿（已知限制，MVP 可接受，跟踪 issue）。

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;

use super::{resolve_args, resolve_command, TrustedPluginManifest};

struct PluginLogPrefix<'a>(&'a str);

const PLUGIN_OUTPUT_CHUNK_BYTES: usize = 8 * 1024;

impl fmt::Display for PluginLogPrefix<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[plugin:{}]", self.0)
    }
}

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
    let command = build_command(manifest, data_root)?;
    let program = command.get_program().to_string_lossy().into_owned();
    // tokio Command 零成本包装 std Command（unix process_group 等配置
    // 在 From 转换中完整保留）。
    let mut cmd = tokio::process::Command::from(command);
    // 长跑服务：stdout/stderr 必须由 engine 持续异步排空；若把管道交给
    // 子进程却不读取，OS pipe buffer 填满后会反向阻塞插件。
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // 审计 B3：kill_on_drop 保证 Child drop 时（含 panic / SIGKILL /
        // runtime 异常退出）直接子进程被 kill，不留孤儿进程。
        .kill_on_drop(true);
    // 审计 B2（Unix）：process_group(0) 让子进程成为新进程组组长
    //（PGID = PID），terminate_graceful 用 killpg 终止整个组（含孙进程）。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn `{program}` failed: {e}"))?;
    // 读取任务与 Child 解耦：任务拥有 stdout/stderr，Child 仍只负责
    // 生命周期 / wait，退出时管道 EOF 会让读取任务自然结束。
    let _drainers = attach_output_drainers(&mut child, &manifest.id)?;
    Ok(child)
}

/// 从 plugin stdout/stderr 持续读取并转发到 engine tracing 日志。
///
/// 固定大小 chunk 读取避免无换行输出让行缓冲无界增长。插件输出的任意
/// 字节都会以 lossy 文本记录，因而即使插件写入二进制或非法 UTF-8，读取
/// 任务也不会提前退出而重新让 pipe 填满。每个 chunk 中的行片段均带
/// `[plugin:<id>]` 前缀；stdout 以 info、stderr 以 warn 记录，并用 `stream`
/// 字段保留来源。
fn spawn_output_drainer<R>(
    plugin_id: String,
    stream: &'static str,
    reader: R,
) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = reader;
        let mut chunk = [0_u8; PLUGIN_OUTPUT_CHUNK_BYTES];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => emit_plugin_output(&plugin_id, stream, &chunk[..read]),
                Err(error) => {
                    let prefix = PluginLogPrefix(&plugin_id);
                    tracing::warn!(
                        target: "airp_core::plugins",
                        stream,
                        "{} failed to read {}: {}",
                        prefix,
                        stream,
                        error
                    );
                    break;
                }
            }
        }
    })
}

/// Emit one bounded read chunk. Splitting only inside the fixed chunk keeps
/// every visible line prefixed while never retaining an unbounded partial line.
fn emit_plugin_output(plugin_id: &str, stream: &'static str, chunk: &[u8]) {
    for segment in chunk.split_inclusive(|byte| *byte == b'\n') {
        let mut segment = segment;
        if segment.last() == Some(&b'\n') {
            segment = &segment[..segment.len() - 1];
        }
        if segment.last() == Some(&b'\r') {
            segment = &segment[..segment.len() - 1];
        }
        let text = String::from_utf8_lossy(segment);
        let prefix = PluginLogPrefix(plugin_id);
        if stream == "stderr" {
            tracing::warn!(
                target: "airp_core::plugins",
                stream,
                "{} {}",
                prefix,
                text
            );
        } else {
            tracing::info!(
                target: "airp_core::plugins",
                stream,
                "{} {}",
                prefix,
                text
            );
        }
    }
}

/// 从已 spawn 的 child 取出 stdout/stderr 并启动两个独立读取任务。
///
/// 返回的句柄可由测试等待到 EOF；生产调用方有意丢弃句柄，让任务在
/// runtime 中脱离 spawn 函数持续运行。
fn attach_output_drainers(
    child: &mut Child,
    plugin_id: &str,
) -> Result<[tokio::task::JoinHandle<()>; 2], String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "trusted plugin stdout pipe missing after spawn".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "trusted plugin stderr pipe missing after spawn".to_string())?;
    Ok([
        spawn_output_drainer(plugin_id.to_string(), "stdout", stdout),
        spawn_output_drainer(plugin_id.to_string(), "stderr", stderr),
    ])
}

/// 环境面配置（审计 A4）：env_clear + 最小白名单。
///
/// daemon 凭据（如 AIRP_ACCESS_KEY）不得继承给插件子进程。trusted plugin
/// 虽为用户显式安装，但环境继承面仍应最小化（#498 §6.3 原文「允许读自己
/// 的环境」修订为白名单语义，见 docs/TRUSTED-PLUGINS.md）。PATH：插件可能
/// 派生工具；TEMP/TMP/SYSTEMROOT：Windows 惯例；AIRP_* 为 engine 注入的
/// 受控环境（env_clear 后不受宿主影响）。独立函数便于测试同时用
/// `get_envs()` 契约断言和真实 spawn 探测（见本文件测试模块）。
fn configure_env(
    cmd: &mut std::process::Command,
    manifest: &TrustedPluginManifest,
    data_root: &Path,
) {
    cmd.env_clear()
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
    cmd.env("AIRP_PLUGIN_PORT", manifest.port.to_string())
        .env("AIRP_DATA_ROOT", data_root)
        .env("AIRP_PLUGIN_ID", &manifest.id);
}

/// 构建子进程 Command：configure_env + args + cwd。
fn build_command(
    manifest: &TrustedPluginManifest,
    data_root: &Path,
) -> Result<std::process::Command, String> {
    let command = resolve_command(data_root, manifest)?;
    let args = resolve_args(manifest);
    let plugin_dir = data_root.join("plugins").join(&manifest.id);
    let mut cmd = std::process::Command::new(&command);
    configure_env(&mut cmd, manifest, data_root);
    cmd.args(&args)
        // 子进程 cwd = 插件目录：args 里的相对路径（如 "server.js"）
        // 以插件目录为基准，而不是 daemon 的 cwd。
        .current_dir(&plugin_dir);
    Ok(cmd)
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
        // killpg 终止整个进程组（含孙进程，审计 B2）。
        if let Some(pid) = child.id() {
            // SAFETY: pid 是自有子进程的 PGID（process_group(0) 在 spawn
            // 时设置），killpg 发信号给整个组，语义同 kill 无内存安全风险。
            let _ = unsafe { libc::killpg(pid as i32, libc::SIGTERM) };
        }
        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(Ok(_)) => return,
            Ok(Err(e)) => {
                tracing::warn!(id = %_id, %e, "trusted plugin wait after SIGTERM failed");
                return;
            }
            Err(_) => {
                tracing::warn!(id = %_id, "trusted plugin ignored SIGTERM, sending SIGKILL");
                // SIGKILL 整个进程组（孙进程也一并强杀）。
                if let Some(pid) = child.id() {
                    let _ = unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
                }
                let _ = child.wait().await;
            }
        }
    }
    #[cfg(windows)]
    {
        // Windows 无 SIGTERM 语义：taskkill /T /F 终止整个进程树（含孙进程，
        // 审计 B2）。taskkill 内部用 Job Object 实现 tree kill，比
        // TerminateProcess（只 kill 直接子）覆盖面更广。
        if let Some(pid) = child.id() {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小合法 manifest + 插件目录（含占位可执行文件，满足
    /// resolve_command 的 canonicalize + is_file 校验）。
    fn sample(data_root: &Path) -> TrustedPluginManifest {
        let id = "com.example.envprobe";
        let plugin_dir = data_root.join("plugins").join(id);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("server"), "#!/bin/sh\n").unwrap();
        TrustedPluginManifest {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            command: "./server".to_string(),
            args: vec!["--port".to_string(), "${AIRP_PLUGIN_PORT}".to_string()],
            port: 8899,
            host_api: "1".to_string(),
        }
    }

    /// 审计 A4 契约锁定：env_clear + 白名单的环境面 = PATH（+ Windows
    /// SYSTEMROOT/TEMP/TMP）+ AIRP_* 注入，恰好、无多余；任何凭据形态
    /// 变量不得出现。`Command::get_envs()` 反映 env_clear 后的完整显式
    /// 环境面，无需真正 spawn 子进程（stdout inherit 无法捕获输出，且
    /// Windows CI 无目录内可执行文件可跑）。
    #[test]
    fn env_whitelist_excludes_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path();
        let m = sample(data_root);
        let cmd = build_command(&m, data_root).unwrap();

        let mut envs: Vec<(String, String)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            })
            .collect();
        envs.sort();

        let mut expected = vec![
            "AIRP_DATA_ROOT".to_string(),
            "AIRP_PLUGIN_ID".to_string(),
            "AIRP_PLUGIN_PORT".to_string(),
            "PATH".to_string(),
        ];
        #[cfg(windows)]
        expected.extend([
            "SYSTEMROOT".to_string(),
            "TEMP".to_string(),
            "TMP".to_string(),
        ]);
        expected.sort();

        let keys: Vec<String> = envs.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            keys, expected,
            "env_clear 白名单必须恰好是预期集合（新增变量需显式修订本测试）"
        );

        // 注入值正确（端口号 / 数据根路径 / 占位符替换后的 args）。
        assert_eq!(
            envs.iter()
                .find(|(k, _)| k == "AIRP_PLUGIN_PORT")
                .unwrap()
                .1,
            "8899"
        );
        assert_eq!(
            envs.iter().find(|(k, _)| k == "AIRP_DATA_ROOT").unwrap().1,
            data_root.to_string_lossy()
        );
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--port", "8899"]);

        // 凭据形态变量不得出现（防未来把凭据加回白名单）。
        for (k, _) in &envs {
            let upper = k.to_ascii_uppercase();
            for marker in ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"] {
                assert!(
                    !upper.contains(marker),
                    "whitelist must not carry credential variable: {k}"
                );
            }
        }
    }

    /// 测试结束恢复注入的模拟凭据，避免污染并行测试进程环境。
    struct CredentialGuard(Option<std::ffi::OsString>);
    impl Drop for CredentialGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("AIRP_ACCESS_KEY", v),
                None => std::env::remove_var("AIRP_ACCESS_KEY"),
            }
        }
    }

    /// CodeRabbit #525 补强：`get_envs()` 只反映显式条目，无法发现
    /// `.env_clear()` 被意外移除的回归（此时继承的 daemon 凭据会泄漏给
    /// 插件子进程，但 get_envs 断言仍绿）。本测试真实 spawn 一个环境枚举
    /// 子进程（Windows `cmd /c set` / Unix `env`），注入模拟凭据后断言
    /// 子进程观察不到它——锁定「凭据不跨进程边界」的运行时行为。
    #[test]
    fn spawned_probe_sees_no_inherited_credentials() {
        let _lock = crate::TEST_ENV_LOCK.blocking_lock();
        // 模拟 daemon 环境携带凭据；Guard 在测试结束（含 panic）恢复。
        let previous = std::env::var_os("AIRP_ACCESS_KEY");
        std::env::set_var("AIRP_ACCESS_KEY", "synthetic-secret");
        let _guard = CredentialGuard(previous);

        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path();
        let m = sample(data_root);

        #[cfg(windows)]
        let mut probe = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "set"]);
            c
        };
        #[cfg(not(windows))]
        let mut probe = std::process::Command::new("env");

        configure_env(&mut probe, &m, data_root);
        let out = probe.output().unwrap();
        assert!(out.status.success(), "env probe failed: {out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.contains("AIRP_ACCESS_KEY"),
            "spawned probe must not observe inherited credential; env_clear 缺失或白名单被破坏"
        );
        // env_clear 未误伤白名单变量：probe 能看到注入的 AIRP_* 与 PATH。
        for expected in [
            "AIRP_PLUGIN_PORT=8899",
            "AIRP_PLUGIN_ID=com.example.envprobe",
        ] {
            assert!(
                stdout.contains(expected),
                "probe missing whitelist entry {expected}"
            );
        }
        assert!(stdout.contains("PATH="), "probe missing PATH");
    }

    #[test]
    fn plugin_log_lines_include_source_prefix() {
        let id = "com.example.logs";
        assert_eq!(
            format!("{} ready", PluginLogPrefix(id)),
            "[plugin:com.example.logs] ready"
        );
    }

    /// 回归：没有换行符的持续输出也必须被固定大小 chunk 读取，不能让
    /// partial line 缓冲随插件输出无限增长；两条 stream 都要保持可写。
    #[tokio::test]
    async fn plugin_output_drainer_handles_no_newline_flood() {
        use tokio::io::AsyncWriteExt;

        let (stdout_writer, stdout_reader) = tokio::io::duplex(PLUGIN_OUTPUT_CHUNK_BYTES);
        let (stderr_writer, stderr_reader) = tokio::io::duplex(PLUGIN_OUTPUT_CHUNK_BYTES);
        let stdout_drainer =
            spawn_output_drainer("com.example.flood".to_string(), "stdout", stdout_reader);
        let stderr_drainer =
            spawn_output_drainer("com.example.flood".to_string(), "stderr", stderr_reader);

        let stdout_writer = tokio::spawn(async move {
            let mut writer = stdout_writer;
            let chunk = vec![b'x'; PLUGIN_OUTPUT_CHUNK_BYTES];
            for _ in 0..1024 {
                writer.write_all(&chunk).await.unwrap();
            }
        });
        let stderr_writer = tokio::spawn(async move {
            let mut writer = stderr_writer;
            let chunk = vec![b'y'; PLUGIN_OUTPUT_CHUNK_BYTES];
            for _ in 0..1024 {
                writer.write_all(&chunk).await.unwrap();
            }
        });

        tokio::time::timeout(std::time::Duration::from_secs(5), stdout_writer)
            .await
            .expect("stdout no-newline flood writer blocked")
            .expect("stdout no-newline flood writer panicked");
        tokio::time::timeout(std::time::Duration::from_secs(5), stderr_writer)
            .await
            .expect("stderr no-newline flood writer blocked")
            .expect("stderr no-newline flood writer panicked");
        tokio::time::timeout(std::time::Duration::from_secs(5), stdout_drainer)
            .await
            .expect("stdout no-newline drainer did not reach EOF")
            .expect("stdout no-newline drainer panicked");
        tokio::time::timeout(std::time::Duration::from_secs(5), stderr_drainer)
            .await
            .expect("stderr no-newline drainer did not reach EOF")
            .expect("stderr no-newline drainer panicked");
    }

    /// 回归：同时写满 stdout/stderr 的插件必须在有限时间内退出；若任一
    /// pipe 没有被独立任务持续读取，子进程会在 OS pipe buffer 满时卡住，
    /// 该 wait 超时即可暴露回归。
    #[tokio::test]
    async fn plugin_output_drainers_prevent_pipe_backpressure() {
        #[cfg(windows)]
        let mut command = {
            let mut command = tokio::process::Command::new("cmd");
            command.args([
                "/C",
                "(for /L %i in (1,1,8192) do @echo x) & (for /L %i in (1,1,8192) do @echo x 1>&2)",
            ]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = tokio::process::Command::new("sh");
            command.args([
                "-c",
                "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
            ]);
            command
        };

        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn pipe-fill fixture");
        let drainers = attach_output_drainers(&mut child, "com.example.flood")
            .expect("pipe handles must be available");

        let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .expect("plugin must not block on a full stdout/stderr pipe")
            .expect("pipe-fill fixture wait failed");
        assert!(status.success(), "pipe-fill fixture failed: {status}");
        for drainer in drainers {
            drainer.await.expect("plugin output drainer task panicked");
        }
    }
}
