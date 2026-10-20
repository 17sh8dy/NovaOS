import { useCallback, useEffect, useRef, useState } from 'react';
import { getApp, listApps } from '../kernel/appRegistry';
import { useWindows, type Size } from '../kernel/windowStore';
import { isExitChord, useDisplay } from './displayMode';
import { StartMenu } from './StartMenu';
import { Taskbar, TASKBAR_HEIGHT } from './Taskbar';
import { useShell } from './shellStore';
import { WALLPAPERS, ACCENTS, useTheme } from './theme';
import { Window } from './Window';

export function Desktop() {
  const layer = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<Size>({ width: 1024, height: 640 });
  const windows = useWindows((s) => s.windows);
  const open = useWindows((s) => s.open);
  const { accent, wallpaper, load } = useTheme();
  const closeStart = useShell((s) => s.closeStart);
  const { mode, exitImmersive } = useDisplay();

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const el = layer.current;
    if (!el) return;
    const measure = () => setSize({ width: el.clientWidth, height: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Global escape hatch for Immersive Mode, active no matter which app has focus.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isExitChord(e) && useDisplay.getState().mode === 'immersive') {
        e.preventDefault();
        void exitImmersive();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [exitImmersive]);

  // If the OS leaves fullscreen behind our back (Esc in a browser, Win+Down), resync the UI.
  useEffect(() => {
    const onChange = () => {
      if (!document.fullscreenElement && useDisplay.getState().mode === 'immersive') {
        useDisplay.setState({ mode: 'windowed' });
      }
    };
    document.addEventListener('fullscreenchange', onChange);
    return () => document.removeEventListener('fullscreenchange', onChange);
  }, []);

  const launch = useCallback(
    (appId: string) => {
      const app = getApp(appId);
      if (!app) return;
      open(
        { appId, title: app.title, size: app.defaultSize, singleton: app.singleton },
        {
          width: layer.current?.clientWidth ?? size.width,
          height: layer.current?.clientHeight ?? size.height,
        },
      );
    },
    [open, size],
  );

  return (
    <div
      data-testid="desktop"
      data-mode={mode}
      className="relative h-screen w-screen overflow-hidden text-white select-none"
      style={{ background: WALLPAPERS[wallpaper], ['--accent' as string]: ACCENTS[accent] }}
      onPointerDown={closeStart}
    >
      <div ref={layer} className="absolute inset-x-0 top-0" style={{ bottom: TASKBAR_HEIGHT }}>
        <div className="absolute top-3 left-3 flex flex-col gap-2">
          {listApps()
            .filter((a) => a.pinned)
            .map((a) => (
              <button
                key={a.id}
                onDoubleClick={() => launch(a.id)}
                onKeyDown={(e) => e.key === 'Enter' && launch(a.id)}
                aria-label={`${a.title} shortcut`}
                className="flex w-20 flex-col items-center gap-1 rounded-lg p-2 text-[11px] hover:bg-white/10 focus:bg-white/15 focus:outline-none"
              >
                <a.icon size={28} className="text-[var(--accent)]" />
                <span className="text-center leading-tight">{a.title.replace('Nova ', '')}</span>
              </button>
            ))}
        </div>
        {Object.values(windows).map((w) => (
          <Window key={w.id} win={w} desktop={size} />
        ))}
      </div>
      <StartMenu launch={launch} />
      <Taskbar launch={launch} />
    </div>
  );
}
