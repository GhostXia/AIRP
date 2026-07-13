use std::process::Command;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

fn production_command(test_root: &std::path::Path, data_root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_airp-core"));
    command
        .arg("--config")
        .arg(test_root.join("production-startup-test-config.json"))
        .arg("daemon")
        .env("AIRP_DEPLOYMENT_MODE", "production")
        .env("AIRP_PUBLIC_ORIGIN", "https://airp.example.com")
        .env("AIRP_DATA_DIR", data_root)
        .env_remove("AIRP_ACCESS_KEY")
        .env_remove("AIRP_ALLOW_LOCAL_PATH");
    command
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn output_with_timeout(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() >= STARTUP_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not reject invalid production startup within {STARTUP_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn production_daemon_exits_before_serving_when_access_key_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("production-startup-test-config.json");
    let output = output_with_timeout(&mut production_command(tmp.path(), tmp.path()));
    assert!(!output.status.success());
    assert!(stderr(&output).contains("AIRP_ACCESS_KEY is required"));
    assert!(!config.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_exits_before_creating_a_missing_data_root() {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().join("must-already-exist");
    let config = tmp.path().join("production-startup-test-config.json");
    let mut command = production_command(tmp.path(), &data_root);
    command.env(
        "AIRP_ACCESS_KEY",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    );
    let output = output_with_timeout(&mut command);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("must already exist"));
    assert!(!data_root.exists());
    assert!(!config.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_rejects_local_path_import_before_serving() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("production-startup-test-config.json");
    let mut command = production_command(tmp.path(), tmp.path());
    command
        .env(
            "AIRP_ACCESS_KEY",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .env("AIRP_ALLOW_LOCAL_PATH", "1");
    let output = output_with_timeout(&mut command);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("AIRP_ALLOW_LOCAL_PATH is forbidden"));
    assert!(!config.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_rejects_invalid_origin_before_config_write() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("production-startup-test-config.json");
    let mut command = production_command(tmp.path(), tmp.path());
    command
        .env(
            "AIRP_ACCESS_KEY",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .env("AIRP_PUBLIC_ORIGIN", "http://airp.example.com");
    let output = output_with_timeout(&mut command);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("canonical HTTPS origin"));
    assert!(!config.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn development_daemon_rejects_non_loopback_listener() {
    let temp = tempfile::tempdir().expect("temp data root");
    let config = temp.path().join("config.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_airp-core"));
    command
        .args([
            "--config",
            config.to_str().expect("utf8 config path"),
            "daemon",
            "--host",
            "0.0.0.0",
            "--port",
            "0",
        ])
        .env("AIRP_DATA_DIR", temp.path())
        .env("AIRP_DEPLOYMENT_MODE", "development");
    let output = output_with_timeout(&mut command);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("non-loopback --host is allowed only in production mode"),
        "unexpected stderr: {stderr}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}
