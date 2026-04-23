use chrono::Local;
use chrono::NaiveDateTime;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::disk::hash_path_id;
use crate::error::AppError;
use crate::model::{self, BackendState, Node, SnapshotDbMeta};
use crate::platform::clean_disk_name;

const SNAPSHOT_SCHEMA_VERSION_V2: u8 = 2;

pub struct SnapshotRecord {
    pub id: i64,
    pub size: i64, // sqlite limitation but should be big enough
    pub dir_flag: bool,
    pub sub_folder_count: i64,
    pub sub_file_count: i64,
}

struct SnapshotCapability {
    schema_version: u8,
    can_preview: bool,
    can_compare: bool,
    root_path: Option<String>,
}

#[derive(Clone)]
struct SnapshotEntryRow {
    id: i64,
    name: String,
    path: String,
    size: i64,
    dir_flag: bool,
    sub_folder_count: i64,
    sub_file_count: i64,
    created: i64,
    modified: i64,
}

struct SnapshotMetaRow {
    root_path: String,
    created_at: String,
}

fn system_time_to_secs(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn secs_to_system_time(secs: i64) -> SystemTime {
    if secs <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    }
}

fn snapshot_db_path(
    state: &tauri::State<'_, BackendState>,
    snapshot_file_name: &str,
) -> PathBuf {
    state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot")
        .join(format!("{}.db", snapshot_file_name))
}

fn table_exists(conn: &Connection, table_name: &str) -> Result<bool, AppError> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table_name],
        |row| row.get(0),
    )?;
    Ok(exists > 0)
}

fn legacy_snapshot_capability() -> SnapshotCapability {
    SnapshotCapability {
        schema_version: 1,
        can_preview: false,
        can_compare: false,
        root_path: None,
    }
}

fn read_snapshot_capability(db_path: &Path) -> Result<SnapshotCapability, AppError> {
    let conn = Connection::open(db_path)?;

    if !table_exists(&conn, "snapshot_meta")? || !table_exists(&conn, "snapshot_entries")? {
        return Ok(legacy_snapshot_capability());
    }

    let capability = conn
        .query_row(
            "SELECT schema_version, root_path FROM snapshot_meta LIMIT 1",
            [],
            |row| {
                let schema_version: i64 = row.get(0)?;
                let root_path: String = row.get(1)?;
                Ok(SnapshotCapability {
                    schema_version: schema_version as u8,
                    can_preview: schema_version as u8 >= SNAPSHOT_SCHEMA_VERSION_V2,
                    can_compare: schema_version as u8 >= SNAPSHOT_SCHEMA_VERSION_V2,
                    root_path: Some(root_path),
                })
            },
        )
        .optional()?;

    Ok(capability.unwrap_or_else(legacy_snapshot_capability))
}

fn query_snapshot_entry_by_parent_null(conn: &Connection) -> Result<SnapshotEntryRow, AppError> {
    let entry = conn.query_row(
        "SELECT id, name, path, size, dir_flag, sub_folder_count, sub_file_count, created, modified
         FROM snapshot_entries
         WHERE parent_id IS NULL
         LIMIT 1",
        [],
        row_to_snapshot_entry,
    )?;

    Ok(entry)
}

fn query_snapshot_children(
    conn: &Connection,
    parent_id: i64,
) -> Result<Vec<SnapshotEntryRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, path, size, dir_flag, sub_folder_count, sub_file_count, created, modified
         FROM snapshot_entries
         WHERE parent_id = ?1",
    )?;

    let rows = stmt.query_map([parent_id], row_to_snapshot_entry)?;
    let mut entries = Vec::new();

    for row in rows {
        entries.push(row?);
    }

    Ok(entries)
}

fn row_to_snapshot_entry(row: &rusqlite::Row<'_>) -> Result<SnapshotEntryRow, rusqlite::Error> {
    Ok(SnapshotEntryRow {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        size: row.get(3)?,
        dir_flag: row.get(4)?,
        sub_folder_count: row.get(5)?,
        sub_file_count: row.get(6)?,
        created: row.get(7)?,
        modified: row.get(8)?,
    })
}

fn read_snapshot_meta(db_path: &Path) -> Result<SnapshotMetaRow, AppError> {
    let conn = Connection::open(db_path)?;
    let meta = conn.query_row(
        "SELECT schema_version, root_path, root_name, created_at, total_size
         FROM snapshot_meta
         LIMIT 1",
        [],
        |row| {
            Ok(SnapshotMetaRow {
                root_path: row.get(1)?,
                created_at: row.get(3)?,
            })
        },
    )?;

    Ok(meta)
}

fn validate_v2_snapshot(db_path: &Path) -> Result<SnapshotMetaRow, AppError> {
    let capability = read_snapshot_capability(db_path)?;
    if !capability.can_compare {
        return Err(AppError::GeneralLogicalErr(
            "Old snapshots cannot be compared. Create new snapshots to use compare.".to_string(),
        ));
    }

    read_snapshot_meta(db_path)
}

fn order_snapshot_paths_by_created_at(
    first_path: PathBuf,
    second_path: PathBuf,
) -> Result<(PathBuf, PathBuf), AppError> {
    let first_meta = read_snapshot_meta(&first_path)?;
    let second_meta = read_snapshot_meta(&second_path)?;

    if first_meta.created_at >= second_meta.created_at {
        Ok((first_path, second_path))
    } else {
        Ok((second_path, first_path))
    }
}

fn entry_to_dir_view_with_diff(
    entry: SnapshotEntryRow,
    previous: Option<&SnapshotEntryRow>,
    new_flag: bool,
    deleted_flag: bool,
) -> model::DirView {
    model::DirView {
        meta: model::DirViewMeta {
            size: entry.size as u64,
            num_files: entry.sub_file_count as u64,
            num_subdir: entry.sub_folder_count as u64,
            diff: Some(model::DirViewMetaDiff {
                new_dir_flag: new_flag,
                deleted_dir_flag: deleted_flag,
                previous_size: previous.map(|row| row.size as u64).unwrap_or(0),
                prev_num_files: previous
                    .map(|row| row.sub_file_count as u64)
                    .unwrap_or(0),
                prev_num_subdir: previous
                    .map(|row| row.sub_folder_count as u64)
                    .unwrap_or(0),
            }),
            created: secs_to_system_time(entry.created),
            modified: secs_to_system_time(entry.modified),
        },
        name: entry.name,
        id: entry.id.to_string(),
        path: Some(entry.path),
    }
}

fn entry_to_file_view_with_diff(
    entry: SnapshotEntryRow,
    previous: Option<&SnapshotEntryRow>,
    new_flag: bool,
    deleted_flag: bool,
) -> model::FileView {
    model::FileView {
        meta: model::FileViewMeta {
            size: entry.size as u64,
            diff: Some(model::FileViewMetaDiff {
                new_file_flag: new_flag,
                deleted_file_flag: deleted_flag,
                previous_size: previous.map(|row| row.size as u64).unwrap_or(0),
            }),
            created: secs_to_system_time(entry.created),
            modified: secs_to_system_time(entry.modified),
        },
        name: entry.name,
        id: entry.id.to_string(),
        path: Some(entry.path),
    }
}

fn diff_snapshot_children(
    newer_children: Vec<SnapshotEntryRow>,
    older_children: Vec<SnapshotEntryRow>,
) -> model::DirViewChildren {
    let mut older_by_id: HashMap<i64, SnapshotEntryRow> = older_children
        .into_iter()
        .map(|entry| (entry.id, entry))
        .collect();
    let mut dir_views = Vec::new();
    let mut file_views = Vec::new();

    for newer_entry in newer_children {
        let older_entry = older_by_id.remove(&newer_entry.id);
        let is_dir = newer_entry.dir_flag;
        let new_flag = older_entry.is_none();

        if is_dir {
            dir_views.push(entry_to_dir_view_with_diff(
                newer_entry,
                older_entry.as_ref(),
                new_flag,
                false,
            ));
        } else {
            file_views.push(entry_to_file_view_with_diff(
                newer_entry,
                older_entry.as_ref(),
                new_flag,
                false,
            ));
        }
    }

    for (_, older_entry) in older_by_id {
        if older_entry.dir_flag {
            dir_views.push(entry_to_dir_view_with_diff(
                older_entry,
                None,
                false,
                true,
            ));
        } else {
            file_views.push(entry_to_file_view_with_diff(
                older_entry,
                None,
                false,
                true,
            ));
        }
    }

    dir_views.sort_by_key(|entry| std::cmp::Reverse(entry.meta.size));
    file_views.sort_by_key(|entry| std::cmp::Reverse(entry.meta.size));

    model::DirViewChildren {
        subdirviews: dir_views,
        files: file_views,
    }
}

fn entry_to_dir_view(entry: SnapshotEntryRow) -> model::DirView {
    model::DirView {
        meta: model::DirViewMeta {
            size: entry.size as u64,
            num_files: entry.sub_file_count as u64,
            num_subdir: entry.sub_folder_count as u64,
            diff: None,
            created: secs_to_system_time(entry.created),
            modified: secs_to_system_time(entry.modified),
        },
        name: entry.name,
        id: entry.id.to_string(),
        path: Some(entry.path),
    }
}

fn entry_to_file_view(entry: SnapshotEntryRow) -> model::FileView {
    model::FileView {
        meta: model::FileViewMeta {
            size: entry.size as u64,
            diff: None,
            created: secs_to_system_time(entry.created),
            modified: secs_to_system_time(entry.modified),
        },
        name: entry.name,
        id: entry.id.to_string(),
        path: Some(entry.path),
    }
}

// This for query stats for a specific ID
// currently used when calling from a Dir object (Dir object .function get data)
// Overall not needed can both use utility way but just keeping both separate
pub fn query_stats_from_id(
    dir: &model::Dir,
    state: tauri::State<BackendState>,
    prev_snapshot_file_path: String,
) -> Result<SnapshotRecord, AppError> {
    let prev_data_db_path: std::path::PathBuf = state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot")
        .join(format!("{}.db", prev_snapshot_file_path));

    let default_record = SnapshotRecord {
        id: dir.id as i64,
        size: 0,
        dir_flag: true,
        sub_folder_count: 0,
        sub_file_count: 0,
    };

    let try_fetch = || -> Result<SnapshotRecord, rusqlite::Error> {
        let conn = Connection::open(&prev_data_db_path)?;

        let stats = conn.query_row(
            "SELECT * FROM snapshot WHERE id == ?1",
            [dir.id as i64], // this needs conv since id is u64 but sqllite cannot recog that
            |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    size: row.get(1)?,
                    dir_flag: row.get(2)?,
                    sub_folder_count: row.get(3)?,
                    sub_file_count: row.get(4)?,
                })
            },
        )?;

        Ok(stats)
    };

    let final_stats = try_fetch().unwrap_or(default_record);

    Ok(final_stats)
}

// Used as utility for any given hashed ID and correct path to DB file
// Will return row if there is, if there is not then throws error (no defaults)
pub fn query_stats_from_id_utility(id: u64, db_path: &Path) -> Result<SnapshotRecord, AppError> {
    let try_fetch = || -> Result<SnapshotRecord, rusqlite::Error> {
        let conn = Connection::open(&db_path)?;

        let stats = conn.query_row(
            "SELECT * FROM snapshot WHERE id == ?1",
            [id as i64], // this needs conv since id is u64 but sqllite cannot recog that
            |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    size: row.get(1)?,
                    dir_flag: row.get(2)?,
                    sub_folder_count: row.get(3)?,
                    sub_file_count: row.get(4)?,
                })
            },
        )?;

        Ok(stats)
    };

    let stats = try_fetch()?;

    Ok(stats)
}

// Needs a parameter for which db file to actually query from
pub fn query_children_stats_from_parent_id(
    parent_dir: &model::Dir,
    state: tauri::State<BackendState>,
    prev_snapshot_file_path: String,
) -> Result<HashMap<u64, SnapshotRecord>, AppError> {
    let prev_data_db_path: std::path::PathBuf = state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot")
        .join(format!("{}.db", prev_snapshot_file_path));

    let parent_id = parent_dir.id;

    let conn = Connection::open(&prev_data_db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM snapshot WHERE parent_id == ?")?;
    let mut rows = stmt.query([parent_id as i64])?; // rows match to snapshot record

    let mut temp_ht: HashMap<u64, SnapshotRecord> = HashMap::new();

    while let Some(row) = rows.next()? {
        let entry: SnapshotRecord = SnapshotRecord {
            id: (row.get(0)?),
            size: (row.get(1)?),
            dir_flag: (row.get(2)?),
            sub_folder_count: (row.get(3)?),
            sub_file_count: (row.get(4)?),
        };

        temp_ht.insert(entry.id as u64, entry); // for each row insert into the hash map
    }

    return Ok(temp_ht);
}

// write to that string as db file name, and the frontend is sending that name over
// TODO change selected_disk_letter to drive name! For linux need to handle it
#[tauri::command]
pub async fn write_current_tree(
    state: tauri::State<'_, BackendState>,
    selected_disk: String,
) -> Result<(), AppError> {
    let guard = state.file_tree.lock().unwrap();
    if guard.is_none() {
        return Ok(());
    }
    let root_ref = guard.as_ref().unwrap();
    let root_size_bytes = root_ref.meta.size;

    let local_time = Local::now();

    let selected_disk_name = clean_disk_name(&selected_disk)?;

    let temp_data_db_path = state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot")
        .join(format!(
            "{}_{}_{}.db",
            selected_disk_name,
            local_time.format("%Y%m%d%H%M").to_string(),
            root_size_bytes.to_string()
        ));

    let mut conn = Connection::open(&temp_data_db_path)?;

    // ?? Set Pragmas for speed (since this is temp data)
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;  
         PRAGMA cache_size = 10000;",
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS snapshot (
            id INTEGER PRIMARY KEY,
            size INTEGER NOT NULL,
            dir_flag INTEGER NOT NULL,
            sub_folder_count INTEGER DEFAULT 0,
            sub_file_count INTEGER DEFAULT 0,
            parent_id INTEGER
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS snapshot_meta (
            schema_version INTEGER NOT NULL,
            root_path TEXT NOT NULL,
            root_name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            total_size INTEGER NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS snapshot_entries (
            id INTEGER PRIMARY KEY,
            parent_id INTEGER,
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            size INTEGER NOT NULL,
            dir_flag INTEGER NOT NULL,
            sub_folder_count INTEGER DEFAULT 0,
            sub_file_count INTEGER DEFAULT 0,
            created INTEGER,
            modified INTEGER
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_snapshot_entries_parent_id
         ON snapshot_entries(parent_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_snapshot_entries_path
         ON snapshot_entries(path)",
        [],
    )?;

    let temp_transaction = conn.transaction()?;

    temp_transaction.execute(
        "INSERT INTO snapshot_meta (schema_version, root_path, root_name, created_at, total_size)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            SNAPSHOT_SCHEMA_VERSION_V2,
            selected_disk,
            root_ref.name,
            local_time.to_rfc3339(),
            root_size_bytes as i64
        ],
    )?;

    {
        let mut legacy_stmt = temp_transaction.prepare(
            "INSERT INTO snapshot (id, size, dir_flag, sub_folder_count, sub_file_count, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;

        let mut entry_stmt = temp_transaction.prepare(
            "INSERT INTO snapshot_entries
             (id, parent_id, name, path, size, dir_flag, sub_folder_count, sub_file_count, created, modified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;

        let mut stack = Vec::new();
        stack.push((Node::Dir(root_ref), None::<i64>, selected_disk.clone()));

        while let Some((node, real_parent_id, current_path)) = stack.pop() {
            let id: i64;
            let size: i64;
            let dir_flag: bool;
            let sub_folder_count: i64;
            let sub_file_count: i64;
            let name: String;
            let created: i64;
            let modified: i64;

            match node {
                Node::File(temp_file) => {
                    id = temp_file.id as i64;
                    size = temp_file.meta.size as i64;
                    dir_flag = false;
                    sub_folder_count = 0;
                    sub_file_count = 0;
                    name = temp_file.name.clone();
                    created = system_time_to_secs(temp_file.meta.created);
                    modified = system_time_to_secs(temp_file.meta.modified);
                }
                Node::Dir(temp_dir) => {
                    id = temp_dir.id as i64;
                    size = temp_dir.meta.size as i64;
                    dir_flag = true;
                    sub_folder_count = temp_dir.meta.num_subdir as i64;
                    sub_file_count = temp_dir.meta.num_files as i64;
                    name = temp_dir.name.clone();
                    created = system_time_to_secs(temp_dir.meta.created);
                    modified = system_time_to_secs(temp_dir.meta.modified);

                    for file in temp_dir.files.values() {
                        let child_path = Path::new(&current_path)
                            .join(&file.name)
                            .to_string_lossy()
                            .to_string();
                        stack.push((Node::File(file), Some(id), child_path));
                    }
                    for subdir in temp_dir.subdirs.values() {
                        let child_path = Path::new(&current_path)
                            .join(&subdir.name)
                            .to_string_lossy()
                            .to_string();
                        stack.push((Node::Dir(subdir), Some(id), child_path));
                    }
                }
            }

            legacy_stmt.execute(params![
                id,
                size,
                dir_flag,
                sub_folder_count,
                sub_file_count,
                real_parent_id.unwrap_or(0)
            ])?;

            entry_stmt.execute(params![
                id,
                real_parent_id,
                name,
                current_path,
                size,
                dir_flag,
                sub_folder_count,
                sub_file_count,
                created,
                modified
            ])?;
        }
    }

    temp_transaction.commit()?;

    Ok(())
}

#[tauri::command]
pub fn get_local_snapshot_files(
    state: tauri::State<'_, BackendState>,
) -> Result<Vec<SnapshotDbMeta>, AppError> {
    let temp_data_db_path = state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot");

    let mut vec_file_names = Vec::new();

    for entry in fs::read_dir(&temp_data_db_path)? {
        let entry = entry?; // entry is a Result
        let path = entry.path();

        if path.is_file() {
            let file_path_name = path
                .file_stem()
                .ok_or(AppError::CustomError(
                    "Path failed to get file stem".to_string(),
                ))?
                .to_string_lossy()
                .to_string(); // file stem removes the file extension

            let mut snapshot_meta = parse_snapshot_file_name(&file_path_name)?;
            let capability =
                read_snapshot_capability(&path).unwrap_or_else(|_| legacy_snapshot_capability());

            snapshot_meta.schema_version = capability.schema_version;
            snapshot_meta.can_preview = capability.can_preview;
            snapshot_meta.can_compare = capability.can_compare;
            snapshot_meta.root_path = capability.root_path;

            vec_file_names.push(snapshot_meta);
        }
    }

    return Ok(vec_file_names);
}

// For this func it should given a path name return the snapshot db file object
fn parse_snapshot_file_name(path: &String) -> Result<SnapshotDbMeta, AppError> {
    let path_segmented: Vec<&str> = path.split("_").collect();

    if path_segmented.len() != 3 {
        return Err(AppError::GeneralLogicalErr(
            "Invalid formatted snapshot file found in app local storage. Restart App.".to_string(),
        ));
    }

    if let [drive_name, date, size] = path_segmented.as_slice() {
        // naivedatetime parse from str should turn somethin like 20261220HHMM to a string
        let snapshot_meta = SnapshotDbMeta {
            drive_letter: drive_name.to_string(),
            date_time: NaiveDateTime::parse_from_str(date, "%Y%m%d%H%M")?.to_string(),
            date_sort_key: date.parse::<u64>()?,
            size: size.parse::<u64>()?,
            schema_version: 1,
            can_preview: false,
            can_compare: false,
            root_path: None,
        };

        return Ok(snapshot_meta);
    } else {
        return Err(AppError::GeneralLogicalErr(
            "Cannot parse malformed snapshot filename. Restart application".to_string(),
        ));
    };
}

#[tauri::command]
pub fn delete_snapshot_file(
    selected_row_file_name: String,
    state: tauri::State<'_, BackendState>,
) -> Result<bool, AppError> {
    let prev_data_db_path: std::path::PathBuf = state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot")
        .join(format!("{}.db", selected_row_file_name));

    fs::remove_file(prev_data_db_path)?;

    // Using fs delete also catch the error for that if needed on the passed in path (such as if X does not exist)

    Ok(true)
}

#[tauri::command]
pub fn open_snapshot_preview(
    snapshot_file_name: String,
    state: tauri::State<'_, BackendState>,
) -> Result<model::DirView, AppError> {
    let db_path = snapshot_db_path(&state, &snapshot_file_name);
    let capability = read_snapshot_capability(&db_path)?;

    if !capability.can_preview {
        return Err(AppError::GeneralLogicalErr(
            "Old snapshots cannot be previewed. Create a new snapshot to use preview.".to_string(),
        ));
    }

    let conn = Connection::open(db_path)?;
    let root_entry = query_snapshot_entry_by_parent_null(&conn)?;

    if !root_entry.dir_flag {
        return Err(AppError::GeneralLogicalErr(
            "Snapshot root entry is not a directory".to_string(),
        ));
    }

    Ok(entry_to_dir_view(root_entry))
}

#[tauri::command]
pub fn query_snapshot_dir_object(
    snapshot_file_name: String,
    parent_id: String,
    state: tauri::State<'_, BackendState>,
) -> Result<model::DirViewChildren, AppError> {
    let db_path = snapshot_db_path(&state, &snapshot_file_name);
    let capability = read_snapshot_capability(&db_path)?;

    if !capability.can_preview {
        return Err(AppError::GeneralLogicalErr(
            "Old snapshots cannot be previewed. Create a new snapshot to use preview.".to_string(),
        ));
    }

    let parent_id = parent_id.parse::<i64>()?;
    let conn = Connection::open(db_path)?;
    let mut dir_views = Vec::new();
    let mut file_views = Vec::new();

    for entry in query_snapshot_children(&conn, parent_id)? {
        if entry.dir_flag {
            dir_views.push(entry_to_dir_view(entry));
        } else {
            file_views.push(entry_to_file_view(entry));
        }
    }

    dir_views.sort_by_key(|entry| std::cmp::Reverse(entry.meta.size));
    file_views.sort_by_key(|entry| std::cmp::Reverse(entry.meta.size));

    Ok(model::DirViewChildren {
        subdirviews: dir_views,
        files: file_views,
    })
}

#[tauri::command]
pub fn compare_snapshots(
    first_snapshot_file_name: String,
    second_snapshot_file_name: String,
    state: tauri::State<'_, BackendState>,
) -> Result<model::DirView, AppError> {
    let first_path = snapshot_db_path(&state, &first_snapshot_file_name);
    let second_path = snapshot_db_path(&state, &second_snapshot_file_name);
    let first_meta = validate_v2_snapshot(&first_path)?;
    let second_meta = validate_v2_snapshot(&second_path)?;

    if first_meta.root_path != second_meta.root_path {
        return Err(AppError::GeneralLogicalErr(
            "Snapshots must belong to the same disk or root path.".to_string(),
        ));
    }

    let (newer_path, older_path) = order_snapshot_paths_by_created_at(first_path, second_path)?;
    let newer_conn = Connection::open(newer_path)?;
    let older_conn = Connection::open(older_path)?;
    let newer_root = query_snapshot_entry_by_parent_null(&newer_conn)?;
    let older_root = query_snapshot_entry_by_parent_null(&older_conn)?;

    Ok(entry_to_dir_view_with_diff(
        newer_root,
        Some(&older_root),
        false,
        false,
    ))
}

#[tauri::command]
pub fn query_snapshot_compare_dir_object(
    newer_snapshot_file_name: String,
    older_snapshot_file_name: String,
    parent_id: String,
    state: tauri::State<'_, BackendState>,
) -> Result<model::DirViewChildren, AppError> {
    let newer_path = snapshot_db_path(&state, &newer_snapshot_file_name);
    let older_path = snapshot_db_path(&state, &older_snapshot_file_name);
    let newer_meta = validate_v2_snapshot(&newer_path)?;
    let older_meta = validate_v2_snapshot(&older_path)?;

    if newer_meta.root_path != older_meta.root_path {
        return Err(AppError::GeneralLogicalErr(
            "Snapshots must belong to the same disk or root path.".to_string(),
        ));
    }

    let parent_id = parent_id.parse::<i64>()?;
    let newer_conn = Connection::open(newer_path)?;
    let older_conn = Connection::open(older_path)?;

    let newer_children = query_snapshot_children(&newer_conn, parent_id).unwrap_or_default();
    let older_children = query_snapshot_children(&older_conn, parent_id).unwrap_or_default();

    Ok(diff_snapshot_children(newer_children, older_children))
}

// pub fn get_path_historical_data(
//     root_path: String,
//     absolute_path: String,
//     state: tauri::State<'_, BackendState>,
// ) -> Result<Vec<(String, i64)>, AppError> {
//     let prev_data_db_path: std::path::PathBuf = state
//         .local_appdata_path
//         .as_ref()
//         .unwrap()
//         .join("tempsnapshot");

//     let cleaned_name = clean_disk_name(&root_path)?;
//     let id = hash_path_id(&absolute_path);

//     let mut data_vec: Vec<(String, i64)> = Vec::new();

//     for entry in fs::read_dir(&prev_data_db_path)? {
//         let entry_result = entry?;
//         let path = entry_result.path(); // abs path of each db file
//         let file_path_name = path
//             .file_stem()
//             .ok_or(AppError::CustomError(
//                 "Path failed to get file stem".to_string(),
//             ))?
//             .to_string_lossy()
//             .to_string();

//         let path_segmented: Vec<&str> = file_path_name.split('_').collect();

//         if let [drive_name, date, size] = path_segmented.as_slice() {
//             if *drive_name == cleaned_name {
//                 if let Ok(temp_states) = query_stats_from_id_utility(id, &path) {
//                     let parsed_date = NaiveDateTime::parse_from_str(date, "%Y%m%d%H%M")?;
//                     data_vec.push((
//                         // 2026-03-18 format
//                         parsed_date.format("%Y-%m-%d").to_string(),
//                         temp_states.size,
//                     ));
//                 }
//             }
//         } else {
//             return Err(AppError::GeneralLogicalErr(
//                 "Cannot parse malformed snapshot filename. Restart application".to_string(),
//             ));
//         }
//     }

//     data_vec.sort_by_key(|tuple| tuple.0.clone());

//     return Ok(data_vec);
// }


// This approach in some cases might be wastefully slow worse than the old solution
// at scale if user has 1000 then they wil need to wait about 1 second aroud to get the data back
// in the future maybe can think of optimize
#[tauri::command]
pub fn get_path_historical_data(
    root_path: String,
    absolute_path: String,
    state: tauri::State<'_, BackendState>,
) -> Result<Vec<(String, i64)>, AppError> {
    let prev_data_db_path: std::path::PathBuf = state
        .local_appdata_path
        .as_ref()
        .unwrap()
        .join("tempsnapshot");

    let id = hash_path_id(&absolute_path);

    let mut data_vec: Vec<(String, i64)> = Vec::new();

    for entry in fs::read_dir(&prev_data_db_path)? {
        let entry_result = entry?;
        let path = entry_result.path(); // abs path of each db file
        let file_path_name = path
            .file_stem()
            .ok_or(AppError::CustomError(
                "Path failed to get file stem".to_string(),
            ))?
            .to_string_lossy()
            .to_string();

        let path_segmented: Vec<&str> = file_path_name.split('_').collect();

        if let [_drive_name, date, _size] = path_segmented.as_slice() {
            if let Ok(temp_states) = query_stats_from_id_utility(id, &path) {
                let parsed_date = NaiveDateTime::parse_from_str(date, "%Y%m%d%H%M")?;
                data_vec.push((
                    // 2026-03-18 format
                    parsed_date.format("%Y-%m-%d").to_string(),
                    temp_states.size,
                ));
            }
        } else {
            // Put your error handling back here for malformed .db filenames
            return Err(AppError::GeneralLogicalErr(
                "Cannot parse malformed snapshot filename. Restart application".to_string(),
            ));
        }
    }

    data_vec.sort_by_key(|tuple| tuple.0.clone());

    return Ok(data_vec);
}
