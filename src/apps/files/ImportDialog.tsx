import { useEffect, useRef, useState } from 'react';
import { AlertTriangle, FileUp, FolderUp, Link2, Loader2, X } from 'lucide-react';
import { errorMessage } from './filesApi';
import { formatDuration, formatSize, formatSpeed } from './filesModel';
import {
  importApi,
  onImportProgress,
  type ImportMode,
  type ImportRow,
  type Progress,
  type ScanReport,
  type SelectionInfo,
} from './importApi';

type Step = 'choose' | 'scanning' | 'review' | 'running' | 'finished';

interface Props {
  onClose: () => void;
  /** Called when an import changed what Nova Files shows (copy finished or links added). */
  onChanged: () => void;
  /** Jump to the folder where copies land. */
  onShowImports: () => void;
}

export function ImportDialog({ onClose, onChanged, onShowImports }: Props) {
  const [step, setStep] = useState<Step>('choose');
  const [selection, setSelection] = useState<SelectionInfo | null>(null);
  const [scan, setScan] = useState<ScanReport | null>(null);
  const [mode, setMode] = useState<ImportMode>('copy');
  const [ack, setAck] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recent, setRecent] = useState<ImportRow[]>([]);
  const jobRef = useRef<string | null>(null);
  const finishedRef = useRef(false);

  const loadRecent = () =>
    importApi
      .list()
      .then(setRecent)
      .catch(() => undefined);

  useEffect(() => {
    void loadRecent();
  }, []);

  const finish = (p: Progress) => {
    if (finishedRef.current) return;
    finishedRef.current = true;
    setProgress(p);
    setStep('finished');
    onChanged();
    void loadRecent();
  };

  // Live progress from Rust, with a slow poll as a safety net in case the final event is missed.
  useEffect(() => {
    if (step !== 'running') return;
    let unlisten: (() => void) | undefined;
    let alive = true;
    void onImportProgress((p) => {
      if (p.job !== jobRef.current) return;
      if (p.state === 'running') setProgress(p);
      else finish(p);
    }).then((u) => {
      if (alive) unlisten = u;
      else u();
    });
    const poll = window.setInterval(() => {
      const job = jobRef.current;
      if (!job) return;
      void importApi
        .status(job)
        .then((p) => {
          if (!p) return;
          if (p.state === 'running') setProgress(p);
          else finish(p);
        })
        .catch(() => undefined);
    }, 1500);
    return () => {
      alive = false;
      unlisten?.();
      window.clearInterval(poll);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  const pick = async (kind: 'files' | 'folder') => {
    setError(null);
    try {
      const sel = await importApi.pick(kind);
      if (!sel) return; // user closed the native dialog
      setSelection(sel);
      setScan(null);
      setAck(false);
      setMode('copy');
      setStep('scanning');
      try {
        setScan(await importApi.scan(sel.id));
        setStep('review');
      } catch (e) {
        const msg = errorMessage(e);
        if (msg !== 'cancelled') setError(msg);
        setStep('choose');
        setSelection(null);
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const start = async () => {
    if (!selection) return;
    setError(null);
    try {
      const initial = await importApi.start(selection.id, mode, ack);
      jobRef.current = selection.id;
      finishedRef.current = false;
      setProgress(initial);
      if (mode === 'link') finish(initial);
      else setStep('running');
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const cancel = () => {
    const id = jobRef.current ?? selection?.id;
    if (id) void importApi.cancel(id).catch(() => undefined);
  };

  const close = () => {
    if (step === 'running') return; // must be cancelled first, so nothing is left running unseen
    if (selection && (step === 'review' || step === 'scanning')) {
      void importApi.discard(selection.id).catch(() => undefined);
    }
    onClose();
  };

  const canStart =
    step === 'review' && scan !== null && (mode === 'link' ? ack : scan.enough_space);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Import files"
      className="absolute inset-0 z-10 flex items-center justify-center bg-black/50 p-3"
    >
      <div className="flex max-h-full w-full max-w-lg flex-col rounded-xl border border-white/15 bg-[var(--surface)] shadow-2xl">
        <header className="flex items-center justify-between border-b border-white/10 px-4 py-2.5">
          <h2 className="text-sm font-semibold">Import from this computer</h2>
          <button
            aria-label="Close import"
            disabled={step === 'running'}
            onClick={close}
            className="grid h-6 w-6 place-items-center rounded hover:bg-white/15 disabled:opacity-30"
          >
            <X size={14} />
          </button>
        </header>

        <div className="min-h-0 flex-1 space-y-3 overflow-auto p-4 text-xs">
          {error && (
            <p role="alert" className="rounded-md bg-rose-500/20 px-3 py-2 text-rose-200">
              {error}
            </p>
          )}

          {step === 'choose' && (
            <>
              <p className="text-white/60">
                Bring files or folders from Windows or Linux into NovaOS. Your originals are never
                moved, changed or deleted by a copy.
              </p>
              <div className="flex gap-2">
                <button
                  onClick={() => void pick('files')}
                  className="flex flex-1 items-center justify-center gap-2 rounded-lg bg-[var(--accent)] px-3 py-2.5 text-sm text-white"
                >
                  <FileUp size={16} /> Import files…
                </button>
                <button
                  onClick={() => void pick('folder')}
                  className="flex flex-1 items-center justify-center gap-2 rounded-lg bg-white/10 px-3 py-2.5 text-sm hover:bg-white/20"
                >
                  <FolderUp size={16} /> Import folder…
                </button>
              </div>
              {recent.length > 0 && (
                <details>
                  <summary className="cursor-pointer text-white/60">
                    Recent imports ({recent.length})
                  </summary>
                  <ul className="mt-2 space-y-1">
                    {recent.slice(0, 20).map((r) => (
                      <li
                        key={r.id}
                        className="flex items-center gap-2 rounded-md bg-white/5 px-2 py-1.5"
                      >
                        <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase">
                          {r.mode}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block truncate">{r.name}</span>
                          <span className="block truncate text-white/40" title={r.source_path}>
                            {r.source_path}
                            {r.mode === 'copy' && ` · ${r.files} files, ${formatSize(r.bytes)}`}
                            {r.status === 'partial' && ` · ${r.errors} skipped`}
                            {!r.available && ' · unavailable'}
                          </span>
                        </span>
                        {r.mode === 'link' && (
                          <button
                            aria-label={`Unlink ${r.name}`}
                            onClick={() =>
                              void importApi
                                .unlink(r.id)
                                .then(() => {
                                  onChanged();
                                  return loadRecent();
                                })
                                .catch((e) => setError(errorMessage(e)))
                            }
                            className="rounded px-2 py-1 hover:bg-white/15"
                          >
                            Unlink
                          </button>
                        )}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </>
          )}

          {step === 'scanning' && selection && (
            <div className="flex items-center gap-3 py-6">
              <Loader2 className="animate-spin" size={18} />
              <span className="flex-1">Checking {selection.items.length} item(s)…</span>
              <button
                onClick={() => void importApi.cancel(selection.id)}
                className="rounded-md bg-white/10 px-3 py-1.5 hover:bg-white/20"
              >
                Cancel
              </button>
            </div>
          )}

          {step === 'review' && selection && scan && (
            <>
              <ul className="space-y-0.5 rounded-lg bg-black/20 p-2">
                {selection.items.slice(0, 5).map((i) => (
                  <li key={i.path} className="truncate" title={i.path}>
                    {i.is_dir ? '📁' : '📄'} {i.name}
                  </li>
                ))}
                {selection.items.length > 5 && (
                  <li className="text-white/50">and {selection.items.length - 5} more…</li>
                )}
              </ul>
              <p data-testid="scan-summary">
                {scan.files.toLocaleString()} file{scan.files === 1 ? '' : 's'} in{' '}
                {scan.dirs.toLocaleString()} folder{scan.dirs === 1 ? '' : 's'},{' '}
                {formatSize(scan.bytes)} total.
              </p>

              {scan.symlinks > 0 && (
                <Note>
                  {scan.symlinks} link{scan.symlinks === 1 ? '' : 's'} / shortcut
                  {scan.symlinks === 1 ? '' : 's'} will be skipped (NovaOS never follows them).
                </Note>
              )}
              {scan.unreadable_count > 0 && (
                <Note>
                  {scan.unreadable_count} item{scan.unreadable_count === 1 ? '' : 's'} can’t be read
                  and will be skipped.
                  <details className="mt-1">
                    <summary className="cursor-pointer">Show</summary>
                    <ul className="mt-1 max-h-24 overflow-auto text-white/60">
                      {scan.unreadable.map((u) => (
                        <li key={u} className="truncate" title={u}>
                          {u}
                        </li>
                      ))}
                    </ul>
                  </details>
                </Note>
              )}
              {scan.programs > 0 && (
                <Note>
                  {scan.programs} program file{scan.programs === 1 ? '' : 's'} (.exe, .bat, .sh …)
                  will be imported as ordinary files. NovaOS can’t run Windows or Linux programs.
                </Note>
              )}

              <fieldset className="space-y-2">
                <legend className="mb-1 font-semibold">How should NovaOS bring them in?</legend>
                <label className="flex cursor-pointer gap-2 rounded-lg bg-white/5 p-3 has-[:checked]:ring-1 has-[:checked]:ring-[var(--accent)]">
                  <input
                    type="radio"
                    name="import-mode"
                    checked={mode === 'copy'}
                    onChange={() => setMode('copy')}
                  />
                  <span>
                    <b>Copy</b> (recommended)
                    <span className="block text-white/60">
                      Makes independent copies inside NovaOS. Edit them freely; your originals stay
                      exactly as they are.
                    </span>
                  </span>
                </label>
                <label className="flex cursor-pointer gap-2 rounded-lg bg-white/5 p-3 has-[:checked]:ring-1 has-[:checked]:ring-amber-400">
                  <input
                    type="radio"
                    name="import-mode"
                    checked={mode === 'link'}
                    onChange={() => setMode('link')}
                  />
                  <span>
                    <b className="inline-flex items-center gap-1">
                      <Link2 size={12} /> Link
                    </b>
                    <span className="block text-white/60">
                      Uses the originals in place, nothing is copied and no extra space is used.
                    </span>
                  </span>
                </label>
              </fieldset>

              {mode === 'link' && (
                <div
                  role="alert"
                  className="space-y-2 rounded-lg border border-amber-400/50 bg-amber-400/10 p-3"
                >
                  <p className="flex gap-2 text-amber-100">
                    <AlertTriangle size={14} className="mt-0.5 shrink-0" />
                    <span>
                      Linked items are your <b>original files</b>. Editing, renaming or deleting
                      them in NovaOS changes or removes them on your computer (deletes go to the
                      Recycle Bin). Unlinking removes only the reference.
                    </span>
                  </p>
                  <label className="flex cursor-pointer items-center gap-2">
                    <input
                      type="checkbox"
                      checked={ack}
                      onChange={(e) => setAck(e.target.checked)}
                    />
                    I understand that changes made in NovaOS affect the original files
                  </label>
                </div>
              )}

              {mode === 'copy' && (
                <p className={scan.enough_space ? 'text-white/50' : 'font-semibold text-rose-300'}>
                  {scan.enough_space
                    ? `Needs ${formatSize(scan.bytes)}; ${formatSize(scan.free_bytes)} free.`
                    : `Not enough disk space: needs ${formatSize(scan.required_bytes)} (including a safety margin) but only ${formatSize(scan.free_bytes)} is free.`}
                </p>
              )}
            </>
          )}

          {step === 'running' && progress && <RunningPanel p={progress} onCancel={cancel} />}

          {step === 'finished' && progress && <FinishedPanel p={progress} mode={mode} />}
        </div>

        {(step === 'review' || step === 'finished') && (
          <footer className="flex justify-end gap-2 border-t border-white/10 px-4 py-2.5">
            {step === 'review' && (
              <>
                <button
                  onClick={() => {
                    if (selection) void importApi.discard(selection.id).catch(() => undefined);
                    setSelection(null);
                    setStep('choose');
                  }}
                  className="rounded-md bg-white/10 px-3 py-1.5 text-xs hover:bg-white/20"
                >
                  Back
                </button>
                <button
                  disabled={!canStart}
                  onClick={() => void start()}
                  className="rounded-md bg-[var(--accent)] px-4 py-1.5 text-xs text-white disabled:opacity-40"
                >
                  {mode === 'copy' ? 'Copy into NovaOS' : 'Link originals'}
                </button>
              </>
            )}
            {step === 'finished' && (
              <>
                {mode === 'copy' && progress && progress.items_done > 0 && (
                  <button
                    onClick={() => {
                      onShowImports();
                      onClose();
                    }}
                    className="rounded-md bg-white/10 px-3 py-1.5 text-xs hover:bg-white/20"
                  >
                    Show in Nova Files
                  </button>
                )}
                <button
                  onClick={onClose}
                  className="rounded-md bg-[var(--accent)] px-4 py-1.5 text-xs text-white"
                >
                  Done
                </button>
              </>
            )}
          </footer>
        )}
      </div>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return <div className="rounded-md bg-white/5 px-3 py-2 text-white/70">{children}</div>;
}

function RunningPanel({ p, onCancel }: { p: Progress; onCancel: () => void }) {
  const known =
    p.bytes_total > 0
      ? p.bytes_done / p.bytes_total
      : p.files_total > 0
        ? p.files_done / p.files_total
        : 0;
  const pct = Math.min(100, Math.round(known * 100));
  const remaining = p.bytes_total - p.bytes_done;
  const eta = p.speed_bps > 0 && remaining > 0 ? formatDuration(remaining / p.speed_bps) : null;
  return (
    <div className="space-y-3" data-testid="import-progress">
      <div
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        className="h-2 overflow-hidden rounded-full bg-white/10"
      >
        <div
          className="h-full bg-[var(--accent)] transition-[width]"
          style={{ width: `${pct}%` }}
        />
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-1">
        <span>
          Files: {p.files_done.toLocaleString()} / {p.files_total.toLocaleString()}
        </span>
        <span className="text-right">{pct}%</span>
        <span>
          {formatSize(p.bytes_done)} / {formatSize(p.bytes_total)}
        </span>
        <span className="text-right">
          {formatSpeed(p.speed_bps)}
          {eta && ` · ~${eta} left`}
        </span>
      </div>
      <p className="truncate text-white/50" title={p.current}>
        {p.current || 'Starting…'}
      </p>
      <ErrorList p={p} />
      <button onClick={onCancel} className="rounded-md bg-white/10 px-3 py-1.5 hover:bg-white/20">
        Cancel import
      </button>
    </div>
  );
}

function ErrorList({ p }: { p: Progress }) {
  if (p.error_count === 0) return null;
  return (
    <details className="rounded-md bg-rose-500/10 px-3 py-2 text-rose-200" open>
      <summary className="cursor-pointer">
        {p.error_count} item{p.error_count === 1 ? '' : 's'} skipped
      </summary>
      <ul className="mt-1 max-h-28 overflow-auto">
        {p.errors.map((e, i) => (
          <li key={i} className="truncate" title={e}>
            {e}
          </li>
        ))}
        {p.error_count > p.errors.length && <li>…and {p.error_count - p.errors.length} more</li>}
      </ul>
    </details>
  );
}

function FinishedPanel({ p, mode }: { p: Progress; mode: ImportMode }) {
  let title: string;
  let tone = 'text-emerald-300';
  if (mode === 'link') {
    title =
      p.state === 'failed'
        ? 'Nothing was linked.'
        : `Linked ${p.items_done} item${p.items_done === 1 ? '' : 's'}. They appear under “Linked” in the sidebar. Changes made to them affect the originals.`;
    if (p.state === 'failed') tone = 'text-rose-300';
  } else if (p.state === 'done') {
    title = `Imported ${p.files_done.toLocaleString()} file${p.files_done === 1 ? '' : 's'} (${formatSize(p.bytes_done)}). Your originals were not changed.`;
    if (p.error_count > 0) tone = 'text-amber-300';
  } else if (p.state === 'cancelled') {
    title =
      'Import cancelled. Items that had already finished were kept; the one in progress was removed. Your originals were not changed.';
    tone = 'text-amber-300';
  } else {
    title = `Import stopped: ${p.message ?? 'an error occurred'}. Items that had already finished were kept. Your originals were not changed.`;
    tone = 'text-rose-300';
  }
  return (
    <div className="space-y-3" data-testid="import-result">
      <p className={tone}>{title}</p>
      <ErrorList p={p} />
    </div>
  );
}
