import { useEffect, useState } from 'react';
import { LayoutGrid, Power } from 'lucide-react';
import { getApp, listApps } from '../kernel/appRegistry';
import { useWindows } from '../kernel/windowStore';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';

export const TASKBAR_HEIGHT = 48;

export function Taskbar({ launch }: { launch: (appId: string) => void }) {
  const windows = useWindows((s) => s.windows);
  const activeId = useWindows((s) => s.activeId);
  const taskbarActivate = useWindows((s) => s.taskbarActivate);
  const { startOpen, toggleStart } = useShell();
  const { mode, exitImmersive } = useDisplay();
  const open = Object.values(windows).sort((a, b) =>
    a.id.localeCompare(b.id, undefined, { numeric: true }),
  );
  const pinned = listApps().filter((a) => a.pinned);

  return (
    <footer
      style={{ height: TASKBAR_HEIGHT }}
      className="absolute inset-x-0 bottom-0 z-[100000] flex items-center gap-1 border-t border-white/10 bg-black/50 px-2 backdrop-blur-2xl"
    >
      <button
        aria-label="Start"
        aria-expanded={startOpen}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={toggleStart}
        className={`grid h-9 w-9 place-items-center rounded-lg ${startOpen ? 'bg-white/20' : 'hover:bg-white/10'}`}
      >
        <LayoutGrid size={18} className="text-[var(--accent)]" />
      </button>
      <div className="mx-1 h-6 w-px bg-white/10" />

      {pinned.map((a) => (
        <button
          key={a.id}
          aria-label={`Open ${a.title}`}
          title={a.title}
          onClick={() => launch(a.id)}
          className="grid h-9 w-9 place-items-center rounded-lg hover:bg-white/10"
        >
          <a.icon size={18} />
        </button>
      ))}
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
            onClick={() => taskbarActivate(w.id)}
            className={`flex h-9 max-w-44 items-center gap-2 rounded-lg px-3 text-xs ${
              active
                ? 'bg-white/20'
                : w.minimized
                  ? 'opacity-60 hover:bg-white/10'
                  : 'hover:bg-white/10'
            }`}
          >
            <app.icon size={14} />
            <span className="truncate">{w.title}</span>
          </button>
        );
      })}

      <div className="flex-1" />
      <Clock />
      {mode === 'immersive' && (
        <button
          aria-label="Exit Immersive Mode"
          title="Exit Immersive Mode (Ctrl+Alt+Q)"
          onClick={() => void exitImmersive()}
          className="grid h-9 w-9 place-items-center rounded-lg hover:bg-rose-500/80"
        >
          <Power size={16} />
        </button>
      )}
    </footer>
  );
}

function Clock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 15_000);
    return () => window.clearInterval(t);
  }, []);
  return (
    <div className="px-2 text-right text-xs leading-tight text-white/80" aria-label="clock">
      <div>{now.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}</div>
      <div className="text-white/50">
        {now.toLocaleDateString([], { month: 'short', day: 'numeric' })}
      </div>
    </div>
  );
}
