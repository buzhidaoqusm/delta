export interface InitDisk {
  name: string;
  desc: string;
}

interface DirViewMetaDiff {
    new_dir_flag: boolean,
    deleted_dir_flag: boolean,
    previous_size: number,
    prev_num_files: number,
    prev_num_subdir: number,
}

interface FileViewMetaDiff {
    new_file_flag: boolean,
    deleted_file_flag: boolean,
    previous_size: number,
}

export interface DirViewMeta {
  size: number;
  num_files: number;
  num_subdir: number;

  diff?: DirViewMetaDiff;

  created: { secs_since_epoch: number, nanos_since_epoch: number };
  modified: { secs_since_epoch: number, nanos_since_epoch: number };
}

interface File {
  meta: FileViewMeta;
  name: string;
  id: string;
  path?: string;
}

export interface FileViewMeta {
  size: number;

  diff?: FileViewMetaDiff;

  created: { secs_since_epoch: number, nanos_since_epoch: number };
  modified: { secs_since_epoch: number, nanos_since_epoch: number };
}

export interface DirView {
  meta: DirViewMeta;
  name: string;
  id: string;
  path?: string;
}

export interface DirViewChildren {
  subdirviews: DirView[];
  files: File[];
}

export type AutoScanStatus =
  | "never_run"
  | "success_saved"
  | "success_skipped_threshold"
  | "skipped_interval"
  | "disabled"
  | "error";

export interface AutoScanConfig {
  enabled: boolean;
  target_path: string;
  interval_days: number;
  save_threshold_bytes: number;
  last_scan_at?: string | null;
  last_scan_size_bytes?: number | null;
  last_snapshot_file?: string | null;
  last_status: AutoScanStatus;
  last_error?: string | null;
}

export interface AppConfig {
  auto_scan: AutoScanConfig;
}
