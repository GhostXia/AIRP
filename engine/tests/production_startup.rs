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
    let output = output_with_timeout(&mut production_command(tmp.path(), tmp.path()));
    assert!(!output.status.success());
    assert!(stderr(&output).contains("AIRP_ACCESS_KEY is required"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_exits_before_creating_a_missing_data_root() {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().join("must-already-exist");
    let mut command = production_command(tmp.path(), &data_root);
    command.env(
        "AIRP_ACCESS_KEY",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    );
    let output = output_with_timeout(&mut command);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("must already exist"));
    assert!(!data_root.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_rejects_local_path_import_before_serving() {
    let tmp = tempfile::tempdir().unwrap();
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
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}
