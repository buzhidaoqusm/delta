use humansize::{format_size, DECIMAL};
use std::collections::HashMap;
use std::fs::{self};
use std::path::PathBuf;
use std::time::SystemTime;
use sysinfo::Disks;
use tauri::{AppHandle, Emitter};
use twox_hash::XxHash64;
use walkdir::WalkDir;

use crate::error::AppError;
use crate::model::{self, BackendState, InitDisk};

const LIVE_FILE_BATCH_SIZE: u64 = 500;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScanEventMode {
    Silent,
    Live,
}

#[derive(Clone, serde::Serialize)]
pub struct LiveScanEntryEvent {
    pub entry_type: String,
    pub path: String,
    pub parent_path: Option<String>,
    pub name: String,
    pub id: String,
    pub size: u64,
    pub num_files: u64,
    pub num_subdir: u64,
    pub created: SystemTime,
    pub modified: SystemTime,
}

#[derive(Clone, serde::Serialize)]
pub struct LiveScanFileBatchUpdate {
    pub parent_path: String,
    pub size: u64,
    pub file_count: u64,
}

#[derive(Clone, serde::Serialize)]
pub struct LiveScanFileBatchEvent {
    pub updates: Vec<LiveScanFileBatchUpdate>,
    pub entry_count: u64,
}

pub fn hash_path_id(path: &str) -> u64 {
    let seed = 420;
    let hash = XxHash64::oneshot(seed, path.as_bytes()); // need as bytes since &str is same bytes but typing says it is bytes that are text
    hash
}

fn path_to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}

fn emit_live_scan_entry(app: &AppHandle, event: LiveScanEntryEvent) -> Result<(), AppError> {
    app.emit("live-scan-entry", event)?;
    Ok(())
}

fn flush_live_file_batch(
    app: &AppHandle,
    pending_file_updates: &mut HashMap<String, (u64, u64)>,
    pending_file_count: &mut u64,
    force: bool,
) -> Result<(), AppError> {
    if *pending_file_count == 0 || (!force && *pending_file_count < LIVE_FILE_BATCH_SIZE) {
        return Ok(());
    }

    let updates = pending_file_updates
        .drain()
        .map(|(parent_path, (size, file_count))| LiveScanFileBatchUpdate {
            parent_path,
            size,
            file_count,
        })
        .collect();

    app.emit(
        "live-scan-file-batch",
        LiveScanFileBatchEvent {
            updates,
            entry_count: *pending_file_count,
        },
    )?;

    *pending_file_count = 0;
    Ok(())
}

#[tauri::command]
pub fn retreive_disks() -> Result<Vec<InitDisk>, AppError> {
    let disks = Disks::new_with_refreshed_list();
    let mut disk_list = Vec::new();

    if disks.is_empty() {
        return Err(AppError::GeneralLogicalErr(
            "No disks found while retrieving disks".to_string(),
        ));
    }

    for disk in &disks {
        let total_size = format_size(disk.total_space(), DECIMAL);
        let size_remaining = format_size(disk.available_space(), DECIMAL);

        disk_list.push(InitDisk {
            name: disk.mount_point().to_string_lossy().to_string(),
            desc: format!(
                "{} free {} total {}",
                disk.mount_point().to_string_lossy().to_string(),
                size_remaining,
                total_size,
            ),
        });
    }

    Ok(disk_list)
}

// FE currently sends in flags and for backend to do different actions but In the future
// should refactor to make multiple diff BE function for diff task and frontend calls it then
// this way more separation of concern
#[tauri::command]
pub async fn disk_scan(
    target: String,
    state: tauri::State<'_, BackendState>,
    app: AppHandle,
    snapshot_file: String,
    snapshot_flag: bool,
) -> Result<model::DirView, AppError> {
    let root = match naive_scan_with_events(&target, app, ScanEventMode::Live) {
        Ok(root) => root,
        Err(e) => return Err(e),
    };

    let root_view = match snapshot_flag {
        true => root.to_dir_view_unexpanded(state.clone(), snapshot_file)?,
        false => root.to_dir_view_unexpanded_no_diff(),
    };

    // make the global state have the FS object
    let mut file_tree = state.file_tree.lock().unwrap();
    *file_tree = Some(root); // deref the mutex guard then assign

    Ok(root_view)
}

#[tauri::command]
// maybe make this async?
pub fn query_new_dir_object(
    path_list: Vec<String>,
    state: tauri::State<BackendState>,
    snapshot_flag: bool,
    prev_snapshot_file_path: String, // < - frontend manages what snapshto file to compare to and it sends in that path to here to query
) -> Result<model::DirViewChildren, AppError> {
    // This asks state for file_tree mutex and locks it to become mutexguard holding Option<Dir>
    let file_tree = state.file_tree.lock().unwrap();

    if let Some(root_dir) = file_tree.as_ref() {
        // using as ref for &Dir we dont want to take ownership of the Dir from the Global State

        let mut current_dir = root_dir; // temp variable, needs to be mut, a mutable ref to a ref of dir, basically you can reassign this var to different ref of Dirs but cannot modify them since not &mut Dir

        for part in &path_list {
            current_dir = current_dir.subdirs.get(part).ok_or_else(|| {
                AppError::GeneralLogicalErr(format!(
                    "Requested query path has word {} which was not found in that directory",
                    part
                ))
            })?;
        }

        if snapshot_flag == false {
            Ok(current_dir.get_subdir_and_files_no_diff())
        } else {
            current_dir.get_subdir_and_files(state.clone(), prev_snapshot_file_path)
        }
    } else {
        Err(AppError::GeneralLogicalErr(
            "There is no root Dir object in backend memory state".to_string(),
        ))
    }
}

pub fn naive_scan(target: &str, app: AppHandle) -> Result<model::Dir, AppError> {
    naive_scan_with_events(target, app, ScanEventMode::Silent)
}

pub fn naive_scan_with_events(
    target: &str,
    app: AppHandle,
    event_mode: ScanEventMode,
) -> Result<model::Dir, AppError> {
    let mut hash_store_dir: HashMap<PathBuf, model::Dir> = HashMap::new();
    let mut hash_store_file: HashMap<PathBuf, model::File> = HashMap::new();
    let mut pending_file_updates: HashMap<String, (u64, u64)> = HashMap::new();
    let mut pending_file_count: u64 = 0;

    let walker = WalkDir::new(target)
        .contents_first(true)
        .same_file_system(true)
        .follow_links(false);

    let mut test_entry_progress_counter: u64 = 0;

    for entry_result in walker {
        if let Ok(entry) = entry_result {
            test_entry_progress_counter += 1; // [TEMP]
            if event_mode == ScanEventMode::Live && test_entry_progress_counter % 10000 == 0 {
                app.emit("progress", test_entry_progress_counter)?;
            }

            if entry.file_type().is_file() {
                if let Ok(file_meta) = entry.metadata() {
                    let entry_path = entry.path().to_path_buf();
                    let entry_parent_path = entry_path.parent().map(path_to_string);

                    // Create a file node with its details and push to the hashmap for it
                    let new_file_node = model::File {
                        meta: model::FileMeta {
                            size: file_meta.len(),
                            created: file_meta.created().unwrap_or(SystemTime::UNIX_EPOCH),
                            modified: file_meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH),
                        },
                        name: entry.file_name().to_string_lossy().to_string(),
                        id: {
                            if let Some(temp) = entry.path().to_str() {
                                hash_path_id(temp)
                            } else {
                                hash_path_id("err")
                            }
                        },
                    };

                    if event_mode == ScanEventMode::Live {
                        if let Some(parent_path) = entry_parent_path {
                            let update = pending_file_updates.entry(parent_path).or_insert((0, 0));
                            update.0 += new_file_node.meta.size;
                            update.1 += 1;
                            pending_file_count += 1;

                            flush_live_file_batch(
                                &app,
                                &mut pending_file_updates,
                                &mut pending_file_count,
                                false,
                            )?;
                        }
                    }

                    hash_store_file.insert(entry_path, new_file_node);
                }
            } else if entry.file_type().is_dir() {
                if let Ok(directory_meta) = entry.metadata() {
                    let entry_path = entry.path().to_path_buf();
                    let entry_path_string = path_to_string(&entry_path);
                    let entry_parent_path = entry_path.parent().map(path_to_string);
                    let mut current_dir_size: u64 = 0;

                    if let Ok(temp_fs_read_dir) = fs::read_dir(entry_path.clone()) {
                        let mut new_dir_node = model::Dir {
                            name: entry.file_name().to_string_lossy().to_string(),
                            files: HashMap::new(),
                            subdirs: HashMap::new(),
                            meta: model::DirMeta {
                                size: 0, // file.metadata does not give full size so to calc manually set to 0 on creation
                                created: directory_meta.created().unwrap_or(SystemTime::UNIX_EPOCH),
                                modified: directory_meta
                                    .accessed()
                                    .unwrap_or(SystemTime::UNIX_EPOCH),
                                num_files: 0, // I believe you can get these from the previous entry variable
                                num_subdir: 0,
                            },
                            id: {
                                if let Some(temp) = entry.path().to_str() {
                                    hash_path_id(temp)
                                } else {
                                    hash_path_id("thereisnothingthisisjusttestchangelater")
                                }
                            },
                        };

                        for temp_entry_result in temp_fs_read_dir {
                            if let Ok(temp_entry) = temp_entry_result {
                                if let Ok(temp_entry_type) = temp_entry.file_type() {
                                    if temp_entry_type.is_dir() {
                                        if let Some(hashed_dir) =
                                            hash_store_dir.remove(&temp_entry.path())
                                        {
                                            current_dir_size += hashed_dir.meta.size; // need to increment first as push makes the struct take ownership
                                                                                      // new_dir_node.subdirs.(hashed_dir);
                                            new_dir_node
                                                .subdirs
                                                .insert(hashed_dir.name.clone(), hashed_dir);
                                        }
                                    } else if temp_entry_type.is_file() {
                                        if let Some(hashed_file) =
                                            hash_store_file.remove(&temp_entry.path())
                                        {
                                            current_dir_size += hashed_file.meta.size;
                                            // new_dir_node.files.push(hashed_file);
                                            new_dir_node
                                                .files
                                                .insert(hashed_file.name.clone(), hashed_file);
                                        }
                                    }
                                    // originally ther was na else statement here
                                }
                            }
                        }

                        // Note
                        // files and subdirs are not recursively counted here. It is local to that specific directory.
                        // Size However is accumulated recursively
                        new_dir_node.meta.num_files = new_dir_node.files.len() as u64; // usize to u64
                        new_dir_node.meta.num_subdir = new_dir_node.subdirs.len() as u64;
                        new_dir_node.meta.size = current_dir_size; // accumulated length should be here

                        if event_mode == ScanEventMode::Live {
                            flush_live_file_batch(
                                &app,
                                &mut pending_file_updates,
                                &mut pending_file_count,
                                true,
                            )?;

                            emit_live_scan_entry(
                                &app,
                                LiveScanEntryEvent {
                                    entry_type: "directory".to_string(),
                                    path: entry_path_string,
                                    parent_path: entry_parent_path,
                                    name: new_dir_node.name.clone(),
                                    id: new_dir_node.id.to_string(),
                                    size: new_dir_node.meta.size,
                                    num_files: new_dir_node.meta.num_files,
                                    num_subdir: new_dir_node.meta.num_subdir,
                                    created: new_dir_node.meta.created,
                                    modified: new_dir_node.meta.modified,
                                },
                            )?;
                        }

                        hash_store_dir.insert(entry_path, new_dir_node);
                    }
                }
            }
            // Originially there was an else block here but choose to ignore symlinks and other stuff
        }
    }

    if event_mode == ScanEventMode::Live {
        flush_live_file_batch(
            &app,
            &mut pending_file_updates,
            &mut pending_file_count,
            true,
        )?;
    }

    // If the structure of the disk scanned then it has only 1 root
    if hash_store_dir.len() == 1 {
        // iter is view only
        // into_iter consumes/takes ownership of it
        let (_root_name, root) = hash_store_dir.into_iter().next().unwrap();

        return Ok(root);
    } else {
        Err(AppError::GeneralLogicalErr(
            "During scan a single root directory for the disk does not exist".to_string(),
        ))
    }
}
