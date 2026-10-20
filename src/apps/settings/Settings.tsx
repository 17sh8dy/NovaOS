import { useMemo, useState, type ComponentType } from 'react';
import {
  Accessibility as AccessibilityIcon,
  Database,
  FolderCog,
  Info,
  Keyboard,
  Monitor,
  Palette,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  type LucideIcon,
} from 'lucide-react';
import { About } from './sections/About';
import { Accessibility } from './sections/Accessibility';
import { Appearance } from './sections/Appearance';
import { DataBackup } from './sections/DataBackup';
import { DesktopTaskbar } from './sections/DesktopTaskbar';
import { FilesStorage } from './sections/FilesStorage';
import { General } from './sections/General';
import { Privacy } from './sections/Privacy';
import { Shortcuts } from './sections/Shortcuts';

export interface SectionDef {
  id: string;
  title: string;
  icon: LucideIcon;
  /** Extra words the search box matches, so "blur" finds Appearance. */
  keywords: string;
  component: ComponentType;
}

/** Adding a settings section is one entry here plus its component. */
export const SECTIONS: SectionDef[] = [
  {
    id: 'general',
    title: 'General',
    icon: SlidersHorizontal,
    keywords: 'start immersive windowed fullscreen display clock time 24 hour date seconds',
    component: General,
  },
  {
    id: 'appearance',
    title: 'Appearance',
    icon: Palette,
    keywords:
      'accent color wallpaper background theme opacity transparency blur glass corners rounded',
    component: Appearance,
  },
  {
    id: 'desktop',
    title: 'Desktop & Taskbar',
    icon: Monitor,
    keywords: 'taskbar position top bottom size icons pinned apps shortcuts',
    component: DesktopTaskbar,
  },
  {
    id: 'files',
    title: 'Files & Storage',
    icon: FolderCog,
    keywords: 'folders access delete confirm recycle sizes data location add folder',
    component: FilesStorage,
  },
  {
    id: 'accessibility',
    title: 'Accessibility',
    icon: AccessibilityIcon,
    keywords: 'text size scale zoom contrast motion animation reduce',
    component: Accessibility,
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
        {visible.length === 0 && (
          <p className="px-2 py-3 text-xs text-white/50">No settings match.</p>
        )}
      </nav>
      <main className="min-w-0 flex-1 overflow-auto p-5">{Body ? <Body /> : null}</main>
    </div>
  );
}
