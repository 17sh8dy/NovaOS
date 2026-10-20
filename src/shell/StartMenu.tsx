import { useState } from 'react';
import { Maximize2, Minimize2, Search } from 'lucide-react';
import { listApps } from '../kernel/appRegistry';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';
import { TASKBAR_HEIGHT } from './Taskbar';

export function StartMenu({ launch }: { launch: (appId: string) => void }) {
  const { startOpen, closeStart } = useShell();
  const { mode, setMode, exitImmersive } = useDisplay();
  const [query, setQuery] = useState('');
  if (!startOpen) return null;

  const apps = listApps().filter((a) => a.title.toLowerCase().includes(query.trim().toLowerCase()));
  const run = (id: string) => {
    launch(id);
    closeStart();
    setQuery('');
  };

  return (
    <div
      role="menu"
      aria-label="Start menu"
      style={{ bottom: TASKBAR_HEIGHT + 8 }}
      onPointerDown={(e) => e.stopPropagation()}
      className="absolute left-2 z-[100001] w-80 rounded-2xl border border-white/10 bg-[var(--surface)] p-3 shadow-2xl backdrop-blur-2xl"
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
        {apps.map((a) => (
          <button
            key={a.id}
            role="menuitem"
            onClick={() => run(a.id)}
            className="flex flex-col items-center gap-1 rounded-lg p-3 text-xs hover:bg-white/10"
          >
            <a.icon size={24} className="text-[var(--accent)]" />
            {a.title}
          </button>
        ))}
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
  );
}
