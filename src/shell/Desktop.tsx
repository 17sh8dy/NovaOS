import { useCallback, useEffect, useRef, useState } from 'react';
import { MotionConfig } from 'motion/react';
import { getApp } from '../kernel/appRegistry';
import { launchApp, setDesktopSize } from '../kernel/launcher';
import { useWindows, type Size } from '../kernel/windowStore';
import { ICON_PX, taskbarReserved, wallpaperFile } from '../settings/schema';
import { playSound } from '../settings/sounds';
import { useSettings } from '../settings/store';
import { themeAttrs, themeStyle } from '../settings/theme';
import { useWallpaperImage } from '../settings/wallpaper';
import { ClickRipple } from './ClickRipple';
import { NovaCursor, useNovaPointer } from './NovaCursor';
import { useContextMenu } from './ContextMenu';
import { DesktopIcon } from './DesktopIcon';
import { RecycleBin } from './RecycleBin';
import { isExitChord, useDisplay } from './displayMode';
import { appMenuItems } from './pinMenu';
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
  const pin = useSettings((s) => s.pin);
  const { appearance, desktop, accessibility, general, cursor, motion } = data;
  const closeStart = useShell((s) => s.closeStart);
  const { mode, setMode, exitImmersive } = useDisplay();
  const startedImmersive = useRef(false);
  const novaPointer = useNovaPointer(cursor.engine, mode);
  const [novaReady, setNovaReady] = useState(false);
  const { open: openMenu, menu } = useContextMenu();

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

  // Sounds for windows opening, closing, minimizing and restoring. Silent unless turned on.
  useEffect(() => {
    return useWindows.subscribe((state, prev) => {
      const sounds = useSettings.getState().data.sounds;
      if (!sounds.enabled) return;
      const now = Object.keys(state.windows);
      const before = Object.keys(prev.windows);
      if (now.some((id) => !before.includes(id))) playSound('open', sounds);
      else if (before.some((id) => !now.includes(id))) playSound('close', sounds);
      else {
        for (const id of now) {
          const a = prev.windows[id]?.minimized;
          const b = state.windows[id]?.minimized;
          if (a === false && b === true) playSound('minimize', sounds);
          if (a === true && b === false) playSound('restore', sounds);
        }
      }
    });
  }, []);

  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if ((e.target as HTMLElement | null)?.closest?.('button')) {
        playSound('click', useSettings.getState().data.sounds);
      }
    };
    window.addEventListener('pointerdown', onDown, true);
    return () => window.removeEventListener('pointerdown', onDown, true);
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

  const reserved = taskbarReserved(desktop);
  const barOnTop = desktop.taskbarPosition === 'top';
  const apps = desktop.desktopPinned.map((id) => getApp(id)).filter((a) => a && !a.hidden);
  const iconPx = ICON_PX[desktop.iconSize];

  return (
    <MotionConfig reducedMotion={accessibility.reduceMotion ? 'always' : 'user'}>
      <div
        data-testid="desktop"
        data-mode={mode}
        {...themeAttrs(data)}
        data-cursor-engine={novaPointer && novaReady ? 'nova' : 'system'}
        className="relative h-screen w-screen overflow-hidden text-white select-none"
        style={themeStyle(data, image)}
        onPointerDown={closeStart}
        onContextMenu={(e) => {
          // Our own menus replace the browser's; text fields keep theirs (copy/paste).
          if (!(e.target as HTMLElement).closest('input, textarea')) e.preventDefault();
        }}
      >
        <div
          ref={layer}
          data-testid="desktop-layer"
          className="absolute inset-x-0"
          style={barOnTop ? { top: reserved, bottom: 0 } : { top: 0, bottom: reserved }}
          onPointerDown={onLayerDown}
          onPointerMove={onLayerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <div
            className="absolute top-3 left-3 flex flex-col"
            style={{ gap: 'var(--icon-gap)' }}
            data-testid="desktop-icons"
          >
            {/* The Recycle Bin is part of the desktop itself: it ignores "Show desktop icons" and pinning. */}
            <RecycleBin
              iconPx={iconPx}
              showLabel={desktop.iconLabels}
              selected={selected.includes('recycle-bin')}
              onSelect={select}
              register={register}
            />
            {desktop.showDesktopIcons &&
              apps.map((a) =>
                a ? (
                  <DesktopIcon
                    key={a.id}
                    id={a.id}
                    label={a.title.replace('Nova ', '')}
                    ariaLabel={`${a.title} shortcut`}
                    icon={a.icon}
                    iconPx={iconPx}
                    showLabel={desktop.iconLabels}
                    selected={selected.includes(a.id)}
                    onSelect={select}
                    open={() => launch(a.id)}
                    register={register}
                    onContextMenu={(e) =>
                      openMenu(e, appMenuItems(a, data, { open: () => launch(a.id), pin }))
                    }
                  />
                ) : null,
              )}
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
        <NovaCursor active={novaPointer} cursor={cursor} onReady={setNovaReady} />
        <ClickRipple enabled={cursor.ripple && !accessibility.reduceMotion} speed={motion.speed} />
        {menu}
      </div>
    </MotionConfig>
  );
}
