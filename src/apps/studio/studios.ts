import {
  AppWindow,
  LayoutTemplate,
  MousePointer2,
  Monitor,
  PanelBottom,
  Palette,
  Sparkles,
  Volume2,
  type LucideIcon,
} from 'lucide-react';
import { DEFAULTS, patchSection, type SettingsData } from '../../settings/schema';

export type StudioId =
  'themes' | 'windows' | 'taskbar' | 'desktop' | 'motion' | 'cursor' | 'sounds' | 'interface';

export type PageId = 'home' | StudioId | 'library';

interface Owned {
  section: keyof SettingsData;
  /** Which fields of that section this studio owns; omitted means the whole section. */
  keys?: string[];
}

export interface StudioDef {
  id: StudioId;
  title: string;
  icon: LucideIcon;
  blurb: string;
  keywords: string;
  owned: Owned[];
}

export const STUDIOS: StudioDef[] = [
  {
    id: 'themes',
    title: 'Themes',
    icon: Palette,
    blurb: 'Accent color, wallpaper and the base color of everything.',
    keywords: 'theme color colour accent wallpaper background image surface tint dim fit',
    owned: [
      {
        section: 'appearance',
        keys: ['accent', 'wallpaper', 'wallpaperFit', 'wallpaperDim', 'surfaceColor'],
      },
    ],
  },
  {
    id: 'windows',
    title: 'Windows',
    icon: AppWindow,
    blurb: 'Glass, corners, shadows, title bars and borders.',
    keywords: 'window glass opacity transparency blur corners rounded shadow title bar border',
    owned: [
      { section: 'appearance', keys: ['windowOpacity', 'blur', 'corners'] },
      { section: 'windows' },
    ],
  },
  {
    id: 'taskbar',
    title: 'Taskbar',
    icon: PanelBottom,
    blurb: 'Position, style, size, transparency and pinned apps.',
    keywords: 'taskbar dock position top bottom floating size labels pinned apps pin',
    owned: [
      {
        section: 'desktop',
        keys: [
          'taskbarPosition',
          'taskbarSize',
          'taskbarStyle',
          'taskbarOpacity',
          'taskbarLabels',
          'taskbarPinned',
        ],
      },
    ],
  },
  {
    id: 'desktop',
    title: 'Desktop',
    icon: Monitor,
    blurb: 'Icons, pinned apps and the drag-to-select box.',
    keywords: 'desktop icons labels spacing size pinned apps selection rectangle box drag',
    owned: [
      {
        section: 'desktop',
        keys: ['showDesktopIcons', 'iconSize', 'iconLabels', 'iconSpacing', 'desktopPinned'],
      },
      {
        section: 'appearance',
        keys: ['selectionEnabled', 'selectionColor', 'selectionOpacity'],
      },
    ],
  },
  {
    id: 'motion',
    title: 'Motion',
    icon: Sparkles,
    blurb: 'How windows move, and how fast.',
    keywords: 'motion animation speed fade slide scale reduce motion',
    owned: [{ section: 'motion' }, { section: 'accessibility', keys: ['reduceMotion'] }],
  },
  {
    id: 'cursor',
    title: 'Cursor',
    icon: MousePointer2,
    blurb: 'A custom pointer in any color and size, and click ripples.',
    keywords: 'cursor pointer mouse arrow dot crosshair ripple click color size',
    owned: [{ section: 'cursor' }],
  },
  {
    id: 'sounds',
    title: 'Sounds',
    icon: Volume2,
    blurb: 'Soft sounds for windows and clicks. Off by default.',
    keywords: 'sound audio volume click chime retro soft effects',
    owned: [{ section: 'sounds' }],
  },
  {
    id: 'interface',
    title: 'Interface',
    icon: LayoutTemplate,
    blurb: 'Text size, font, contrast, and how Nova Studio itself looks.',
    keywords:
      'interface font text size scale zoom contrast accessibility studio layout sidebar preview',
    owned: [
      { section: 'accessibility', keys: ['uiScale', 'highContrast'] },
      { section: 'interface' },
      { section: 'studio', keys: ['sidebar', 'showPreview', 'previewPosition'] },
    ],
  },
];

export const studioById = (id: StudioId): StudioDef =>
  STUDIOS.find((s) => s.id === id) as StudioDef;

/** Put every setting a studio owns back to its default. */
export function resetStudio(data: SettingsData, id: StudioId): SettingsData {
  let next = data;
  for (const o of studioById(id).owned) {
    const defaults = DEFAULTS[o.section] as unknown as Record<string, unknown>;
    const keys = o.keys ?? Object.keys(defaults);
    const patch: Record<string, unknown> = {};
    for (const k of keys) patch[k] = defaults[k];
    next = patchSection(next, o.section, patch as never);
  }
  return next;
}

/** Every customization back to the defaults, keeping general, files, favorites and saved looks. */
export function resetAllCustomization(data: SettingsData): SettingsData {
  let next = data;
  for (const s of STUDIOS) next = resetStudio(next, s.id);
  return next;
}
