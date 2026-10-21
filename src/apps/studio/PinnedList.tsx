import { ArrowDown, ArrowUp, Plus, X } from 'lucide-react';
import { listApps } from '../../kernel/appRegistry';
import { pinsFor, type PinPlace } from '../../settings/pins';
import type { SettingsData } from '../../settings/schema';

/**
 * Pick which apps are pinned to the taskbar or the desktop, and in what order. Works on the data
 * it is given (so Studio's draft mode previews it) and reports changes through `onChange`.
 */
export function PinnedList({
  place,
  data,
  onChange,
}: {
  place: PinPlace;
  data: SettingsData;
  onChange: (ids: string[]) => void;
}) {
  const apps = listApps().filter((a) => !a.hidden);
  const pinnedIds = pinsFor(data, place);
  const pinned = pinnedIds
    .map((id) => apps.find((a) => a.id === id))
    .filter((a) => a !== undefined);
  const others = apps.filter((a) => !pinnedIds.includes(a.id));
  const where = place === 'taskbar' ? 'taskbar' : 'desktop';

  const move = (id: string, delta: -1 | 1) => {
    const i = pinnedIds.indexOf(id);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= pinnedIds.length) return;
    const next = [...pinnedIds];
    [next[i], next[j]] = [next[j], next[i]];
    onChange(next);
  };

  return (
    <div className="space-y-1.5">
      {pinned.length === 0 && (
        <p className="text-xs text-white/50">Nothing is pinned to the {where}.</p>
      )}
      {pinned.map((a, i) => (
        <div
          key={a.id}
          className="flex items-center gap-2 rounded-lg bg-black/20 px-2.5 py-1.5 text-xs"
        >
          <a.icon size={14} className="text-[var(--accent)]" />
          <span className="flex-1">{a.title}</span>
          <button
            aria-label={`Move ${a.title} earlier on the ${where}`}
            disabled={i === 0}
            onClick={() => move(a.id, -1)}
            className="grid h-6 w-6 place-items-center rounded hover:bg-white/10 disabled:opacity-25"
          >
            <ArrowUp size={12} />
          </button>
          <button
            aria-label={`Move ${a.title} later on the ${where}`}
            disabled={i === pinned.length - 1}
            onClick={() => move(a.id, 1)}
            className="grid h-6 w-6 place-items-center rounded hover:bg-white/10 disabled:opacity-25"
          >
            <ArrowDown size={12} />
          </button>
          <button
            aria-label={`Unpin ${a.title} from the ${where}`}
            onClick={() => onChange(pinnedIds.filter((x) => x !== a.id))}
            className="grid h-6 w-6 place-items-center rounded hover:bg-rose-500/30"
          >
            <X size={12} />
          </button>
        </div>
      ))}
      {others.length > 0 && (
        <div className="flex flex-wrap gap-1.5 pt-1">
          {others.map((a) => (
            <button
              key={a.id}
              aria-label={`Pin ${a.title} to the ${where}`}
              onClick={() => onChange([...pinnedIds, a.id])}
              className="flex items-center gap-1.5 rounded-full bg-white/10 px-3 py-1 text-xs hover:bg-white/20"
            >
              <Plus size={11} /> {a.title}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
