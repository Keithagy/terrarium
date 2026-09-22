//! Terrarium desktop app (Tauri 2, macOS).

pub mod bridge;
pub mod commands;
pub mod layout_runner;
pub mod snapshot;
pub mod state;
pub mod telemetry;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;

pub fn run() {
    let telemetry = telemetry::init();
    let state = Arc::new(AppState::new(telemetry));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), pid = std::process::id(), home = %terrarium_core::cache::home_dir().display(), "terrarium starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::scan_repo,
            commands::get_view,
            commands::run_layout,
            commands::stop_layout,
            commands::set_position,
            commands::get_node,
            commands::search_nodes,
            commands::list_flows,
            commands::list_traces,
            commands::get_trace,
            commands::list_endpoints,
            commands::list_boundaries,
            commands::list_hotspots,
            commands::recent_repos,
            commands::report_ui,
            commands::report_metrics,
            commands::log_event,
            commands::bridge_reply,
            commands::bridge_info,
            commands::open_path,
            commands::initial_repo,
        ])
        .setup(move |app| {
            let window = app.get_webview_window("main").expect("main window");
            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{
                    NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy,
                };
                if let Err(e) = apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::UnderWindowBackground,
                    Some(NSVisualEffectState::Active),
                    Some(16.0),
                ) {
                    tracing::warn!(error = %e, "vibrancy unavailable");
                }
            }
            let ctx = bridge::Ctx {
                app: app.handle().clone(),
                state: state.clone(),
            };
            tauri::async_runtime::spawn(async move {
                if let Err(e) = bridge::serve(ctx).await {
                    tracing::error!(error = %e, "agent bridge failed to start");
                }
            });
            // `TERRARIUM_OPEN=<path>` scans a repository as soon as the UI is ready
            // (the frontend asks for it via `initial_repo`, see commands in the UI).
            if std::env::var("TERRARIUM_DEVTOOLS")
                .map(|v| v == "1")
                .unwrap_or(false)
            {
                window.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running terrarium");
}
