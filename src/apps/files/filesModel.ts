export interface FsRoot {
  id: string;
  label: string;
  kind: 'nova' | 'host' | 'custom';
  path: string;
}

export interface FsEntry {
  name: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: number | null;
}

export interface Listing {
  entries: FsEntry[];
  truncated: boolean;
}

/** A place in the virtual filesystem: a root id and a `/`-separated path relative to it. */
export interface Location {
  root: string;
  path: string;
}

export const joinPath = (dir: string, name: string): string => (dir ? `${dir}/${name}` : name);

export function parentPath(path: string): string {
  const i = path.lastIndexOf('/');
  return i < 0 ? '' : path.slice(0, i);
}

export const baseName = (path: string): string => path.slice(path.lastIndexOf('/') + 1);

export function breadcrumbs(path: string): { name: string; path: string }[] {
  const out: { name: string; path: string }[] = [];
  let acc = '';
  for (const seg of path.split('/').filter(Boolean)) {
    acc = joinPath(acc, seg);
    out.push({ name: seg, path: acc });
  }
  return out;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

const TEXT_EXTENSIONS = new Set([
  'txt',
  'md',
  'markdown',
  'json',
  'csv',
  'log',
  'toml',
  'yaml',
  'yml',
  'xml',
  'html',
  'css',
  'js',
  'ts',
  'tsx',
  'jsx',
  'rs',
  'py',
  'ini',
  'cfg',
  'sh',
  'ps1',
  'bat',
]);

/** Files NovaOS edits itself instead of handing to the host. (Opening scripts here is safe: it only edits text.) */
export function isTextFile(name: string): boolean {
  const i = name.lastIndexOf('.');
  return i > 0 && TEXT_EXTENSIONS.has(name.slice(i + 1).toLowerCase());
}

// ---------------------------------------------------------------- navigation history

export interface History {
  stack: Location[];
  index: number;
}

export const initialHistory = (loc: Location): History => ({ stack: [loc], index: 0 });
export const currentLocation = (h: History): Location => h.stack[h.index];

/** Navigate to a new place; drops any forward entries. No-op if already there. */
export function navigate(h: History, loc: Location): History {
  const cur = currentLocation(h);
  if (cur.root === loc.root && cur.path === loc.path) return h;
  return { stack: [...h.stack.slice(0, h.index + 1), loc], index: h.index + 1 };
}

export const canBack = (h: History): boolean => h.index > 0;
export const canForward = (h: History): boolean => h.index < h.stack.length - 1;
export const back = (h: History): History => (canBack(h) ? { ...h, index: h.index - 1 } : h);
export const forward = (h: History): History => (canForward(h) ? { ...h, index: h.index + 1 } : h);
