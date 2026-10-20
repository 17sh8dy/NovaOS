import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ClipboardPaste,
  Copy,
  Download,
  File as FileIcon,
  FilePlus,
  Folder,
  FolderPlus,
  HardDrive,
  Link2,
  Pencil,
  Scissors,
  Trash2,
  TriangleAlert,
} from 'lucide-react';
import { create } from 'zustand';
import { launchApp } from '../../kernel/launcher';
import { errorMessage, filesApi } from './filesApi';
import { useSettings } from '../../settings/store';
import { ImportDialog } from './ImportDialog';
import {
  back,
  breadcrumbs,
  canBack,
  canForward,
  currentLocation,
  forward,
  formatSize,
  initialHistory,
  isLinkRoot,
  joinPath,
  navigate,
  openerFor,
  parentPath,
  type FsEntry,
  type FsRoot,
  type History,
  type Location,
} from './filesModel';

interface FileClipboard {
  op: 'copy' | 'cut' | null;
  root: string;
  path: string;
  set: (op: 'copy' | 'cut', root: string, path: string) => void;
  clear: () => void;
}

/** Shared across Files windows so you can copy in one and paste in another. */
export const useFileClipboard = create<FileClipboard>((set) => ({
  op: null,
  root: '',
  path: '',
  set: (op, root, path) => set({ op, root, path }),
  clear: () => set({ op: null, root: '', path: '' }),
}));

type Prompt = { kind: 'folder' | 'file' | 'rename'; value: string } | null;

export function Files() {
  const [roots, setRoots] = useState<FsRoot[]>([]);
  const [history, setHistory] = useState<History | null>(null);
  const [entries, setEntries] = useState<FsEntry[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [prompt, setPrompt] = useState<Prompt>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [confirmTrash, setConfirmTrash] = useState<string | null>(null);
  const { confirmDelete, showSizes } = useSettings((s) => s.data.files);
  const clip = useFileClipboard();
  const loadSeq = useRef(0);

  const loc: Location | null = history ? currentLocation(history) : null;

  useEffect(() => {
    filesApi
      .roots()
      .then((r) => {
        setRoots(r);
        if (r.length > 0) setHistory(initialHistory({ root: r[0].id, path: '' }));
      })
      .catch((e) => setError(errorMessage(e)));
  }, []);

  const refresh = useCallback(async () => {
    if (!loc) return;
    const seq = ++loadSeq.current;
    try {
      const listing = await filesApi.list(loc.root, loc.path);
      if (seq !== loadSeq.current) return; // a newer navigation superseded this one
      setEntries(listing.entries);
      setTruncated(listing.truncated);
    } catch (e) {
      if (seq !== loadSeq.current) return;
      setEntries([]);
      setError(errorMessage(e));
    }
  }, [loc]);

  useEffect(() => {
    setSelected(null);
    setPrompt(null);
    void refresh();
  }, [refresh]);

  const go = (target: Location) => {
    setError(null);
    setHistory((h) => (h ? navigate(h, target) : h));
  };

  /** Run a filesystem action, surfacing any error, then reload the listing. */
  const act = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorMessage(e));
    }
    await refresh();
  };

  /** Text opens in Nova Editor, images in Nova Viewer, the rest in the host's default app. */
  const openFile = (root: string, path: string, name: string) => {
    const opener = openerFor(name);
    if (opener === 'editor') launchApp('editor', { root, path });
    else if (opener === 'viewer') launchApp('viewer', { root, path });
    else void act(() => filesApi.openExternal(root, path));
  };

  const open = (entry: FsEntry) => {
    if (!loc) return;
    const path = joinPath(loc.path, entry.name);
    if (entry.is_dir) go({ root: loc.root, path });
    else openFile(loc.root, path, entry.name);
  };

  /** A linked single file has no folder to browse: selecting it opens the file itself. */
  const openRoot = (r: FsRoot) => {
    if (r.kind === 'linkfile') openFile(r.id, '', r.label);
    else go({ root: r.id, path: '' });
  };

  const reloadRoots = () => {
    filesApi
      .roots()
      .then(setRoots)
      .catch(() => undefined);
    void refresh();
  };

  const submitPrompt = async () => {
    if (!loc || !prompt) return;
    const name = prompt.value.trim();
    if (!name) return;
    setPrompt(null);
    if (prompt.kind === 'rename' && selected) {
      await act(() => filesApi.rename(loc.root, joinPath(loc.path, selected), name));
      setSelected(name);
    } else if (prompt.kind === 'folder') {
      await act(() => filesApi.mkdir(loc.root, joinPath(loc.path, name)));
    } else if (prompt.kind === 'file') {
      await act(() => filesApi.createFile(loc.root, joinPath(loc.path, name)));
    }
  };

  const trashNow = (name: string) => {
    if (!loc) return;
    setSelected(null);
    setConfirmTrash(null);
    void act(() => filesApi.trash(loc.root, joinPath(loc.path, name)));
  };

  /** Asks first when "Confirm before deleting" is on; deleted items always go to the Recycle Bin. */
  const doTrash = () => {
    if (!loc || !selected) return;
    if (confirmDelete) setConfirmTrash(selected);
    else trashNow(selected);
  };

  const doPaste = () => {
    if (!loc || !clip.op) return;
    const { op, root, path } = clip;
    void act(async () => {
      if (op === 'copy') await filesApi.copy(root, path, loc.root, loc.path);
      else {
        await filesApi.move(root, path, loc.root, loc.path);
        clip.clear();
      }
    });
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (prompt || !loc) return;
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === 'Delete') doTrash();
    else if (e.key === 'F2' && selected) setPrompt({ kind: 'rename', value: selected });
    else if (e.key === 'Backspace') go({ root: loc.root, path: parentPath(loc.path) });
    else if (e.key === 'Enter' && selected) {
      const entry = entries.find((x) => x.name === selected);
      if (entry) open(entry);
    } else if (mod && e.key.toLowerCase() === 'c' && selected)
      clip.set('copy', loc.root, joinPath(loc.path, selected));
    else if (mod && e.key.toLowerCase() === 'x' && selected)
      clip.set('cut', loc.root, joinPath(loc.path, selected));
    else if (mod && e.key.toLowerCase() === 'v') doPaste();
    else return;
    e.preventDefault();
  };

  const currentRoot = roots.find((r) => r.id === loc?.root);
  const rootLabel = roots.find((r) => r.id === loc?.root)?.label ?? '';
  const crumbs = loc ? breadcrumbs(loc.path) : [];
  const hasSel = selected !== null;

  return (
    <div className="relative flex h-full text-sm" data-testid="files">
      <nav
        aria-label="Locations"
        className="w-40 shrink-0 space-y-0.5 overflow-auto border-r border-white/10 p-2"
      >
        {roots
          .filter((r) => !isLinkRoot(r))
          .map((r) => (
            <RootButton key={r.id} r={r} active={loc?.root === r.id} onClick={() => openRoot(r)} />
          ))}
        {roots.some(isLinkRoot) && (
          <>
            <p
              className="flex items-center gap-1 px-2 pt-3 pb-1 text-[10px] font-semibold tracking-wider text-amber-300/80 uppercase"
              title="Linked items are your original files. Changes here affect them."
            >
              <TriangleAlert size={10} /> Linked
            </p>
            {roots.filter(isLinkRoot).map((r) => (
              <RootButton
                key={r.id}
                r={r}
                active={loc?.root === r.id}
                onClick={() => openRoot(r)}
              />
            ))}
          </>
        )}
      </nav>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center gap-1 border-b border-white/10 p-1.5">
          <Tool
            label="Back"
            disabled={!history || !canBack(history)}
            onClick={() => setHistory((h) => h && back(h))}
          >
            <ArrowLeft size={15} />
          </Tool>
          <Tool
            label="Forward"
            disabled={!history || !canForward(history)}
            onClick={() => setHistory((h) => h && forward(h))}
          >
            <ArrowRight size={15} />
          </Tool>
          <Tool
            label="Up"
            disabled={!loc || loc.path === ''}
            onClick={() => loc && go({ root: loc.root, path: parentPath(loc.path) })}
          >
            <ArrowUp size={15} />
          </Tool>
          <ol
            aria-label="Path"
            className="mx-1 flex min-w-0 flex-1 items-center gap-1 truncate rounded-md bg-black/25 px-2 py-1 text-xs"
          >
            <li>
              <button
                onClick={() => loc && go({ root: loc.root, path: '' })}
                className="hover:underline"
              >
                {rootLabel}
              </button>
            </li>
            {crumbs.map((c) => (
              <li key={c.path} className="flex items-center gap-1">
                <span className="text-white/30">/</span>
                <button
                  onClick={() => loc && go({ root: loc.root, path: c.path })}
                  className="truncate hover:underline"
                >
                  {c.name}
                </button>
              </li>
            ))}
          </ol>
          <Tool label="Import files or folders" disabled={!loc} onClick={() => setImportOpen(true)}>
            <Download size={15} />
          </Tool>
          <Tool
            label="New folder"
            disabled={!loc}
            onClick={() => setPrompt({ kind: 'folder', value: '' })}
          >
            <FolderPlus size={15} />
          </Tool>
          <Tool
            label="New text file"
            disabled={!loc}
            onClick={() => setPrompt({ kind: 'file', value: '' })}
          >
            <FilePlus size={15} />
          </Tool>
          <Tool
            label="Rename"
            disabled={!hasSel}
            onClick={() => selected && setPrompt({ kind: 'rename', value: selected })}
          >
            <Pencil size={15} />
          </Tool>
          <Tool
            label="Copy"
            disabled={!hasSel}
            onClick={() =>
              loc && selected && clip.set('copy', loc.root, joinPath(loc.path, selected))
            }
          >
            <Copy size={15} />
          </Tool>
          <Tool
            label="Cut"
            disabled={!hasSel}
            onClick={() =>
              loc && selected && clip.set('cut', loc.root, joinPath(loc.path, selected))
            }
          >
            <Scissors size={15} />
          </Tool>
          <Tool label="Paste" disabled={!clip.op || !loc} onClick={doPaste}>
            <ClipboardPaste size={15} />
          </Tool>
          <Tool label="Delete" danger disabled={!hasSel} onClick={doTrash}>
            <Trash2 size={15} />
          </Tool>
        </div>

        {currentRoot && isLinkRoot(currentRoot) && (
          <div
            role="note"
            className="flex items-center gap-2 border-b border-amber-400/30 bg-amber-400/10 px-3 py-1.5 text-xs text-amber-100"
          >
            <TriangleAlert size={13} className="shrink-0" />
            Linked folder: these are your original files. Edits, renames and deletes here change
            them on your computer.
          </div>
        )}

        {prompt && (
          <form
            className="flex items-center gap-2 border-b border-white/10 bg-white/5 p-2"
            onSubmit={(e) => {
              e.preventDefault();
              void submitPrompt();
            }}
          >
            <label className="text-xs text-white/60" htmlFor="files-prompt">
              {prompt.kind === 'rename'
                ? 'Rename to'
                : prompt.kind === 'folder'
                  ? 'New folder'
                  : 'New file'}
            </label>
            <input
              id="files-prompt"
              autoFocus
              value={prompt.value}
              onChange={(e) => setPrompt({ ...prompt, value: e.target.value })}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === 'Escape') setPrompt(null);
              }}
              className="flex-1 rounded-md bg-black/30 px-2 py-1 text-xs outline-none"
            />
            <button
              type="submit"
              className="rounded-md bg-[var(--accent)] px-3 py-1 text-xs text-white"
            >
              OK
            </button>
          </form>
        )}

        {confirmTrash && (
          <div
            role="alertdialog"
            aria-label="Confirm delete"
            className="flex items-center gap-2 border-b border-white/10 bg-rose-500/10 p-2 text-xs"
          >
            <span className="flex-1 truncate">Move “{confirmTrash}” to the Recycle Bin?</span>
            <button
              onClick={() => trashNow(confirmTrash)}
              className="rounded-md bg-rose-500/80 px-3 py-1 text-white hover:bg-rose-500"
            >
              Delete
            </button>
            <button
              onClick={() => setConfirmTrash(null)}
              className="rounded-md bg-white/10 px-3 py-1 hover:bg-white/20"
            >
              Cancel
            </button>
          </div>
        )}

        {error && (
          <div
            role="alert"
            className="flex items-center justify-between gap-2 bg-rose-500/20 px-3 py-1.5 text-xs text-rose-200"
          >
            <span>{error}</span>
            <button aria-label="Dismiss error" onClick={() => setError(null)} className="px-1">
              ×
            </button>
          </div>
        )}

        <div
          tabIndex={0}
          onKeyDown={onKeyDown}
          className="min-h-0 flex-1 overflow-auto outline-none"
          aria-label="Files"
        >
          {entries.length === 0 && !error && (
            <p className="p-6 text-center text-xs text-white/40">This folder is empty.</p>
          )}
          <ul>
            {entries.map((e) => (
              <li key={e.name}>
                <button
                  onClick={() => setSelected(e.name)}
                  onDoubleClick={() => open(e)}
                  aria-pressed={selected === e.name}
                  className={`flex w-full items-center gap-2 px-3 py-1.5 text-left ${
                    selected === e.name ? 'bg-[var(--accent)]/30' : 'hover:bg-white/10'
                  }`}
                >
                  {e.is_dir ? (
                    <Folder size={15} className="shrink-0 text-[var(--accent)]" />
                  ) : (
                    <FileIcon size={15} className="shrink-0 text-white/60" />
                  )}
                  <span className="flex-1 truncate">{e.name}</span>
                  {e.is_symlink && <span className="text-[10px] text-white/40">link</span>}
                  <span className="w-16 text-right text-xs text-white/40">
                    {e.is_dir || !showSizes ? '' : formatSize(e.size)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {truncated && (
            <p className="p-2 text-center text-xs text-white/40">Showing the first 10,000 items.</p>
          )}
        </div>

        <div className="border-t border-white/10 px-3 py-1 text-xs text-white/40">
          {entries.length} item{entries.length === 1 ? '' : 's'}
          {clip.op && ` · ${clip.op === 'copy' ? 'copied' : 'cut'}: ${clip.path.split('/').pop()}`}
        </div>
      </div>

      {importOpen && (
        <ImportDialog
          onClose={() => setImportOpen(false)}
          onChanged={reloadRoots}
          onShowImports={() => go({ root: 'nova', path: 'Imports' })}
        />
      )}
    </div>
  );
}

function RootButton(props: { r: FsRoot; active: boolean; onClick: () => void }) {
  const { r } = props;
  const linked = isLinkRoot(r);
  return (
    <button
      onClick={props.onClick}
      aria-current={props.active}
      title={
        r.kind === 'nova'
          ? 'NovaOS user folder'
          : linked
            ? `${r.path}\nLinked: changes affect the original`
            : r.path
      }
      className={`flex w-full items-center gap-2 truncate rounded-md px-2 py-1.5 text-left text-xs ${
        props.active ? 'bg-white/15' : 'hover:bg-white/10'
      }`}
    >
      {linked ? (
        <Link2 size={14} className="shrink-0 text-amber-300" />
      ) : r.kind === 'host' ? (
        <HardDrive size={14} className="shrink-0" />
      ) : (
        <Folder size={14} className="shrink-0 text-[var(--accent)]" />
      )}
      <span className="truncate">{r.label}</span>
    </button>
  );
}

function Tool(props: {
  label: string;
  disabled?: boolean;
  danger?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      aria-label={props.label}
      title={props.label}
      disabled={props.disabled}
      onClick={props.onClick}
      className={`grid h-7 w-7 shrink-0 place-items-center rounded-md disabled:opacity-30 ${
        props.danger ? 'hover:bg-rose-500/70' : 'hover:bg-white/15'
      }`}
    >
      {props.children}
    </button>
  );
}
