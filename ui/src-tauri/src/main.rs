//! AIRP UI desktop shell. Hosts the Vue WebView and wires the State Protocol
//! bridge (PLAN task B): upstream envelopes arrive via the `airp_dispatch`
//! command; downstream envelopes are emitted on the `airp:envelope` event.
//!
//! The relay itself (`bus::BusRelay`) bridges to the headless engine over HTTP.
//! The engine is launched as a sidecar (`binaries/airp-core`) so a packaged
//! `.exe` is self-contained: double-click → UI + engine both come up (首要目标,
//! DEV-GUIDE §0). Engine URL defaults to the configured local daemon port.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;

use std::path::Path;

use airp_state_protocol::{Body, Envelope};
use bus::BusRelay;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tracing_subscriber::EnvFilter;

const DEFAULT_ENGINE_PORT: u16 = 8000;

#[derive(Debug, Deserialize, Default)]
struct SidecarSettings {
    daemon_port: Option<u16>,
    access_api_key: Option<String>,
}

fn main() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(BusRelay::new())
        .invoke_handler(tauri::generate_handler![bus::airp_dispatch])
        .setup(|app| {
            // Register the webview as the downstream sink. The relay emits
            // downstream envelopes on `airp:envelope`; the UI's `TauriBus`
            // listens on that event (see src/protocol/tauri-bus.ts).
            let relay = app.state::<BusRelay>();
            relay.subscribe_downstream(app.handle().clone());

            let data_root = app.path().app_data_dir()?.join("data");
            std::fs::create_dir_all(&data_root)?;
            let (port, access_key) = load_sidecar_settings(&data_root);
            let env_engine_url = std::env::var("AIRP_ENGINE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty());
            let engine_url = env_engine_url
                .clone()
                .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
            relay.configure_engine(engine_url.clone(), access_key.clone());

            tracing::info!(
                data_root = %data_root.display(),
                engine_url = %engine_url,
                has_access_key = access_key.is_some(),
                "engine connection configured"
            );

            if env_engine_url.is_some() {
                tracing::info!(
                    engine_url = %engine_url,
                    "AIRP_ENGINE_URL is set; skipping bundled sidecar spawn"
                );
                return Ok(());
            }

            // Spawn the engine sidecar (airp-core daemon). The binary lives at
            // `binaries/airp-core-$TARGET_TRIPLE` per tauri.conf.json externalBin;
            // tauri-plugin-shell resolves the platform-suffixed name. The daemon
            // port and BusRelay URL share the same settings/env-derived value.
            // AIRP_DATA_DIR forces packaged builds to use a per-user data root
            // instead of writing relative to Program Files or the install dir.
            let port_arg = port.to_string();
            match app.shell().sidecar("airp-core") {
                Ok(mut cmd) => {
                    cmd = cmd
                        .args(["daemon", "--port", port_arg.as_str()])
                        .current_dir(&data_root)
                        .env("AIRP_DATA_DIR", &data_root)
                        .env("AIRP_ALLOW_LOCAL_PATH", "1");
                    match cmd.spawn() {
                        Ok((mut rx, _child)) => {
                            // Single receiver yields all CommandEvent variants
                            // (Stdout/Stderr/Terminated/...). Log each for
                            // debuggability (透明取向: 引擎状态可观察).
                            tauri::async_runtime::spawn(async move {
                                while let Some(ev) = rx.recv().await {
                                    match ev {
                                        CommandEvent::Stdout(b) => tracing::info!(
                                            target: "airp-core",
                                            "engine: {}", String::from_utf8_lossy(&b).trim_end()),
                                        CommandEvent::Stderr(b) => tracing::warn!(
                                            target: "airp-core",
                                            "engine err: {}", String::from_utf8_lossy(&b).trim_end()),
                                        CommandEvent::Terminated(p) => tracing::warn!(
                                            target: "airp-core",
                                            "engine sidecar terminated: {:?}", p),
                                        _ => {}
                                    }
                                }
                            });
                            tracing::info!(
                                port = port,
                                data_root = %data_root.display(),
                                "engine sidecar spawned"
                            );
                            spawn_engine_health_probe(app.handle().clone(), engine_url.clone());
                        }
                        Err(e) => {
                            tracing::error!(err = %e,
                                "failed to spawn engine sidecar");
                            emit_engine_error(
                                app.handle(),
                                format!(
                                    "Engine failed to start. Run `cargo run -p airp-core -- daemon --port {port}` manually or rebuild the sidecar: {e}"
                                ),
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(err = %e,
                        "sidecar 'airp-core' not configured/found — packaging must build \
                         binaries/airp-core-$TARGET_TRIPLE first");
                    emit_engine_error(
                        app.handle(),
                        format!(
                            "Engine sidecar is missing. Run `ui/build-engine-sidecar.ps1` or start `cargo run -p airp-core -- daemon --port {port}` manually: {e}"
                        ),
                    );
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AIRP UI");
}

fn init_tracing() {
    let filter = std::env::var("AIRP_UI_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn load_sidecar_settings(data_root: &Path) -> (u16, Option<String>) {
    let mut port = DEFAULT_ENGINE_PORT;
    let mut access_key = None;

    let settings_path = data_root.join("settings.json");
    if settings_path.exists() {
        match std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<SidecarSettings>(&raw).ok())
        {
            Some(settings) => {
                if let Some(value) = settings.daemon_port {
                    port = value;
                }
                access_key = settings.access_api_key.filter(|key| !key.is_empty());
            }
            None => tracing::warn!(
                path = %settings_path.display(),
                "failed to parse sidecar settings; using defaults"
            ),
        }
    }

    if let Ok(value) = std::env::var("AIRP_DAEMON_PORT") {
        match value.parse::<u16>() {
            Ok(value) => port = value,
            Err(e) => tracing::warn!(err = %e, value = %value, "invalid AIRP_DAEMON_PORT"),
        }
    }
    if let Ok(value) = std::env::var("AIRP_ACCESS_KEY") {
        if !value.is_empty() {
            access_key = Some(value);
        }
    }

    (port, access_key)
}

fn spawn_engine_health_probe(app: tauri::AppHandle, engine_url: String) {
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        for _ in 0..50 {
            if client
                .get(format!("{engine_url}/version"))
                .send()
                .await
                .map(|resp| resp.status().is_success())
                .unwrap_or(false)
            {
                tracing::info!(engine_url = %engine_url, "engine sidecar ready");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        tracing::error!(engine_url = %engine_url, "engine sidecar did not become ready");
        emit_engine_error(
            &app,
            format!("Engine did not become ready at {engine_url} within 5 seconds."),
        );
    });
}

fn emit_engine_error(app: &tauri::AppHandle, message: String) {
    let env = Envelope::new(
        "engine-startup-error",
        now_ms(),
        "gateway",
        Body::Error(airp_state_protocol::ErrorMsg {
            code: "engine_startup_error".into(),
            message,
            detail: None,
        }),
    );
    let _ = app.emit(bus::ENVELOPE_EVENT, &env);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("airp-ui-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn sidecar_settings_default_when_missing() {
        let root = temp_data_root("missing-settings");
        let (port, access_key) = load_sidecar_settings(&root);
        assert_eq!(port, DEFAULT_ENGINE_PORT);
        assert_eq!(access_key, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sidecar_settings_reads_port_and_access_key() {
        let root = temp_data_root("settings");
        std::fs::write(
            root.join("settings.json"),
            r#"{"daemon_port": 8123, "access_api_key": "secret"}"#,
        )
        .unwrap();
        let (port, access_key) = load_sidecar_settings(&root);
        assert_eq!(port, 8123);
        assert_eq!(access_key.as_deref(), Some("secret"));
        let _ = std::fs::remove_dir_all(root);
    }
}
