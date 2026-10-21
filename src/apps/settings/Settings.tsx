import { useMemo, useState, type ComponentType } from 'react';
import {
  Database,
  FolderCog,
  Info,
  Keyboard,
  Palette,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  type LucideIcon,
} from 'lucide-react';
import { launchApp } from '../../kernel/launcher';
import { About } from './sections/About';
import { DataBackup } from './sections/DataBackup';
import { FilesStorage } from './sections/FilesStorage';
import { General } from './sections/General';
import { Privacy } from './sections/Privacy';
import { Shortcuts } from './sections/Shortcuts';

export interface SectionDef {
  id: string;
  title: string;
  icon: LucideIcon;
  /** Extra words the search box matches, so "clock" finds General. */
  keywords: string;
  component: ComponentType;
}

/**
 * System settings. Everything about how NovaOS looks and feels (themes, wallpaper, windows,
 * taskbar, desktop, cursor, motion, sounds, accessibility) lives in Nova Studio.
 * Adding a section here is one entry plus its component.
 */
export const SECTIONS: SectionDef[] = [
  {
    id: 'general',
    title: 'General',
    icon: SlidersHorizontal,
    keywords:
      'start immersive windowed fullscreen display clock time 24 hour date seconds click open',
    component: General,
  },
  {
    id: 'files',
    title: 'Files & Storage',
    icon: FolderCog,
    keywords: 'folders access delete confirm recycle sizes data location add folder',
    component: FilesStorage,
  },
  {
    id: 'shortcuts',
    title: 'Keyboard shortcuts',
    icon: Keyboard,
    keywords: 'keys hotkeys ctrl alt',
    component: Shortcuts,
  },
  {
    id: 'privacy',
    title: 'Privacy & Security',
    icon: ShieldCheck,
    keywords: 'permissions programs executables network linked unlink protections',
    component: Privacy,
  },
  {
    id: 'data',
    title: 'Data & Backup',
    icon: Database,
    keywords: 'export import backup reset restore json',
    component: DataBackup,
  },
  { id: 'about', title: 'About', icon: Info, keywords: 'version platform build', component: About },
];

/** Words that mean "how NovaOS looks": we point those searches at Nova Studio. */
export const STUDIO_WORDS =
  /theme|wallpaper|background|accent|colou?r|taskbar|desktop|icon|cursor|mouse|motion|animation|sound|audio|font|contrast|scale|zoom|blur|opacity|transparen|corner|window|pin|accessib|selection|ripple|preset|look|style/i;

function StudioCard({ query }: { query?: string }) {
  return (
    <div className="rounded-xl border border-white/10 bg-white/[0.03] p-4">
      <div className="flex items-center gap-3">
        <Palette size={22} className="text-[var(--accent)]" />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold">
            {query ? `“${query}” is in Nova Studio` : 'Looking to customize NovaOS?'}
          </div>
          <div className="text-xs text-white/50">
            Themes, wallpaper, windows, taskbar, desktop, cursor, motion, sounds and accessibility
            all live in Nova Studio.
          </div>
        </div>
        <button
          onClick={() => launchApp('studio')}
          className="rounded-md bg-[var(--accent)] px-3 py-1.5 text-xs text-white"
        >
          Open Nova Studio
        </button>
      </div>
    </div>
  );
}

export function Settings() {
  const [active, setActive] = useState(SECTIONS[0].id);
  const [query, setQuery] = useState('');

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return SECTIONS;
    return SECTIONS.filter((s) => `${s.title} ${s.keywords}`.toLowerCase().includes(q));
  }, [query]);

  // If a search hides the open section, show the first match instead of an empty pane.
  const current = visible.find((s) => s.id === active) ?? visible[0];
  const Body = current?.component;
  const q = query.trim();
  const studioHint = q !== '' && STUDIO_WORDS.test(q);

  return (
    <div className="flex h-full text-sm" data-testid="settings">
      <nav
        aria-label="Settings sections"
        className="flex w-52 shrink-0 flex-col gap-1 overflow-auto border-r border-white/10 p-2"
      >
        <label className="mb-1 flex items-center gap-2 rounded-lg bg-black/30 px-2.5 py-1.5">
          <Search size={13} className="text-white/50" />
          <input
            aria-label="Search settings"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search settings"
            className="w-full bg-transparent text-xs outline-none"
          />
        </label>
        {visible.map((s) => (
          <button
            key={s.id}
            onClick={() => setActive(s.id)}
            aria-current={current?.id === s.id ? 'page' : undefined}
            className={`flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-xs ${
              current?.id === s.id ? 'bg-white/15' : 'hover:bg-white/10'
            }`}
          >
            <s.icon
              size={15}
              className={current?.id === s.id ? 'text-[var(--accent)]' : 'text-white/60'}
            />
            {s.title}
          </button>
        ))}
        {visible.length === 0 && !studioHint && (
          <p className="px-2 py-3 text-xs text-white/50">No settings match.</p>
        )}
        <div className="flex-1" />
        <button
          onClick={() => launchApp('studio')}
          className="flex items-center gap-2.5 rounded-lg border border-white/10 px-2.5 py-2 text-left text-xs hover:bg-white/10"
        >
          <Palette size={15} className="text-[var(--accent)]" />
          Open Nova Studio
        </button>
      </nav>
      <main className="min-w-0 flex-1 space-y-4 overflow-auto p-5">
        {studioHint && <StudioCard query={q} />}
        {Body ? <Body /> : null}
        {!q && current?.id === 'general' && <StudioCard />}
      </main>
    </div>
  );
}
