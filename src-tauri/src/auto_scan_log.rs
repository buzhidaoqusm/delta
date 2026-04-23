use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;

const LOG_FILE_NAME: &str = "auto-scan.log";

pub fn log_path(local_appdata_path: &Path) -> PathBuf {
    local_appdata_path.join(LOG_FILE_NAME)
}

pub fn append(local_appdata_path: &Path, message: impl AsRef<str>) -> Result<(), AppError> {
    fs::create_dir_all(local_appdata_path)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(local_appdata_path))?;

    writeln!(
        file,
        "[{}] {}",
        Local::now().to_rfc3339(),
        message.as_ref()
    )?;

    Ok(())
}
