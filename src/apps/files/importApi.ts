import { listen } from '@tauri-apps/api/event';
import { isTauri, native } from '../../native/bridge';

export interface SelectionItem {
  name: string;
  path: string;
  is_dir: boolean;
}
export interface SelectionInfo {
  id: string;
  items: SelectionItem[];
}

export interface ScanReport {
  files: number;
  dirs: number;
  bytes: number;
  symlinks: number;
  programs: number;
  unreadable: string[];
  unreadable_count: number;
  free_bytes: number;
  required_bytes: number;
  enough_space: boolean;
}

export type JobState = 'running' | 'done' | 'cancelled' | 'failed';

export interface Progress {
  job: string;
  state: JobState;
  items_done: number;
  items_total: number;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
  speed_bps: number;
  current: string;
  error_count: number;
  errors: string[];
  message: string | null;
}

export interface ImportRow {
  id: number;
  mode: 'copy' | 'link';
  name: string;
  source_path: string;
  dest_rel: string | null;
  is_dir: boolean;
  files: number;
  bytes: number;
  errors: number;
  status: 'running' | 'complete' | 'partial' | 'linked';
  created_at: number;
  available: boolean;
}

export type ImportMode = 'copy' | 'link';

/**
 * Importing host files. The UI never supplies a host path: `pick` opens the native dialog in
 * Rust and returns an opaque selection id that every later call refers to.
 */
export const importApi = {
  pick: (kind: 'files' | 'folder') => native<SelectionInfo | null>('import_pick', { kind }),
  scan: (id: string) => native<ScanReport>('import_scan', { id }),
  start: (id: string, mode: ImportMode, acknowledged: boolean) =>
    native<Progress>('import_start', { id, mode, acknowledged }),
  cancel: (id: string) => native<void>('import_cancel', { id }),
  discard: (id: string) => native<void>('import_discard', { id }),
  status: (id: string) => native<Progress | null>('import_status', { id }),
  list: () => native<ImportRow[]>('import_list'),
  unlink: (id: number) => native<void>('import_unlink', { id }),
};

/** Subscribe to live progress. Returns an unsubscribe function. */
export async function onImportProgress(cb: (p: Progress) => void): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<Progress>('import-progress', (e) => cb(e.payload));
}
