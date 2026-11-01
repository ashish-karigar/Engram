// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use engram_daemon::{DaemonOptions, RunningDaemon, start_daemon};
use serde::Serialize;
use tauri::{Manager, RunEvent};

/// What the UI needs to talk to the daemon running inside this process.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DaemonInfo {
    url: String,
    admin_token: String,
}

struct DaemonState(Mutex<Option<RunningDaemon>>);

#[tauri::command]
fn daemon_info(state: tauri::State<'_, DaemonState>) -> Result<DaemonInfo, String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    let daemon = guard.as_ref().ok_or("daemon is not running")?;
    Ok(DaemonInfo { url: daemon.url.clone(), admin_token: daemon.admin_token.clone() })
}

fn main() {
    // Agent hooks call this same binary: `MemoryOS hook claude`. Answer and exit without a window.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("hook") {
        std::process::exit(engram_daemon::hook::run_hook(args.get(2).map_or("claude", String::as_str)));
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![daemon_info])
        .setup(|app| {
            // The daemon runs in-process: one binary, no separate runtime to install.
            let daemon = tauri::async_runtime::block_on(start_daemon(DaemonOptions::default()))?;
            app.manage(DaemonState(Mutex::new(Some(daemon))));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build MemoryOS");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            let state = handle.state::<DaemonState>();
            let daemon = state.0.lock().ok().and_then(|mut guard| guard.take());
            if let Some(daemon) = daemon {
                tauri::async_runtime::block_on(daemon.stop());
            }
        }
    });
}
