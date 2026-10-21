import { FxText } from './bits/FxText';
import { useEffect, useState } from 'react';
import { LayoutGrid, Power } from 'lucide-react';
import { getApp } from '../kernel/appRegistry';
import { launchApp } from '../kernel/launcher';
import { useWindows } from '../kernel/windowStore';
import { taskbarHeight, type SettingsData } from '../settings/schema';
import { useSettings } from '../settings/store';
import { useContextMenu } from './ContextMenu';
import { useDisplay } from './displayMode';
import { appMenuItems } from './pinMenu';
import { useShell } from './shellStore';

export function Taskbar({ launch }: { launch: (appId: string) => void }) {
  const windows = useWindows((s) => s.windows);
  const activeId = useWindows((s) => s.activeId);
  const taskbarActivate = useWindows((s) => s.taskbarActivate);
  const { startOpen, toggleStart } = useShell();
  const { mode, exitImmersive } = useDisplay();
  const data = useSettings((s) => s.data);
  const pin = useSettings((s) => s.pin);
  const { desktop, general } = data;
  const { open: openMenu, menu } = useContextMenu();
  const open = Object.values(windows).sort((a, b) =>
    a.id.localeCompare(b.id, undefined, { numeric: true }),
  );
  // Pinned apps appear in the order they were pinned; apps that no longer exist are skipped.
  const pinned = desktop.taskbarPinned.map((id) => getApp(id)).filter((a) => a && !a.hidden);
  const top = desktop.taskbarPosition === 'top';
  const floating = desktop.taskbarStyle === 'floating';
  const btn =
    desktop.taskbarSize === 'large'
      ? 'h-11 w-11'
      : desktop.taskbarSize === 'compact'
        ? 'h-8 w-8'
        : 'h-9 w-9';
  const taskBtn =
    desktop.taskbarSize === 'large' ? 'h-11' : desktop.taskbarSize === 'compact' ? 'h-8' : 'h-9';

  return (
    <>
      <footer
        data-testid="taskbar"
        data-position={desktop.taskbarPosition}
        data-style={desktop.taskbarStyle}
        style={{
          height: taskbarHeight(desktop.taskbarSize),
          background: 'var(--taskbar-bg)',
          ...(floating ? { margin: '8px 8px', borderRadius: 'calc(var(--radius) + 4px)' } : {}),
        }}
        className={`glass absolute z-[100000] flex items-center gap-1 border-white/10 px-2 ${
          floating ? 'inset-x-0 border' : `inset-x-0 ${top ? 'border-b' : 'border-t'}`
        } ${top ? 'top-0' : 'bottom-0'}`}
      >
        <button
          aria-label="Start"
          data-bit="magnet"
          aria-expanded={startOpen}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={toggleStart}
          className={`grid ${btn} place-items-center rounded-lg ${startOpen ? 'bg-white/20' : 'hover:bg-white/10'}`}
        >
          <LayoutGrid size={18} data-bit="dock" className="text-[var(--accent)]" />
        </button>
        <div className="mx-1 h-6 w-px bg-white/10" />

        {pinned.map((a) =>
          a ? (
            <button
              key={a.id}
              aria-label={`Open ${a.title}`}
              data-bit="magnet chroma"
              title={a.title}
              onClick={() => launch(a.id)}
              onContextMenu={(e) =>
                openMenu(e, appMenuItems(a, data, { open: () => launchApp(a.id), pin }))
              }
              className={`grid ${btn} place-items-center rounded-lg hover:bg-white/10`}
            >
              <a.icon size={18} data-bit="dock" />
            </button>
          ) : null,
        )}
        {open.length > 0 && <div className="mx-1 h-6 w-px bg-white/10" />}

        {open.map((w) => {
          const app = getApp(w.appId);
          if (!app) return null;
          const active = activeId === w.id && !w.minimized;
          return (
            <button
              key={w.id}
              data-testid={`task-${w.appId}`}
              aria-pressed={active}
              title={w.title}
              onClick={() => taskbarActivate(w.id)}
              onContextMenu={(e) =>
                openMenu(e, appMenuItems(app, data, { open: () => taskbarActivate(w.id), pin }))
              }
              className={`flex ${taskBtn} max-w-44 items-center gap-2 rounded-lg ${
                desktop.taskbarLabels ? 'px-3' : 'justify-center px-2'
              } text-xs ${
                active
                  ? 'bg-white/20'
                  : w.minimized
                    ? 'opacity-60 hover:bg-white/10'
                    : 'hover:bg-white/10'
              }`}
            >
              <app.icon size={14} />
              {desktop.taskbarLabels && <span className="truncate">{w.title}</span>}
            </button>
          );
        })}

        <div className="flex-1" />
        <Clock general={general} />
        {mode === 'immersive' && (
          <button
            aria-label="Exit Immersive Mode"
            title="Exit Immersive Mode (Ctrl+Alt+Q)"
            onClick={() => void exitImmersive()}
            className={`grid ${btn} place-items-center rounded-lg hover:bg-rose-500/80`}
          >
            <Power size={16} />
          </button>
        )}
      </footer>
      {menu}
    </>
  );
}

export function formatClock(now: Date, g: SettingsData['general']): { time: string; date: string } {
  const time = now.toLocaleTimeString([], {
    hour: g.clockFormat === '24h' ? '2-digit' : 'numeric',
    minute: '2-digit',
    second: g.showSeconds ? '2-digit' : undefined,
    hour12: g.clockFormat === '12h',
  });
  const date = now.toLocaleDateString([], { month: 'short', day: 'numeric' });
  return { time, date };
}

function Clock({ general }: { general: SettingsData['general'] }) {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), general.showSeconds ? 1000 : 15_000);
    return () => window.clearInterval(t);
  }, [general.showSeconds]);
  const { time, date } = formatClock(now, general);
  return (
    <div className="px-2 text-right text-xs leading-tight text-white/80" aria-label="clock">
      <div>
        <FxText where="clock">{time}</FxText>
      </div>
      {general.showDate && <div className="text-white/50">{date}</div>}
    </div>
  );
}
