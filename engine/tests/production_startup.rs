use std::process::Command;

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

#[test]
fn production_daemon_exits_before_serving_when_access_key_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let output = production_command(tmp.path(), tmp.path()).output().unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("AIRP_ACCESS_KEY is required"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_exits_before_creating_a_missing_data_root() {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().join("must-already-exist");
    let output = production_command(tmp.path(), &data_root)
        .env(
            "AIRP_ACCESS_KEY",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("must already exist"));
    assert!(!data_root.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}

#[test]
fn production_daemon_rejects_local_path_import_before_serving() {
    let tmp = tempfile::tempdir().unwrap();
    let output = production_command(tmp.path(), tmp.path())
        .env(
            "AIRP_ACCESS_KEY",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .env("AIRP_ALLOW_LOCAL_PATH", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("AIRP_ALLOW_LOCAL_PATH is forbidden"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Gateway running"));
}
