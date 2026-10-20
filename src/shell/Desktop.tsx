import { useCallback, useEffect, useRef, useState } from 'react';
import { MotionConfig } from 'motion/react';
import { listApps } from '../kernel/appRegistry';
import { launchApp, setDesktopSize } from '../kernel/launcher';
import { useWindows, type Size } from '../kernel/windowStore';
import { CORNER_PX, ICON_PX, WALLPAPERS, accentColor, taskbarHeight } from '../settings/schema';
import { useSettings } from '../settings/store';
import { isExitChord, useDisplay } from './displayMode';
import { StartMenu } from './StartMenu';
import { Taskbar } from './Taskbar';
import { useShell } from './shellStore';
import { Window } from './Window';

export function Desktop() {
  const layer = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<Size>({ width: 1024, height: 640 });
  const windows = useWindows((s) => s.windows);
  const { data, loaded, load } = useSettings();
  const { appearance, desktop, accessibility, general } = data;
  const closeStart = useShell((s) => s.closeStart);
  const { mode, setMode, exitImmersive } = useDisplay();
  const startedImmersive = useRef(false);

  useEffect(() => {
    void load();
  }, [load]);

  // "Start in Immersive Mode": applied once, after the saved settings have loaded.
  useEffect(() => {
    if (loaded && general.startInImmersive && !startedImmersive.current) {
      startedImmersive.current = true;
      void setMode('immersive');
    }
  }, [loaded, general.startInImmersive, setMode]);

  // Interface scale: every rem-based size (text, padding) follows it. Window geometry stays in px.
  useEffect(() => {
    document.documentElement.style.fontSize = `${accessibility.uiScale * 100}%`;
    return () => {
      document.documentElement.style.fontSize = '';
    };
  }, [accessibility.uiScale]);

  useEffect(() => {
    const el = layer.current;
    if (!el) return;
    const measure = () => {
      const next = { width: el.clientWidth, height: el.clientHeight };
      setSize(next);
      setDesktopSize(next);
    };
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

  const launch = useCallback((appId: string) => void launchApp(appId), []);

  const barHeight = taskbarHeight(desktop.taskbarSize);
  const barOnTop = desktop.taskbarPosition === 'top';
  const icons = listApps().filter((a) => a.pinned && !desktop.unpinned.includes(a.id));
  const iconPx = ICON_PX[desktop.iconSize];

  const vars = {
    background: WALLPAPERS[appearance.wallpaper],
    '--accent': accentColor(appearance.accent),
    '--surface': `rgb(20 22 34 / ${accessibility.highContrast ? 1 : appearance.windowOpacity})`,
    '--blur': `${appearance.blur}px`,
    '--radius': `${CORNER_PX[appearance.corners]}px`,
  } as React.CSSProperties;

  return (
    <MotionConfig reducedMotion={accessibility.reduceMotion ? 'always' : 'user'}>
      <div
        data-testid="desktop"
        data-mode={mode}
        data-contrast={accessibility.highContrast ? 'high' : 'normal'}
        data-motion={accessibility.reduceMotion ? 'reduced' : 'normal'}
        className="relative h-screen w-screen overflow-hidden text-white select-none"
        style={vars}
        onPointerDown={closeStart}
      >
        <div
          ref={layer}
          className="absolute inset-x-0"
          style={barOnTop ? { top: barHeight, bottom: 0 } : { top: 0, bottom: barHeight }}
        >
          {desktop.showDesktopIcons && (
            <div className="absolute top-3 left-3 flex flex-col gap-2" data-testid="desktop-icons">
              {icons.map((a) => (
                <button
                  key={a.id}
                  onDoubleClick={() => launch(a.id)}
                  onKeyDown={(e) => e.key === 'Enter' && launch(a.id)}
                  aria-label={`${a.title} shortcut`}
                  className="flex w-20 flex-col items-center gap-1 rounded-lg p-2 text-[11px] hover:bg-white/10 focus:bg-white/15 focus:outline-none"
                >
                  <a.icon size={iconPx} className="text-[var(--accent)]" />
                  <span className="text-center leading-tight">{a.title.replace('Nova ', '')}</span>
                </button>
              ))}
            </div>
          )}
          {Object.values(windows).map((w) => (
            <Window key={w.id} win={w} desktop={size} />
          ))}
        </div>
        <StartMenu launch={launch} />
        <Taskbar launch={launch} />
      </div>
    </MotionConfig>
  );
}
