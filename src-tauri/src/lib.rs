use model::BackendState;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::error::AppError;

mod auto_scan;
mod auto_scan_log;
mod config;
mod database;
mod disk; // compile my stuff
mod error;
mod model;
mod platform;
mod startup;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// register the function in the invoke handler
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background-scan"]),
        ))
        .setup(|app| {
            let is_background_scan = auto_scan::is_background_scan_requested();
            let local_app_data_path = match app.path().app_local_data_dir() {
                Ok(path) => path,
                Err(_) => {
                    if !is_background_scan {
                        app.dialog()
                        .message(
                            "Fatal Error: An error has occured during app startup. 
                            The app cannot get it's local appdata path. 
                            This is likely due to a corrupted OS environment or missing environment variables. 
                            The application cannot function without this path and will now exit.")
                        .kind(MessageDialogKind::Error)
                        .title("Startup Error")
                        .blocking_show();
                    }

                    return Err(Box::new(AppError::CustomError("Failed app startup".to_string())));
                }
            };

            if let Err(e) = startup::startup_checks(&local_app_data_path) {
                if is_background_scan {
                    let _ = auto_scan::record_background_error(
                        &local_app_data_path,
                        e.to_string(),
                    );
                    return Err(Box::new(e));
                } else {
                    app.dialog()
                        .message(format!(
                            "An error has occured during app startup: {}",
                            e.to_string()
                        ))
                        .kind(MessageDialogKind::Error)
                        .title("Startup Error")
                        .blocking_show();
                }
            }

            if is_background_scan {
                let _ = auto_scan_log::append(
                    &local_app_data_path,
                    format!(
                        "app setup detected background scan mode: args={:?}",
                        std::env::args().collect::<Vec<_>>()
                    ),
                );
            }

            let state = BackendState {
                file_tree: std::sync::Mutex::new(None),
                local_appdata_path: Some(local_app_data_path.clone()),
            };
            app.manage(state);

            if is_background_scan {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_skip_taskbar(true);
                    let _ = window.hide();
                }

                let app_handle = app.handle().clone();
                let scan_app_handle = app.handle().clone();
                let scan_local_app_data_path = local_app_data_path.clone();

                std::thread::spawn(move || {
                    if let Err(err) = auto_scan::run_background_auto_scan(
                        scan_app_handle,
                        &scan_local_app_data_path,
                    ) {
                        let _ = auto_scan::record_background_error(
                            &scan_local_app_data_path,
                            err.to_string(),
                        );
                    }
                    app_handle.exit(0);
                });
            } else if let Some(window) = app.get_webview_window("main") {
                window.show()?;
                window.set_focus()?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            disk::retreive_disks,
            disk::disk_scan,
            disk::query_new_dir_object,
            database::write_current_tree,
            database::get_local_snapshot_files,
            database::delete_snapshot_file,
            database::open_snapshot_preview,
            database::query_snapshot_dir_object,
            database::compare_snapshots,
            database::query_snapshot_compare_dir_object,
            database::get_path_historical_data,
            config::get_auto_scan_config,
            config::update_auto_scan_config,
            config::get_auto_scan_diagnostics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
