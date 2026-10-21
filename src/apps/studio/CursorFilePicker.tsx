import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { ExternalLink } from '../../components/ExternalLink';
import { isTauri, native } from '../../native/bridge';
import { useCustomCursor } from '../../settings/customCursor';
import {
  CURSOR_DIR,
  WALLPAPER_ROOT,
  isCursorFileName,
  type SettingsData,
} from '../../settings/schema';
import { errorMessage, filesApi } from '../files/filesApi';
import { Button } from '../settings/ui';

/** One cursor thumbnail: the first frame of the file, drawn from the sandboxed copy. */
function CursorThumb({
  file,
  active,
  onPick,
  onRemove,
}: {
  file: string;
  active: boolean;
  onPick: () => void;
  onRemove: () => void;
}) {
  const { cursor, status } = useCustomCursor(file);
  const frame = cursor?.frames[cursor.sequence[0] ?? 0];
  return (
    <div className="group relative">
      <button
        aria-label={`cursor ${file}`}
        aria-pressed={active}
        title={file}
        onClick={onPick}
        className={`grid h-14 w-full place-items-center rounded-lg bg-black/30 ${active ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
      >
        {frame ? (
          <img
            src={frame.url}
            alt=""
            draggable={false}
            style={{ maxWidth: 40, maxHeight: 40, imageRendering: 'auto' }}
          />
        ) : (
          <span className="text-[10px] text-white/60">
            {status === 'failed' ? 'unavailable' : '…'}
          </span>
        )}
      </button>
      <button
        aria-label={`Remove cursor ${file}`}
        title="Remove (goes to the Recycle Bin)"
        onClick={onRemove}
        className="absolute -top-1.5 -right-1.5 hidden h-5 w-5 place-items-center rounded-full bg-black/80 ring-1 ring-white/30 group-hover:grid focus:grid"
      >
        <X size={11} />
      </button>
    </div>
  );
}

/** Your own cursor files: pick one, add more from your computer, or go back to the built-in styles. */
export function CursorFilePicker({
  cursor,
  onChange,
}: {
  cursor: SettingsData['cursor'];
  onChange: (patch: Partial<SettingsData['cursor']>) => void;
}) {
  const [files, setFiles] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const desktopApp = isTauri();
  const active = cursor.customFile;
  const { status } = useCustomCursor(active || null);

  const load = () =>
    filesApi
      .list(WALLPAPER_ROOT, CURSOR_DIR)
      .then((l) =>
        setFiles(l.entries.filter((e) => !e.is_dir && isCursorFileName(e.name)).map((e) => e.name)),
      )
      .catch(() => setFiles([]));

  useEffect(() => {
    if (desktopApp) void load();
  }, [desktopApp]);

  const add = async () => {
    setError(null);
    try {
      const file = await native<string | null>('cursor_add');
      if (!file) return; // dialog cancelled
      await load();
      onChange({ customFile: file });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const remove = async (file: string) => {
    setError(null);
    try {
      await filesApi.trash(WALLPAPER_ROOT, `${CURSOR_DIR}/${file}`); // Recycle Bin: recoverable
      if (active === file) onChange({ customFile: '' });
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  if (!desktopApp) {
    return (
      <p className="text-xs text-white/50">
        Custom cursor files are available in the NovaOS desktop app.
      </p>
    );
  }

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
        <button
          aria-label="no custom cursor"
          aria-pressed={active === ''}
          onClick={() => onChange({ customFile: '' })}
          className={`grid h-14 place-items-center rounded-lg bg-black/30 px-1 text-center text-[10px] leading-tight text-white/70 ${active === '' ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
        >
          None
          <br />
          (built-in)
        </button>
        {files.map((f) => (
          <CursorThumb
            key={f}
            file={f}
            active={active === f}
            onPick={() => onChange({ customFile: f })}
            onRemove={() => void remove(f)}
          />
        ))}
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <Button onClick={() => void add()}>Add cursor…</Button>
        <span className="text-xs text-white/50">
          .cur, .ani (animated), PNG or GIF up to 2 MB. Copied into NovaOS; the original is never
          changed.
        </span>
      </div>

      <p className="text-xs text-white/50">
        Need some? Browse <ExternalLink to="rw-designer">RW-Designer</ExternalLink>,{' '}
        <ExternalLink to="custom-cursor">Custom Cursor</ExternalLink> or{' '}
        <ExternalLink to="cursors-4u">Cursors-4U</ExternalLink>, download a file (unzip packs
        first), then add it here. Cursors belong to their creators, so check each one’s license.
      </p>

      {active !== '' && status === 'failed' && (
        <p role="alert" className="text-xs text-amber-300">
          NovaOS couldn’t read that cursor, so your normal pointer is showing instead.
        </p>
      )}
      {active !== '' && cursor.engine === 'system' && (
        <p role="status" className="text-xs text-amber-300">
          “Who draws the pointer” is set to Windows, which can’t show custom cursor files.{' '}
          <button
            className="text-[var(--accent)] underline"
            onClick={() => onChange({ engine: 'auto' })}
          >
            Let NovaOS draw it
          </button>
        </p>
      )}
      {error && (
        <p role="alert" className="text-xs text-rose-400">
          {error}
        </p>
      )}
    </div>
  );
}
