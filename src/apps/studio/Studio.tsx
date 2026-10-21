import { useMemo, useState } from 'react';
import {
  Home as HomeIcon,
  Library as LibraryIcon,
  Redo2,
  RotateCcw,
  Search,
  Star,
  Undo2,
} from 'lucide-react';
import { BUILTIN_LOOKS } from '../../settings/builtinPresets';
import { applyLook } from '../../settings/schema';
import { ExternalLink } from '../../components/ExternalLink';
import { Button, Group, Page, Toggle } from '../settings/ui';
import { CONTROLS, controlById, searchControls, type ControlDef } from './controls';
import { Library } from './Library';
import { MiniDesktop } from './MiniDesktop';
import { ControlRow } from './studioUi';
import {
  STUDIOS,
  resetAllCustomization,
  resetStudio,
  studioById,
  type PageId,
  type StudioId,
} from './studios';
import { useStudioSession, type StudioSession } from './useStudioSession';

/** One control, rendered the same way on a Studio page, in search results, or on Home. */
function Control({ c, session }: { c: ControlDef; session: StudioSession }) {
  return (
    <ControlRow
      id={c.id}
      title={c.title}
      hint={c.hint}
      advanced={c.advanced}
      stacked={c.stacked}
      favorite={session.favorites.includes(c.id)}
      onToggleFavorite={session.toggleFavorite}
    >
      {c.render({ data: session.data, set: session.set, advanced: session.advanced })}
    </ControlRow>
  );
}

/** Controls grouped by their `group` title, in order of first appearance. */
function Groups({ controls, session }: { controls: ControlDef[]; session: StudioSession }) {
  const groups = useMemo(() => {
    const map = new Map<string, ControlDef[]>();
    for (const c of controls) map.set(c.group, [...(map.get(c.group) ?? []), c]);
    return [...map.entries()];
  }, [controls]);
  return (
    <>
      {groups.map(([title, list]) => (
        <Group key={title} title={title}>
          {list.map((c) => (
            <Control key={c.id} c={c} session={session} />
          ))}
        </Group>
      ))}
    </>
  );
}

function StudioPage({ id, session }: { id: StudioId; session: StudioSession }) {
  const def = studioById(id);
  const controls = CONTROLS.filter((c) => c.studio === id && (!c.advanced || session.advanced));
  return (
    <Page title={`${def.title} Studio`} description={def.blurb}>
      <Groups controls={controls} session={session} />
      <div>
        <Button
          label={`Reset ${def.title} Studio`}
          onClick={() => session.applyData(resetStudio(session.data, id), `reset.${id}`)}
        >
          <span className="flex items-center gap-1.5">
            <RotateCcw size={12} /> Reset this studio
          </span>
        </Button>
        <span className="ml-3 text-xs text-white/40">You can undo it right after.</span>
      </div>
    </Page>
  );
}

function Home({ session, go }: { session: StudioSession; go: (p: PageId) => void }) {
  const favs = session.favorites.map((id) => controlById(id)).filter((c) => c !== undefined);
  const mine = session.data.studio.presets;
  return (
    <Page
      title="Welcome to Nova Studio"
      description="Customize every part of NovaOS, all in one place."
    >
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {STUDIOS.map((s) => (
          <button
            key={s.id}
            onClick={() => go(s.id)}
            aria-label={`Open ${s.title} Studio`}
            data-bit="bento"
            className="relative flex flex-col items-start gap-1.5 rounded-xl border border-white/10 bg-white/[0.03] p-3 text-left hover:bg-white/10"
          >
            <s.icon size={20} className="text-[var(--accent)]" />
            <span className="text-sm font-medium">{s.title}</span>
            <span className="text-[11px] leading-snug text-white/50">{s.blurb}</span>
          </button>
        ))}
      </div>

      <Group title="Quick looks">
        <div className="flex flex-wrap gap-2 px-4 py-3">
          {[...BUILTIN_LOOKS.map((l) => ({ id: l.id, name: l.name, data: l.data })), ...mine].map(
            (l) => (
              <button
                key={l.id}
                aria-label={`Quick look ${l.name}`}
                onClick={() => session.applyData(applyLook(session.data, l.data), `look.${l.name}`)}
                className="rounded-full bg-white/10 px-3 py-1 text-xs hover:bg-white/20"
              >
                {l.name}
              </button>
            ),
          )}
        </div>
      </Group>

      <div>
        <h3 className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-wider text-white/50 uppercase">
          <Star size={11} /> Favorites
        </h3>
        {favs.length === 0 ? (
          <p className="rounded-xl border border-dashed border-white/15 px-4 py-5 text-xs text-white/50">
            Tap the star next to any setting to keep it here for quick access.
          </p>
        ) : (
          <Group>
            {favs.map((c) => (
              <Control key={c.id} c={c} session={session} />
            ))}
          </Group>
        )}
      </div>

      <div>
        <Button
          label="Reset all customization"
          onClick={() => session.applyData(resetAllCustomization(session.data), 'reset.all')}
        >
          <span className="flex items-center gap-1.5">
            <RotateCcw size={12} /> Reset all customization
          </span>
        </Button>
        <span className="ml-3 text-xs text-white/40">
          Keeps your saved looks and favorites. Undo is one click away.
        </span>
      </div>
      <p className="text-xs text-white/40">
        Want to suggest an idea for Nova Studio?{' '}
        <ExternalLink to="discord">Join the Discord</ExternalLink>
      </p>
    </Page>
  );
}

function SearchResults({
  query,
  session,
  go,
}: {
  query: string;
  session: StudioSession;
  go: (p: PageId) => void;
}) {
  const results = searchControls(query, (id) => studioById(id).title).filter(
    (c) => !c.advanced || session.advanced,
  );
  const byStudio = STUDIOS.map((s) => ({
    s,
    list: results.filter((c) => c.studio === s.id),
  })).filter((x) => x.list.length > 0);
  return (
    <Page
      title={`Results for “${query}”`}
      description={`${results.length} setting${results.length === 1 ? '' : 's'} found`}
    >
      {byStudio.length === 0 && (
        <p className="text-xs text-white/50">
          Nothing matches. Try a word like “cursor”, “blur”, “taskbar” or “font”.
        </p>
      )}
      {byStudio.map(({ s, list }) => (
        <Group key={s.id} title={s.title}>
          {list.map((c) => (
            <Control key={c.id} c={c} session={session} />
          ))}
          <div className="px-4 py-2">
            <button
              onClick={() => go(s.id)}
              className="text-xs text-[var(--accent)] hover:underline"
            >
              Open {s.title} Studio →
            </button>
          </div>
        </Group>
      ))}
    </Page>
  );
}

export function Studio() {
  const session = useStudioSession();
  const [page, setPage] = useState<PageId>('home');
  const [query, setQuery] = useState('');
  const { data } = session;
  const compact = data.studio.sidebar === 'compact';
  const showPreview = data.studio.showPreview;
  const side = data.studio.previewPosition === 'side';
  const searching = query.trim() !== '';

  const go = (p: PageId) => {
    setQuery('');
    setPage(p);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (t.closest('input, textarea')) return; // text fields keep their own undo
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      if (e.shiftKey) session.redo();
      else session.undo();
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y') {
      e.preventDefault();
      session.redo();
    }
  };

  const navItems: {
    id: PageId;
    title: string;
    icon: React.ComponentType<{ size?: number; className?: string }>;
  }[] = [
    { id: 'home', title: 'Home', icon: HomeIcon },
    ...STUDIOS.map((s) => ({ id: s.id as PageId, title: s.title, icon: s.icon })),
    { id: 'library', title: 'Library', icon: LibraryIcon },
  ];

  return (
    <div className="flex h-full text-sm" data-testid="studio" onKeyDown={onKeyDown}>
      <nav
        aria-label="Studios"
        className={`flex shrink-0 flex-col gap-1 overflow-auto border-r border-white/10 p-2 ${compact ? 'w-14' : 'w-48'}`}
      >
        {navItems.map((n) => (
          <button
            key={n.id}
            onClick={() => go(n.id)}
            title={n.title}
            aria-label={n.title}
            aria-current={!searching && page === n.id ? 'page' : undefined}
            className={`flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-xs ${compact ? 'justify-center' : ''} ${
              !searching && page === n.id ? 'bg-white/15' : 'hover:bg-white/10'
            }`}
          >
            <n.icon
              size={15}
              className={!searching && page === n.id ? 'text-[var(--accent)]' : 'text-white/60'}
            />
            {!compact && n.title}
          </button>
        ))}
      </nav>

      <main className="flex min-w-0 flex-1 flex-col">
        <header className="flex flex-wrap items-center gap-2 border-b border-white/10 px-3 py-2">
          <label className="flex min-w-40 flex-1 items-center gap-2 rounded-lg bg-black/30 px-2.5 py-1.5">
            <Search size={13} className="text-white/50" />
            <input
              aria-label="Search all settings"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search every setting: cursor speed, blur, taskbar…"
              className="w-full bg-transparent text-xs outline-none"
            />
          </label>
          <button
            aria-label="Undo"
            title="Undo (Ctrl+Z)"
            disabled={!session.canUndo}
            onClick={session.undo}
            className="grid h-7 w-7 place-items-center rounded-md hover:bg-white/10 disabled:opacity-30"
          >
            <Undo2 size={14} />
          </button>
          <button
            aria-label="Redo"
            title="Redo (Ctrl+Shift+Z)"
            disabled={!session.canRedo}
            onClick={session.redo}
            className="grid h-7 w-7 place-items-center rounded-md hover:bg-white/10 disabled:opacity-30"
          >
            <Redo2 size={14} />
          </button>
          <label className="flex items-center gap-2 text-xs text-white/70">
            Apply instantly
            <Toggle
              label="Apply instantly"
              checked={session.instant}
              onChange={session.setInstant}
            />
          </label>
          <label className="flex items-center gap-2 text-xs text-white/70">
            Advanced
            <Toggle
              label="Advanced mode"
              checked={session.advanced}
              onChange={session.setAdvanced}
            />
          </label>
        </header>

        {session.dirty && (
          <div
            role="status"
            className="flex items-center gap-2 border-b border-amber-400/30 bg-amber-400/10 px-3 py-2 text-xs text-amber-100"
          >
            <span className="flex-1">
              You have changes waiting. The preview shows them; your desktop doesn’t yet.
            </span>
            <Button onClick={session.apply}>Apply</Button>
            <Button onClick={session.discard}>Discard</Button>
          </div>
        )}

        <div className={`flex min-h-0 flex-1 ${side && showPreview ? 'flex-row' : 'flex-col'}`}>
          {showPreview && (
            <div
              className={`shrink-0 ${side ? 'border-l border-white/10 order-2 p-3' : 'border-b border-white/10 p-3'} flex justify-center`}
            >
              <MiniDesktop data={data} width={side ? 300 : 420} />
            </div>
          )}
          <div className="min-h-0 min-w-0 flex-1 overflow-auto p-5">
            {searching ? (
              <SearchResults query={query.trim()} session={session} go={go} />
            ) : page === 'home' ? (
              <Home session={session} go={go} />
            ) : page === 'library' ? (
              <Library session={session} />
            ) : (
              <StudioPage id={page} session={session} />
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
