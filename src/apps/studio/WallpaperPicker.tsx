import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { isTauri, native } from '../../native/bridge';
import {
  WALLPAPERS,
  WALLPAPER_DIR,
  WALLPAPER_ROOT,
  isWallpaperFileName,
  wallpaperFile,
  type SettingsData,
  type WallpaperName,
} from '../../settings/schema';
import { useWallpaperImage } from '../../settings/wallpaper';
import { errorMessage, filesApi } from '../files/filesApi';
import { Button } from '../settings/ui';

/** One custom wallpaper thumbnail. Loads its own image through the sandboxed filesystem. */
function CustomThumb({
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
  const { url, status } = useWallpaperImage(file);
  return (
    <div className="group relative">
      <button
        aria-label={`wallpaper ${file}`}
        aria-pressed={active}
        title={file}
        onClick={onPick}
        style={
          url
            ? {
                backgroundImage: `url("${url}")`,
                backgroundSize: 'cover',
                backgroundPosition: 'center',
              }
            : { background: '#1b2030' }
        }
        className={`h-14 w-full rounded-lg ${active ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
      >
        {status === 'failed' && <span className="text-[10px] text-white/60">unavailable</span>}
      </button>
      <button
        aria-label={`Remove wallpaper ${file}`}
        title="Remove (goes to the Recycle Bin)"
        onClick={onRemove}
        className="absolute -top-1.5 -right-1.5 hidden h-5 w-5 place-items-center rounded-full bg-black/80 ring-1 ring-white/30 group-hover:grid focus:grid"
      >
        <X size={11} />
      </button>
    </div>
  );
}

/** Built-in wallpapers plus the user's own pictures, with add and remove. */
export function WallpaperPicker({
  wallpaper,
  onPick,
}: {
  wallpaper: SettingsData['appearance']['wallpaper'];
  onPick: (wallpaper: string) => void;
}) {
  const [files, setFiles] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const desktopApp = isTauri();
  const activeFile = wallpaperFile(wallpaper);

  const load = () =>
    filesApi
      .list(WALLPAPER_ROOT, WALLPAPER_DIR)
      .then((l) =>
        setFiles(
          l.entries.filter((e) => !e.is_dir && isWallpaperFileName(e.name)).map((e) => e.name),
        ),
      )
      .catch(() => setFiles([]));

  useEffect(() => {
    if (desktopApp) void load();
  }, [desktopApp]);

  const add = async () => {
    setError(null);
    try {
      const file = await native<string | null>('wallpaper_add');
      if (!file) return; // dialog cancelled
      await load();
      onPick(`file:${file}`);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const remove = async (file: string) => {
    setError(null);
    try {
      await filesApi.trash(WALLPAPER_ROOT, `${WALLPAPER_DIR}/${file}`); // Recycle Bin: recoverable
      if (activeFile === file) onPick('aurora');
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
        {(Object.keys(WALLPAPERS) as WallpaperName[]).map((name) => (
          <button
            key={name}
            aria-label={`wallpaper ${name}`}
            aria-pressed={wallpaper === name}
            onClick={() => onPick(name)}
            style={{ background: WALLPAPERS[name] }}
            className={`h-14 rounded-lg ${wallpaper === name ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
            title={name}
          />
        ))}
      </div>
      {desktopApp ? (
        <>
          {files.length > 0 && (
            <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
              {files.map((f) => (
                <CustomThumb
                  key={f}
                  file={f}
                  active={activeFile === f}
                  onPick={() => onPick(`file:${f}`)}
                  onRemove={() => void remove(f)}
                />
              ))}
            </div>
          )}
          <div className="flex items-center gap-3">
            <Button onClick={() => void add()}>Add wallpaper…</Button>
            <span className="text-xs text-white/50">
              PNG, JPG, WebP, GIF or BMP up to 30 MB. Copied into NovaOS; the original is never
              changed.
            </span>
          </div>
          {error && (
            <p role="alert" className="text-xs text-rose-400">
              {error}
            </p>
          )}
        </>
      ) : (
        <p className="text-xs text-white/50">
          Custom wallpapers are available in the NovaOS desktop app.
        </p>
      )}
    </div>
  );
}
