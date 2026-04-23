use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri_plugin_autostart::ManagerExt;

use crate::auto_scan_log;
use crate::error::AppError;

const CONFIG_FILE_NAME: &str = "config.json";
const DEFAULT_INTERVAL_DAYS: u64 = 7;
const DEFAULT_THRESHOLD_BYTES: u64 = 1_073_741_824;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    #[serde(default)]
    pub auto_scan: AutoScanConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AutoScanDiagnostics {
    pub log_path: String,
    pub autostart_enabled: bool,
    pub current_exe: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AutoScanConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub target_path: String,
    #[serde(default = "default_interval_days")]
    pub interval_days: u64,
    #[serde(default = "default_threshold_bytes")]
    pub save_threshold_bytes: u64,
    #[serde(default)]
    pub last_scan_at: Option<String>,
    #[serde(default)]
    pub last_scan_size_bytes: Option<u64>,
    #[serde(default)]
    pub last_snapshot_file: Option<String>,
    #[serde(default)]
    pub last_status: AutoScanStatus,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum AutoScanStatus {
    NeverRun,
    SuccessSaved,
    SuccessSkippedThreshold,
    SkippedInterval,
    Disabled,
    Error,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_scan: AutoScanConfig::default(),
        }
    }
}

impl Default for AutoScanConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_path: String::new(),
            interval_days: DEFAULT_INTERVAL_DAYS,
            save_threshold_bytes: DEFAULT_THRESHOLD_BYTES,
            last_scan_at: None,
            last_scan_size_bytes: None,
            last_snapshot_file: None,
            last_status: AutoScanStatus::NeverRun,
            last_error: None,
        }
    }
}

impl Default for AutoScanStatus {
    fn default() -> Self {
        Self::NeverRun
    }
}

fn default_interval_days() -> u64 {
    DEFAULT_INTERVAL_DAYS
}

fn default_threshold_bytes() -> u64 {
    DEFAULT_THRESHOLD_BYTES
}

pub fn config_path(local_appdata_path: &Path) -> PathBuf {
    local_appdata_path.join(CONFIG_FILE_NAME)
}

pub fn read_app_config(local_appdata_path: &Path) -> Result<AppConfig, AppError> {
    let path = config_path(local_appdata_path);
    if !path.exists() {
        let config = AppConfig::default();
        write_app_config(local_appdata_path, &config)?;
        return Ok(config);
    }

    let raw = fs::read_to_string(path)?;
    let config = serde_json::from_str::<AppConfig>(&raw)
        .map_err(|err| AppError::CustomError(format!("Failed to parse config.json: {err}")))?;

    Ok(normalize_app_config(config))
}

pub fn write_app_config(local_appdata_path: &Path, config: &AppConfig) -> Result<(), AppError> {
    fs::create_dir_all(local_appdata_path)?;
    let raw = serde_json::to_string_pretty(config)
        .map_err(|err| AppError::CustomError(format!("Failed to serialize config.json: {err}")))?;
    fs::write(config_path(local_appdata_path), raw)?;
    Ok(())
}

pub fn normalize_app_config(mut config: AppConfig) -> AppConfig {
    if config.auto_scan.interval_days == 0 {
        config.auto_scan.interval_days = DEFAULT_INTERVAL_DAYS;
    }
    config
}

fn validate_auto_scan_config(config: &AutoScanConfig) -> Result<(), AppError> {
    if config.enabled && config.target_path.trim().is_empty() {
        return Err(AppError::GeneralLogicalErr(
            "Auto scan target path is required when auto scan is enabled".to_string(),
        ));
    }

    if config.interval_days == 0 {
        return Err(AppError::GeneralLogicalErr(
            "Auto scan interval must be at least 1 day".to_string(),
        ));
    }

    Ok(())
}

#[tauri::command]
pub fn get_auto_scan_config(
    state: tauri::State<'_, crate::model::BackendState>,
) -> Result<AppConfig, AppError> {
    let local_appdata_path = state
        .local_appdata_path
        .as_ref()
        .ok_or_else(|| AppError::StartupError("Missing local app data path".to_string()))?;

    read_app_config(local_appdata_path)
}

#[tauri::command]
pub fn update_auto_scan_config(
    config: AutoScanConfig,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::model::BackendState>,
) -> Result<AppConfig, AppError> {
    validate_auto_scan_config(&config)?;

    let local_appdata_path = state
        .local_appdata_path
        .as_ref()
        .ok_or_else(|| AppError::StartupError("Missing local app data path".to_string()))?;

    let mut app_config = read_app_config(local_appdata_path)?;
    app_config.auto_scan = config;
    let current_exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|err| format!("failed to read current exe: {err}"));

    let _ = auto_scan_log::append(
        local_appdata_path,
        format!(
            "settings update requested: enabled={}, target_path=\"{}\", interval_days={}, save_threshold_bytes={}, current_exe=\"{}\"",
            app_config.auto_scan.enabled,
            app_config.auto_scan.target_path,
            app_config.auto_scan.interval_days,
            app_config.auto_scan.save_threshold_bytes,
            current_exe
        ),
    );

    if app_config.auto_scan.enabled {
        app.autolaunch().enable()?;
        let is_enabled = app.autolaunch().is_enabled()?;
        let _ = auto_scan_log::append(
            local_appdata_path,
            format!("autostart enable succeeded: plugin_is_enabled={is_enabled}"),
        );
    } else {
        match app.autolaunch().disable() {
            Ok(()) => {
                let _ = auto_scan_log::append(local_appdata_path, "autostart disable succeeded");
            }
            Err(err) => {
                let _ = auto_scan_log::append(
                    local_appdata_path,
                    format!("autostart disable returned error: {err}"),
                );
                if app.autolaunch().is_enabled().unwrap_or(false) {
                    return Err(err.into());
                }
            }
        }
    }

    write_app_config(local_appdata_path, &app_config)?;
    Ok(app_config)
}

#[tauri::command]
pub fn get_auto_scan_diagnostics(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::model::BackendState>,
) -> Result<AutoScanDiagnostics, AppError> {
    let local_appdata_path = state
        .local_appdata_path
        .as_ref()
        .ok_or_else(|| AppError::StartupError("Missing local app data path".to_string()))?;

    let current_exe = std::env::current_exe()?.display().to_string();
    let autostart_enabled = app.autolaunch().is_enabled()?;

    Ok(AutoScanDiagnostics {
        log_path: auto_scan_log::log_path(local_appdata_path)
            .display()
            .to_string(),
        autostart_enabled,
        current_exe,
    })
}
