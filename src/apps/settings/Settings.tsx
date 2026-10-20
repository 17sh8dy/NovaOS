import { useEffect, useState } from 'react';
import { isTauri, native, type PlatformInfo } from '../../native/bridge';
import { errorMessage, filesApi } from '../files/filesApi';
import type { FsRoot } from '../files/filesModel';
import { useDisplay } from '../../shell/displayMode';
import {
  ACCENTS,
  WALLPAPERS,
  useTheme,
  type AccentName,
  type WallpaperName,
} from '../../shell/theme';

export function Settings() {
  const { accent, wallpaper, setAccent, setWallpaper } = useTheme();
  const { mode, setMode, error } = useDisplay();
  const [info, setInfo] = useState<PlatformInfo | null>(null);
  const [custom, setCustom] = useState<FsRoot[]>([]);
  const [fileError, setFileError] = useState<string | null>(null);

  const loadRoots = () =>
    filesApi
      .roots()
      .then((r) => setCustom(r.filter((x) => x.kind === 'custom')))
      .catch(() => undefined);

  useEffect(() => {
    if (isTauri())
      void native<PlatformInfo>('platform_info')
        .then(setInfo)
        .catch(() => undefined);
    if (isTauri()) void loadRoots();
  }, []);

  const addFolder = async () => {
    setFileError(null);
    try {
      await filesApi.addRoot();
      await loadRoots();
    } catch (e) {
      setFileError(errorMessage(e));
    }
  };
  const removeFolder = async (id: string) => {
    try {
      await filesApi.removeRoot(id);
      await loadRoots();
    } catch (e) {
      setFileError(errorMessage(e));
    }
  };

  return (
    <div className="h-full space-y-5 overflow-auto p-4 text-sm" data-testid="settings">
      <section>
        <h2 className="mb-2 text-xs font-semibold tracking-wider text-white/50 uppercase">
          Display mode
        </h2>
        <div className="flex gap-2">
          <ModeButton active={mode === 'windowed'} onClick={() => void setMode('windowed')}>
            Windowed
          </ModeButton>
          <ModeButton active={mode === 'immersive'} onClick={() => void setMode('immersive')}>
            Immersive
          </ModeButton>
          <ModeButton disabled title="Shell Mode is not available in this release">
            Shell (soon)
          </ModeButton>
        </div>
        <p className="mt-2 text-xs text-white/50">
          Leave Immersive at any time with{' '}
          <kbd className="rounded bg-white/10 px-1">Ctrl+Alt+Q</kbd>, the taskbar power button, or
          the Start menu.
        </p>
        {error && <p className="mt-1 text-xs text-rose-400">{error}</p>}
      </section>

      <section>
        <h2 className="mb-2 text-xs font-semibold tracking-wider text-white/50 uppercase">
          Accent
        </h2>
        <div className="flex gap-2">
          {(Object.keys(ACCENTS) as AccentName[]).map((name) => (
            <button
              key={name}
              aria-label={`accent ${name}`}
              aria-pressed={accent === name}
              onClick={() => setAccent(name)}
              style={{ background: ACCENTS[name] }}
              className={`h-7 w-7 rounded-full ring-offset-2 ring-offset-transparent ${
                accent === name ? 'ring-2 ring-white' : ''
              }`}
            />
          ))}
        </div>
      </section>

      <section>
        <h2 className="mb-2 text-xs font-semibold tracking-wider text-white/50 uppercase">
          Wallpaper
        </h2>
        <div className="flex gap-2">
          {(Object.keys(WALLPAPERS) as WallpaperName[]).map((name) => (
            <button
              key={name}
              aria-label={`wallpaper ${name}`}
              aria-pressed={wallpaper === name}
              onClick={() => setWallpaper(name)}
              style={{ background: WALLPAPERS[name] }}
              className={`h-12 w-20 rounded-md ${wallpaper === name ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
            />
          ))}
        </div>
      </section>

      {isTauri() && (
        <section>
          <h2 className="mb-2 text-xs font-semibold tracking-wider text-white/50 uppercase">
            File access
          </h2>
          <p className="mb-2 text-xs text-white/50">
            Nova Files can reach your Nova folder, Documents, Pictures, Downloads and Desktop, plus
            any folder you add here. Drives and system folders are never allowed.
          </p>
          <ul className="mb-2 space-y-1">
            {custom.map((r) => (
              <li
                key={r.id}
                className="flex items-center gap-2 rounded-md bg-white/5 px-2 py-1 text-xs"
              >
                <span className="flex-1 truncate" title={r.path}>
                  {r.label} <span className="text-white/40">{r.path}</span>
                </span>
                <button
                  aria-label={`Remove ${r.label}`}
                  onClick={() => void removeFolder(r.id)}
                  className="rounded px-2 hover:bg-white/15"
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
          <button
            onClick={() => void addFolder()}
            className="rounded-md bg-white/10 px-3 py-1.5 hover:bg-white/20"
          >
            Add folder…
          </button>
          {fileError && <p className="mt-1 text-xs text-rose-400">{fileError}</p>}
        </section>
      )}

      <section>
        <h2 className="mb-2 text-xs font-semibold tracking-wider text-white/50 uppercase">About</h2>
        <p>NovaOS 0.1.0 — a desktop environment running as an application.</p>
        <p className="text-white/50">
          {info ? `${info.os} ${info.arch} · ${info.version}` : 'Running in browser preview'}
        </p>
        <p className="mt-1 text-xs text-white/40">
          NovaOS is not a kernel or a replacement for Windows; it runs on top of it.
        </p>
      </section>
    </div>
  );
}

function ModeButton(props: {
  active?: boolean;
  disabled?: boolean;
  title?: string;
  onClick?: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      disabled={props.disabled}
      title={props.title}
      onClick={props.onClick}
      aria-pressed={props.active}
      className={`rounded-md px-3 py-1.5 ${
        props.active ? 'bg-[var(--accent)] text-white' : 'bg-white/10 hover:bg-white/20'
      } disabled:cursor-not-allowed disabled:opacity-40`}
    >
      {props.children}
    </button>
  );
}
