//! AIRP UI desktop shell. Hosts the Vue WebView and wires the State Protocol
//! bridge (PLAN task B): upstream envelopes arrive via the `airp_dispatch`
//! command; downstream envelopes are emitted on the `airp:envelope` event.
//!
//! The relay itself (`bus::BusRelay`) is an in-process mock standing in for
//! AIRP-Gateway until the live link lands (runtime verification item, PLAN §2.5).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;

use bus::BusRelay;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(BusRelay::new())
        .invoke_handler(tauri::generate_handler![bus::airp_dispatch])
        .setup(|app| {
            // Register the webview as the downstream sink. The relay emits
            // downstream envelopes on `airp:envelope`; the UI's `TauriBus`
            // listens on that event (see src/protocol/tauri-bus.ts).
            let relay = app.state::<BusRelay>();
            relay.subscribe_downstream(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AIRP UI");
}
