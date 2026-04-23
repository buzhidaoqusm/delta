use chrono::{DateTime, Duration, Utc};
use std::path::Path;

use crate::auto_scan_log;
use crate::config::{self, AutoScanStatus};
use crate::database;
use crate::disk;
use crate::error::AppError;

const BACKGROUND_SCAN_ARG: &str = "--background-scan";

pub fn is_background_scan_requested() -> bool {
    std::env::args().any(|arg| arg == BACKGROUND_SCAN_ARG)
}

fn update_status(
    local_appdata_path: &Path,
    status: AutoScanStatus,
    error: Option<String>,
) -> Result<(), AppError> {
    let mut app_config = config::read_app_config(local_appdata_path)?;
    app_config.auto_scan.last_status = status;
    app_config.auto_scan.last_error = error;
    config::write_app_config(local_appdata_path, &app_config)
}

pub fn record_background_error(local_appdata_path: &Path, message: String) -> Result<(), AppError> {
    let _ = auto_scan_log::append(
        local_appdata_path,
        format!("background scan error: {message}"),
    );
    update_status(local_appdata_path, AutoScanStatus::Error, Some(message))
}

fn is_due(last_scan_at: Option<&str>, interval_days: u64) -> bool {
    let Some(last_scan_at) = last_scan_at else {
        return true;
    };

    let Ok(last) = DateTime::parse_from_rfc3339(last_scan_at) else {
        return true;
    };

    Utc::now().signed_duration_since(last.with_timezone(&Utc))
        >= Duration::days(interval_days as i64)
}

pub fn run_background_auto_scan(
    app: tauri::AppHandle,
    local_appdata_path: &Path,
) -> Result<(), AppError> {
    let _ = auto_scan_log::append(
        local_appdata_path,
        format!(
            "background scan started: args={:?}, current_exe={}",
            std::env::args().collect::<Vec<_>>(),
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|err| format!("failed to read current exe: {err}"))
        ),
    );

    let mut app_config = config::read_app_config(local_appdata_path)?;
    let auto_scan = app_config.auto_scan.clone();

    if !auto_scan.enabled {
        let _ = auto_scan_log::append(local_appdata_path, "background scan skipped: disabled");
        update_status(local_appdata_path, AutoScanStatus::Disabled, None)?;
        return Ok(());
    }

    if auto_scan.target_path.trim().is_empty() {
        let _ = auto_scan_log::append(local_appdata_path, "background scan failed: empty target");
        update_status(
            local_appdata_path,
            AutoScanStatus::Error,
            Some("Auto scan target path is empty".to_string()),
        )?;
        return Ok(());
    }

    let target_path = Path::new(&auto_scan.target_path);
    if !target_path.exists() {
        let _ = auto_scan_log::append(
            local_appdata_path,
            format!(
                "background scan failed: target does not exist: {}",
                auto_scan.target_path
            ),
        );
        update_status(
            local_appdata_path,
            AutoScanStatus::Error,
            Some("Auto scan target path does not exist".to_string()),
        )?;
        return Ok(());
    }

    if !is_due(auto_scan.last_scan_at.as_deref(), auto_scan.interval_days) {
        let _ = auto_scan_log::append(
            local_appdata_path,
            format!(
                "background scan skipped: interval not elapsed, last_scan_at={:?}, interval_days={}",
                auto_scan.last_scan_at, auto_scan.interval_days
            ),
        );
        update_status(local_appdata_path, AutoScanStatus::SkippedInterval, None)?;
        return Ok(());
    }

    let _ = auto_scan_log::append(
        local_appdata_path,
        format!("background scan running: target_path=\"{}\"", auto_scan.target_path),
    );
    let root = disk::naive_scan(&auto_scan.target_path, app)?;
    let current_size = root.meta.size;
    let previous_size = auto_scan.last_scan_size_bytes;
    let should_save = previous_size
        .map(|previous| current_size.abs_diff(previous) >= auto_scan.save_threshold_bytes)
        .unwrap_or(true);

    app_config.auto_scan.last_scan_at = Some(Utc::now().to_rfc3339());
    app_config.auto_scan.last_scan_size_bytes = Some(current_size);
    app_config.auto_scan.last_error = None;

    if should_save {
        let written =
            database::write_tree_snapshot(&root, &auto_scan.target_path, local_appdata_path)?;
        let _ = auto_scan_log::append(
            local_appdata_path,
            format!(
                "background scan completed: snapshot saved, file_stem={}, current_size={}, previous_size={:?}, threshold={}",
                written.file_stem,
                current_size,
                previous_size,
                auto_scan.save_threshold_bytes
            ),
        );
        app_config.auto_scan.last_snapshot_file = Some(written.file_stem);
        app_config.auto_scan.last_status = AutoScanStatus::SuccessSaved;
    } else {
        let _ = auto_scan_log::append(
            local_appdata_path,
            format!(
                "background scan completed: skipped snapshot by threshold, current_size={}, previous_size={:?}, threshold={}",
                current_size,
                previous_size,
                auto_scan.save_threshold_bytes
            ),
        );
        app_config.auto_scan.last_status = AutoScanStatus::SuccessSkippedThreshold;
    }

    config::write_app_config(local_appdata_path, &app_config)?;
    Ok(())
}
