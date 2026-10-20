import { native } from '../../native/bridge';
import type { FsRoot, Listing } from './filesModel';

/** Thin typed wrappers over the Rust `fs_*` commands. All paths are root-relative. */
export const filesApi = {
  roots: () => native<FsRoot[]>('fs_roots'),
  list: (root: string, path: string) => native<Listing>('fs_list', { root, path }),
  mkdir: (root: string, path: string) => native<void>('fs_mkdir', { root, path }),
  createFile: (root: string, path: string) => native<void>('fs_create_file', { root, path }),
  rename: (root: string, path: string, newName: string) =>
    native<string>('fs_rename', { root, path, newName }),
  copy: (root: string, path: string, destRoot: string, destDir: string) =>
    native<string>('fs_copy', { root, path, destRoot, destDir }),
  move: (root: string, path: string, destRoot: string, destDir: string) =>
    native<string>('fs_move', { root, path, destRoot, destDir }),
  trash: (root: string, path: string) => native<void>('fs_trash', { root, path }),
  readText: (root: string, path: string) => native<string>('fs_read_text', { root, path }),
  readBinary: (root: string, path: string) => native<ArrayBuffer>('fs_read_binary', { root, path }),
  writeText: (root: string, path: string, text: string) =>
    native<void>('fs_write_text', { root, path, text }),
  openExternal: (root: string, path: string) => native<void>('fs_open_external', { root, path }),
  addRoot: () => native<FsRoot | null>('fs_add_root'),
  removeRoot: (id: string) => native<void>('fs_remove_root', { id }),
};

/** Tauri rejects with a plain string; normalise to a message. */
export const errorMessage = (e: unknown): string =>
  typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong';
