import { useCallback, useEffect, useRef, useState } from 'react';
import { MotionConfig } from 'motion/react';
import { listApps } from '../kernel/appRegistry';
import { launchApp, setDesktopSize } from '../kernel/launcher';
import { useWindows, type Size } from '../kernel/windowStore';
import {
  CORNER_PX,
  ICON_PX,
  accentColor,
  selectionColor,
  taskbarHeight,
  wallpaperFile,
} from '../settings/schema';
import { useSettings } from '../settings/store';
import { useWallpaperImage, wallpaperStyle } from '../settings/wallpaper';
import { DesktopIcon } from './DesktopIcon';
import { RecycleBin } from './RecycleBin';
import { isExitChord, useDisplay } from './displayMode';
import {
  boxFrom,
  clampPoint,
  exceedsThreshold,
  hitTest,
  mergeSelection,
  type Box,
  type Point,
} from './selection';
import { StartMenu } from './StartMenu';
import { Taskbar } from './Taskbar';
import { useShell } from './shellStore';
import { Window } from './Window';

interface DragState {
  start: Point;
  base: string[];
  additive: boolean;
  active: boolean;
}

export function Desktop() {
  const layer = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<Size>({ width: 1024, height: 640 });
  const windows = useWindows((s) => s.windows);
  const { data, loaded, load } = useSettings();
  const { appearance, desktop, accessibility, general } = data;
  const closeStart = useShell((s) => s.closeStart);
  const { mode, setMode, exitImmersive } = useDisplay();
  const startedImmersive = useRef(false);

  const image = useWallpaperImage(wallpaperFile(appearance.wallpaper));

  // Desktop icon selection (click, Ctrl+click, or drag a box around them).
  const [selected, setSelected] = useState<string[]>([]);
  const [box, setBox] = useState<Box | null>(null);
  const icons = useRef(new Map<string, HTMLElement>());
  const drag = useRef<DragState | null>(null);

  const register = useCallback((id: string, el: HTMLElement | null) => {
    if (el) icons.current.set(id, el);
    else icons.current.delete(id);
  }, []);

  const select = useCallback((id: string, additive: boolean) => {
    setSelected((cur) =>
      additive ? (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]) : [id],
    );
  }, []);

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

  // ---- drag-to-select on empty desktop
  const onLayerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    // Only the bare desktop starts a selection; icons, windows and the taskbar keep their own clicks.
    if ((e.target as HTMLElement).closest('button, [role="dialog"], [role="status"]')) return;
    const additive = e.ctrlKey || e.metaKey;
    if (!additive) setSelected([]);
    if (!appearance.selectionEnabled) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = {
      start: { x: e.clientX, y: e.clientY },
      base: additive ? selected : [],
      additive,
      active: false,
    };
  };

  const onLayerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    const area = layer.current?.getBoundingClientRect();
    if (!d || !area) return;
    const here: Point = { x: e.clientX, y: e.clientY };
    if (!d.active && !exceedsThreshold(d.start, here)) return;
    d.active = true;
    const bounds: Box = { left: area.left, top: area.top, right: area.right, bottom: area.bottom };
    const sel = boxFrom(clampPoint(d.start, bounds), clampPoint(here, bounds));
    setBox({
      left: sel.left - area.left,
      top: sel.top - area.top,
      right: sel.right - area.left,
      bottom: sel.bottom - area.top,
    });
    const rects = new Map<string, Box>();
    for (const [id, el] of icons.current) {
      const r = el.getBoundingClientRect();
      rects.set(id, { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
    }
    setSelected(mergeSelection(d.base, hitTest(sel, rects), d.additive));
  };

  const endDrag = () => {
    drag.current = null;
    setBox(null);
  };

  const barHeight = taskbarHeight(desktop.taskbarSize);
  const barOnTop = desktop.taskbarPosition === 'top';
  const apps = listApps().filter((a) => a.pinned && !desktop.unpinned.includes(a.id));
  const iconPx = ICON_PX[desktop.iconSize];

  const vars = {
    ...wallpaperStyle(appearance, image),
    '--accent': accentColor(appearance.accent),
    '--selection': selectionColor(appearance),
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
          data-testid="desktop-layer"
          className="absolute inset-x-0"
          style={barOnTop ? { top: barHeight, bottom: 0 } : { top: 0, bottom: barHeight }}
          onPointerDown={onLayerDown}
          onPointerMove={onLayerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <div className="absolute top-3 left-3 flex flex-col gap-2" data-testid="desktop-icons">
            {/* The Recycle Bin is part of the desktop itself: it ignores "Show desktop icons" and pinning. */}
            <RecycleBin
              iconPx={iconPx}
              selected={selected.includes('recycle-bin')}
              onSelect={select}
              register={register}
            />
            {desktop.showDesktopIcons &&
              apps.map((a) => (
                <DesktopIcon
                  key={a.id}
                  id={a.id}
                  label={a.title.replace('Nova ', '')}
                  ariaLabel={`${a.title} shortcut`}
                  icon={a.icon}
                  iconPx={iconPx}
                  selected={selected.includes(a.id)}
                  onSelect={select}
                  open={() => launch(a.id)}
                  register={register}
                />
              ))}
          </div>
          {Object.values(windows).map((w) => (
            <Window key={w.id} win={w} desktop={size} />
          ))}
          {box && (
            <div
              data-testid="selection-box"
              aria-hidden="true"
              className="pointer-events-none absolute"
              style={{
                left: box.left,
                top: box.top,
                width: box.right - box.left,
                height: box.bottom - box.top,
                border: '1px solid var(--selection)',
                background: `color-mix(in srgb, var(--selection) ${Math.round(appearance.selectionOpacity * 100)}%, transparent)`,
                zIndex: 99999,
              }}
            />
          )}
        </div>
        <StartMenu launch={launch} />
        <Taskbar launch={launch} />
      </div>
    </MotionConfig>
  );
}
