import { useState } from 'react';
import { Maximize2, Minimize2, Monitor, PanelBottom, Search } from 'lucide-react';
import { listApps } from '../kernel/appRegistry';
import { isPinned } from '../settings/pins';
import { taskbarReserved } from '../settings/schema';
import { useSettings } from '../settings/store';
import { useContextMenu } from './ContextMenu';
import { useDisplay } from './displayMode';
import { useListFx } from './bits/useBits';
import { appMenuItems } from './pinMenu';
import { useShell } from './shellStore';

export function StartMenu({ launch }: { launch: (appId: string) => void }) {
  const { startOpen, closeStart } = useShell();
  const { mode, setMode, exitImmersive } = useDisplay();
  const [query, setQuery] = useState('');
  const data = useSettings((s) => s.data);
  const pin = useSettings((s) => s.pin);
  const { open: openMenu, menu } = useContextMenu();
  const listFx = useListFx();
  if (!startOpen) return null;

  const { taskbarPosition } = data.desktop;
  const offset = taskbarReserved(data.desktop) + 8;
  const apps = listApps().filter(
    (a) => !a.hidden && a.title.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const run = (id: string) => {
    launch(id);
    closeStart();
    setQuery('');
  };

  return (
    <>
      <div
        role="menu"
        aria-label="Start menu"
        style={taskbarPosition === 'top' ? { top: offset } : { bottom: offset }}
        onPointerDown={(e) => e.stopPropagation()}
        data-bit="glow"
        className="glass absolute left-2 z-[100001] w-80 rounded-[calc(var(--radius)+4px)] border border-white/10 bg-[var(--surface)] p-3 shadow-2xl"
      >
        <label className="mb-3 flex items-center gap-2 rounded-lg bg-black/30 px-3 py-2">
          <Search size={14} className="text-white/50" />
          <input
            autoFocus
            aria-label="Search apps"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && apps[0]) run(apps[0].id);
            }}
            placeholder="Search apps"
            className="w-full bg-transparent text-sm outline-none"
          />
        </label>
        <div className="grid grid-cols-3 gap-1">
          {apps.map((a, index) => {
            const fx = listFx(index);
            const onTaskbar = isPinned(data, 'taskbar', a.id);
            const onDesktop = isPinned(data, 'desktop', a.id);
            return (
              <div
                key={a.id}
                data-bit="bento chroma"
                style={fx.style}
                className={`group relative rounded-lg ${fx.className ?? ''}`}
              >
                <button
                  role="menuitem"
                  onClick={() => run(a.id)}
                  onContextMenu={(e) =>
                    openMenu(e, appMenuItems(a, data, { open: () => run(a.id), pin }))
                  }
                  className="flex w-full flex-col items-center gap-1 rounded-lg p-3 text-xs hover:bg-white/10"
                >
                  <a.icon size={24} data-bit="dock" className="text-[var(--accent)]" />
                  {a.title}
                </button>
                <div className="absolute top-1 right-1 hidden gap-0.5 group-focus-within:flex group-hover:flex">
                  <button
                    aria-label={`${onTaskbar ? 'Unpin' : 'Pin'} ${a.title} ${onTaskbar ? 'from' : 'to'} taskbar`}
                    aria-pressed={onTaskbar}
                    title={onTaskbar ? 'Unpin from taskbar' : 'Pin to taskbar'}
                    onClick={() => pin('taskbar', a.id, !onTaskbar)}
                    className={`grid h-5 w-5 place-items-center rounded ${onTaskbar ? 'bg-[var(--accent)] text-white' : 'bg-black/50 hover:bg-white/20'}`}
                  >
                    <PanelBottom size={11} />
                  </button>
                  <button
                    aria-label={`${onDesktop ? 'Unpin' : 'Pin'} ${a.title} ${onDesktop ? 'from' : 'to'} desktop`}
                    aria-pressed={onDesktop}
                    title={onDesktop ? 'Unpin from desktop' : 'Pin to desktop'}
                    onClick={() => pin('desktop', a.id, !onDesktop)}
                    className={`grid h-5 w-5 place-items-center rounded ${onDesktop ? 'bg-[var(--accent)] text-white' : 'bg-black/50 hover:bg-white/20'}`}
                  >
                    <Monitor size={11} />
                  </button>
                </div>
              </div>
            );
          })}
          {apps.length === 0 && (
            <p className="col-span-3 p-3 text-center text-xs text-white/50">No apps match.</p>
          )}
        </div>
        <div className="mt-3 border-t border-white/10 pt-2">
          {mode === 'immersive' ? (
            <button
              onClick={() => {
                void exitImmersive();
                closeStart();
              }}
              className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm hover:bg-white/10"
            >
              <Minimize2 size={14} /> Exit Immersive Mode
            </button>
          ) : (
            <button
              onClick={() => {
                void setMode('immersive');
                closeStart();
              }}
              className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm hover:bg-white/10"
            >
              <Maximize2 size={14} /> Enter Immersive Mode
            </button>
          )}
        </div>
      </div>
      {menu}
    </>
  );
}
