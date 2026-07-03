//! AIRP UI desktop shell. Hosts the Vue WebView and wires the State Protocol
//! bridge (PLAN task B): upstream envelopes arrive via the `airp_dispatch`
//! command; downstream envelopes are emitted on the `airp:envelope` event.
//!
//! The relay itself (`bus::BusRelay`) bridges to the headless engine over HTTP.
//! The engine is launched as a sidecar (`binaries/airp-core`) so a packaged
//! `.exe` is self-contained: double-click → UI + engine both come up (首要目标,
//! DEV-GUIDE §0). Engine URL defaults to http://127.0.0.1:8000.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;

use bus::BusRelay;
use tauri::Manager;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

fn main() {
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

            // Spawn the engine sidecar (airp-core daemon). The binary lives at
            // `binaries/airp-core-$TARGET_TRIPLE` per tauri.conf.json externalBin;
            // tauri-plugin-shell resolves the platform-suffixed name. We pass
            // `daemon --port 8000` so the relay's default engine URL connects.
            // On app exit Tauri tears down spawned children. (首要目标: 双击 .exe
            // → UI + engine 自起, 见 DEV-GUIDE §0.)
            match app.shell().sidecar("airp-core") {
                Ok(mut cmd) => {
                    cmd = cmd.args(["daemon", "--port", "8000"]);
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
                            tracing::info!("engine sidecar spawned (airp-core daemon --port 8000)");
                        }
                        Err(e) => {
                            tracing::error!(err = %e,
                                "failed to spawn engine sidecar — UI will not be able to chat \
                                 until you run `cargo run -p airp-core -- daemon --port 8000` manually");
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(err = %e,
                        "sidecar 'airp-core' not configured/found — packaging must build \
                         binaries/airp-core-$TARGET_TRIPLE first; dev mode can run the engine \
                         manually via `cargo run -p airp-core -- daemon --port 8000`");
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AIRP UI");
}
