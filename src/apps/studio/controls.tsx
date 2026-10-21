import type { ReactNode } from 'react';
import {
  ACCENTS,
  FONTS,
  LIMITS,
  accentColor,
  isHexColor,
  type AccentName,
  type CursorStyle,
  type FontChoice,
  type SettingsData,
  type SoundPack,
} from '../../settings/schema';
import { playSound, type SoundEvent } from '../../settings/sounds';
import { Button, Segmented, Toggle } from '../settings/ui';
import { PinnedList } from './PinnedList';
import { AdvSlider, ColorPick } from './studioUi';
import type { StudioId } from './studios';
import { WallpaperPicker } from './WallpaperPicker';

export type SetFn = <S extends keyof SettingsData>(
  section: S,
  patch: Partial<SettingsData[S]>,
) => void;

export interface Ctx {
  data: SettingsData;
  set: SetFn;
  advanced: boolean;
}

/**
 * One customization control. Studio pages, global search and Home favorites all render from this
 * list, so a control looks and works the same wherever it shows up.
 */
export interface ControlDef {
  /** Stable id (also what Favorites store): lowercase letters, digits, dots and dashes. */
  id: string;
  studio: StudioId;
  group: string;
  title: string;
  hint?: string;
  keywords: string;
  /** Only shown when Advanced mode is on. */
  advanced?: boolean;
  stacked?: boolean;
  render: (c: Ctx) => ReactNode;
}

const pct = (v: number) => `${Math.round(v * 100)}%`;
const SWATCHES = ['#8b5cf6', '#3b82f6', '#14b8a6', '#22c55e', '#f59e0b', '#f43f5e', '#ffffff'];

const SOUND_TESTS: [SoundEvent, string][] = [
  ['open', 'Window opens'],
  ['close', 'Window closes'],
  ['minimize', 'Minimize'],
  ['click', 'Click'],
];

export const CONTROLS: ControlDef[] = [
  // ------------------------------------------------------------------ Themes
  {
    id: 'themes.accent',
    studio: 'themes',
    group: 'Colors',
    title: 'Accent color',
    hint: 'Used for highlights, buttons and the focused window.',
    keywords: 'accent color colour highlight primary',
    stacked: true,
    render: ({ data, set, advanced }) => {
      const a = data.appearance;
      const custom = isHexColor(a.accent);
      return (
        <div className="flex flex-wrap items-center gap-2">
          {(Object.keys(ACCENTS) as AccentName[]).map((name) => (
            <button
              key={name}
              aria-label={`accent ${name}`}
              aria-pressed={a.accent === name}
              onClick={() => set('appearance', { accent: name })}
              style={{ background: ACCENTS[name] }}
              className={`h-7 w-7 rounded-full ${a.accent === name ? 'ring-2 ring-white ring-offset-2 ring-offset-black/60' : ''}`}
            />
          ))}
          <span
            className={`ml-2 flex items-center gap-2 rounded-lg bg-black/30 px-2 py-1 text-xs ${custom ? 'ring-1 ring-white' : ''}`}
          >
            Custom
            <ColorPick
              label="custom accent color"
              value={accentColor(a.accent)}
              onChange={(hex) => set('appearance', { accent: hex })}
              advanced={advanced}
            />
          </span>
        </div>
      );
    },
  },
  {
    id: 'themes.surface',
    studio: 'themes',
    group: 'Colors',
    title: 'Window color',
    hint: 'The base color of windows, menus and panels.',
    keywords: 'surface window panel base tint background color',
    render: ({ data, set, advanced }) => (
      <ColorPick
        label="window color"
        value={data.appearance.surfaceColor}
        onChange={(hex) => set('appearance', { surfaceColor: hex })}
        advanced={advanced}
      />
    ),
  },
  {
    id: 'themes.wallpaper',
    studio: 'themes',
    group: 'Wallpaper',
    title: 'Wallpaper',
    hint: 'Pick a built-in one, or add your own picture.',
    keywords: 'wallpaper background picture image photo custom add',
    stacked: true,
    render: ({ data, set }) => (
      <WallpaperPicker
        wallpaper={data.appearance.wallpaper}
        onPick={(w) => set('appearance', { wallpaper: w })}
      />
    ),
  },
  {
    id: 'themes.fit',
    studio: 'themes',
    group: 'Wallpaper',
    title: 'Fit',
    hint: 'How a picture fills the screen.',
    keywords: 'wallpaper fit fill stretch tile center size',
    render: ({ data, set }) => (
      <Segmented
        label="Wallpaper fit"
        value={data.appearance.wallpaperFit}
        onChange={(v) => set('appearance', { wallpaperFit: v })}
        options={[
          { value: 'fill', label: 'Fill' },
          { value: 'fit', label: 'Fit' },
          { value: 'stretch', label: 'Stretch' },
          { value: 'tile', label: 'Tile' },
          { value: 'center', label: 'Center' },
        ]}
      />
    ),
  },
  {
    id: 'themes.dim',
    studio: 'themes',
    group: 'Wallpaper',
    title: 'Dim wallpaper',
    hint: 'Darkens the picture so icons are easier to read.',
    keywords: 'wallpaper dim dark brightness',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Dim wallpaper"
        {...LIMITS.wallpaperDim}
        value={data.appearance.wallpaperDim}
        onChange={(v) => set('appearance', { wallpaperDim: v })}
        format={(v) => (v === 0 ? 'Off' : pct(v))}
        advanced={advanced}
        toField={(v) => Math.round(v * 100)}
        fromField={(n) => n / 100}
        unit="%"
      />
    ),
  },

  // ------------------------------------------------------------------ Windows
  {
    id: 'windows.opacity',
    studio: 'windows',
    group: 'Glass',
    title: 'Window opacity',
    hint: 'Lower values let the wallpaper show through.',
    keywords: 'window opacity transparency see-through glass',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Window opacity"
        {...LIMITS.windowOpacity}
        value={data.appearance.windowOpacity}
        onChange={(v) => set('appearance', { windowOpacity: v })}
        format={pct}
        advanced={advanced}
        toField={(v) => Math.round(v * 100)}
        fromField={(n) => n / 100}
        unit="%"
      />
    ),
  },
  {
    id: 'windows.blur',
    studio: 'windows',
    group: 'Glass',
    title: 'Background blur',
    hint: 'Frosted-glass strength behind windows and the taskbar.',
    keywords: 'blur frosted glass backdrop',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Background blur"
        {...LIMITS.blur}
        value={data.appearance.blur}
        onChange={(v) => set('appearance', { blur: v })}
        format={(v) => (v === 0 ? 'Off' : `${v}px`)}
        advanced={advanced}
        unit="px"
      />
    ),
  },
  {
    id: 'windows.corners',
    studio: 'windows',
    group: 'Shape',
    title: 'Corner style',
    keywords: 'corners rounded sharp round radius',
    render: ({ data, set }) => (
      <Segmented
        label="Corner style"
        value={data.appearance.corners}
        onChange={(v) => set('appearance', { corners: v })}
        options={[
          { value: 'sharp', label: 'Sharp' },
          { value: 'rounded', label: 'Rounded' },
          { value: 'round', label: 'Round' },
        ]}
      />
    ),
  },
  {
    id: 'windows.shadow',
    studio: 'windows',
    group: 'Shape',
    title: 'Shadow',
    hint: 'How much windows lift off the desktop.',
    keywords: 'shadow depth elevation',
    render: ({ data, set }) => (
      <Segmented
        label="Window shadow"
        value={data.windows.shadow}
        onChange={(v) => set('windows', { shadow: v })}
        options={[
          { value: 'none', label: 'None' },
          { value: 'soft', label: 'Soft' },
          { value: 'strong', label: 'Strong' },
        ]}
      />
    ),
  },
  {
    id: 'windows.titlebar',
    studio: 'windows',
    group: 'Title bar',
    title: 'Title bar size',
    keywords: 'title bar header height size',
    render: ({ data, set }) => (
      <Segmented
        label="Title bar size"
        value={data.windows.titleBar}
        onChange={(v) => set('windows', { titleBar: v })}
        options={[
          { value: 'compact', label: 'Compact' },
          { value: 'default', label: 'Default' },
          { value: 'large', label: 'Large' },
        ]}
      />
    ),
  },
  {
    id: 'windows.border',
    studio: 'windows',
    group: 'Title bar',
    title: 'Active window border',
    hint: 'How the window you are using stands out.',
    keywords: 'border outline focus active highlight',
    render: ({ data, set }) => (
      <Segmented
        label="Active window border"
        value={data.windows.activeBorder}
        onChange={(v) => set('windows', { activeBorder: v })}
        options={[
          { value: 'accent', label: 'Accent' },
          { value: 'subtle', label: 'Subtle' },
          { value: 'off', label: 'Off' },
        ]}
      />
    ),
  },

  // ------------------------------------------------------------------ Taskbar
  {
    id: 'taskbar.position',
    studio: 'taskbar',
    group: 'Layout',
    title: 'Position',
    keywords: 'taskbar position top bottom',
    render: ({ data, set }) => (
      <Segmented
        label="Taskbar position"
        value={data.desktop.taskbarPosition}
        onChange={(v) => set('desktop', { taskbarPosition: v })}
        options={[
          { value: 'bottom', label: 'Bottom' },
          { value: 'top', label: 'Top' },
        ]}
      />
    ),
  },
  {
    id: 'taskbar.style',
    studio: 'taskbar',
    group: 'Layout',
    title: 'Style',
    hint: 'Docked fills the edge; floating hovers with rounded corners.',
    keywords: 'taskbar style dock docked floating',
    render: ({ data, set }) => (
      <Segmented
        label="Taskbar style"
        value={data.desktop.taskbarStyle}
        onChange={(v) => set('desktop', { taskbarStyle: v })}
        options={[
          { value: 'docked', label: 'Docked' },
          { value: 'floating', label: 'Floating' },
        ]}
      />
    ),
  },
  {
    id: 'taskbar.size',
    studio: 'taskbar',
    group: 'Layout',
    title: 'Size',
    keywords: 'taskbar size compact large height',
    render: ({ data, set }) => (
      <Segmented
        label="Taskbar size"
        value={data.desktop.taskbarSize}
        onChange={(v) => set('desktop', { taskbarSize: v })}
        options={[
          { value: 'compact', label: 'Compact' },
          { value: 'default', label: 'Default' },
          { value: 'large', label: 'Large' },
        ]}
      />
    ),
  },
  {
    id: 'taskbar.opacity',
    studio: 'taskbar',
    group: 'Look',
    title: 'Transparency',
    hint: 'Lower values let the wallpaper show through the taskbar.',
    keywords: 'taskbar opacity transparency see-through',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Taskbar opacity"
        {...LIMITS.taskbarOpacity}
        value={data.desktop.taskbarOpacity}
        onChange={(v) => set('desktop', { taskbarOpacity: v })}
        format={pct}
        advanced={advanced}
        toField={(v) => Math.round(v * 100)}
        fromField={(n) => n / 100}
        unit="%"
      />
    ),
  },
  {
    id: 'taskbar.labels',
    studio: 'taskbar',
    group: 'Look',
    title: 'Show window titles',
    hint: 'Off shows only icons for open windows.',
    keywords: 'taskbar labels titles names text icons only',
    render: ({ data, set }) => (
      <Toggle
        label="Show window titles on the taskbar"
        checked={data.desktop.taskbarLabels}
        onChange={(v) => set('desktop', { taskbarLabels: v })}
      />
    ),
  },
  {
    id: 'taskbar.apps',
    studio: 'taskbar',
    group: 'Pinned apps',
    title: 'Apps on the taskbar',
    hint: 'Pin any app and put them in the order you like. You can also right-click an app anywhere.',
    keywords: 'taskbar pinned apps pin unpin order',
    stacked: true,
    render: ({ data, set }) => (
      <PinnedList
        place="taskbar"
        data={data}
        onChange={(ids) => set('desktop', { taskbarPinned: ids })}
      />
    ),
  },

  // ------------------------------------------------------------------ Desktop
  {
    id: 'desktop.icons',
    studio: 'desktop',
    group: 'Icons',
    title: 'Show desktop icons',
    hint: 'The Recycle Bin always stays.',
    keywords: 'desktop icons show hide',
    render: ({ data, set }) => (
      <Toggle
        label="Show desktop icons"
        checked={data.desktop.showDesktopIcons}
        onChange={(v) => set('desktop', { showDesktopIcons: v })}
      />
    ),
  },
  {
    id: 'desktop.iconsize',
    studio: 'desktop',
    group: 'Icons',
    title: 'Icon size',
    keywords: 'desktop icon size small large',
    render: ({ data, set }) => (
      <Segmented
        label="Icon size"
        value={data.desktop.iconSize}
        onChange={(v) => set('desktop', { iconSize: v })}
        options={[
          { value: 'small', label: 'Small' },
          { value: 'medium', label: 'Medium' },
          { value: 'large', label: 'Large' },
        ]}
      />
    ),
  },
  {
    id: 'desktop.labels',
    studio: 'desktop',
    group: 'Icons',
    title: 'Show icon names',
    keywords: 'desktop icon labels names text',
    render: ({ data, set }) => (
      <Toggle
        label="Show icon names"
        checked={data.desktop.iconLabels}
        onChange={(v) => set('desktop', { iconLabels: v })}
      />
    ),
  },
  {
    id: 'desktop.spacing',
    studio: 'desktop',
    group: 'Icons',
    title: 'Icon spacing',
    keywords: 'desktop icon spacing gap tight roomy',
    render: ({ data, set }) => (
      <Segmented
        label="Icon spacing"
        value={data.desktop.iconSpacing}
        onChange={(v) => set('desktop', { iconSpacing: v })}
        options={[
          { value: 'tight', label: 'Tight' },
          { value: 'normal', label: 'Normal' },
          { value: 'roomy', label: 'Roomy' },
        ]}
      />
    ),
  },
  {
    id: 'desktop.apps',
    studio: 'desktop',
    group: 'Pinned apps',
    title: 'Apps on the desktop',
    hint: 'Pin any app and put them in the order you like.',
    keywords: 'desktop pinned apps pin unpin order shortcuts',
    stacked: true,
    render: ({ data, set }) => (
      <PinnedList
        place="desktop"
        data={data}
        onChange={(ids) => set('desktop', { desktopPinned: ids })}
      />
    ),
  },
  {
    id: 'desktop.selection',
    studio: 'desktop',
    group: 'Selection rectangle',
    title: 'Drag to select',
    hint: 'Drag on the empty desktop to draw a box around icons.',
    keywords: 'selection rectangle box drag select marquee',
    render: ({ data, set }) => (
      <Toggle
        label="Selection rectangle"
        checked={data.appearance.selectionEnabled}
        onChange={(v) => set('appearance', { selectionEnabled: v })}
      />
    ),
  },
  {
    id: 'desktop.selectioncolor',
    studio: 'desktop',
    group: 'Selection rectangle',
    title: 'Selection color',
    hint: 'Used for the box and for selected icons.',
    keywords: 'selection rectangle color colour box',
    stacked: true,
    render: ({ data, set, advanced }) => {
      const a = data.appearance;
      const custom = isHexColor(a.selectionColor) && !SWATCHES.includes(a.selectionColor);
      return (
        <div className="flex flex-wrap items-center gap-2">
          <button
            aria-label="selection color match accent"
            aria-pressed={a.selectionColor === 'accent'}
            onClick={() => set('appearance', { selectionColor: 'accent' })}
            className={`rounded-full px-3 py-1 text-xs ${a.selectionColor === 'accent' ? 'bg-[var(--accent)] text-white' : 'bg-white/10 hover:bg-white/20'}`}
          >
            Match accent
          </button>
          {SWATCHES.map((c) => (
            <button
              key={c}
              aria-label={`selection color ${c}`}
              aria-pressed={a.selectionColor === c}
              onClick={() => set('appearance', { selectionColor: c })}
              style={{ background: c }}
              className={`h-6 w-6 rounded-full ${a.selectionColor === c ? 'ring-2 ring-white ring-offset-2 ring-offset-black/60' : ''}`}
            />
          ))}
          <span
            className={`flex items-center gap-2 rounded-lg bg-black/30 px-2 py-1 text-xs ${custom ? 'ring-1 ring-white' : ''}`}
          >
            Custom
            <ColorPick
              label="custom selection color"
              value={isHexColor(a.selectionColor) ? a.selectionColor : accentColor(a.accent)}
              onChange={(hex) => set('appearance', { selectionColor: hex })}
              advanced={advanced}
            />
          </span>
        </div>
      );
    },
  },
  {
    id: 'desktop.selectionfill',
    studio: 'desktop',
    group: 'Selection rectangle',
    title: 'Selection fill strength',
    hint: 'How see-through the box is.',
    keywords: 'selection rectangle fill opacity strength',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Selection fill"
        {...LIMITS.selectionOpacity}
        value={data.appearance.selectionOpacity}
        onChange={(v) => set('appearance', { selectionOpacity: v })}
        format={pct}
        advanced={advanced}
        toField={(v) => Math.round(v * 100)}
        fromField={(n) => n / 100}
        unit="%"
      />
    ),
  },

  // ------------------------------------------------------------------ Motion
  {
    id: 'motion.animation',
    studio: 'motion',
    group: 'Windows',
    title: 'Window animation',
    hint: 'How windows appear and minimize.',
    keywords: 'motion animation window scale fade slide none',
    render: ({ data, set }) => (
      <Segmented
        label="Window animation"
        value={data.motion.windowAnimation}
        onChange={(v) => set('motion', { windowAnimation: v })}
        options={[
          { value: 'scale', label: 'Scale' },
          { value: 'fade', label: 'Fade' },
          { value: 'slide', label: 'Slide' },
          { value: 'none', label: 'None' },
        ]}
      />
    ),
  },
  {
    id: 'motion.speed',
    studio: 'motion',
    group: 'Windows',
    title: 'Animation speed',
    hint: '1× is normal; higher is snappier.',
    keywords: 'motion speed fast slow animation duration',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Animation speed"
        {...LIMITS.motionSpeed}
        value={data.motion.speed}
        onChange={(v) => set('motion', { speed: v })}
        format={(v) => `${v.toFixed(1)}×`}
        advanced={advanced}
        unit="×"
      />
    ),
  },
  {
    id: 'motion.reduce',
    studio: 'motion',
    group: 'Comfort',
    title: 'Reduce motion',
    hint: 'Turns off window animations, transitions and click ripples.',
    keywords: 'reduce motion accessibility animation vestibular',
    render: ({ data, set }) => (
      <Toggle
        label="Reduce motion"
        checked={data.accessibility.reduceMotion}
        onChange={(v) => set('accessibility', { reduceMotion: v })}
      />
    ),
  },

  // ------------------------------------------------------------------ Cursor
  {
    id: 'cursor.style',
    studio: 'cursor',
    group: 'Pointer',
    title: 'Pointer style',
    hint: 'Hover the preview to try it. System uses your normal Windows pointer.',
    keywords: 'cursor pointer mouse arrow classic dot crosshair system',
    render: ({ data, set }) => (
      <Segmented<CursorStyle>
        label="Pointer style"
        value={data.cursor.style}
        onChange={(v) => set('cursor', { style: v })}
        options={[
          { value: 'system', label: 'System' },
          { value: 'classic', label: 'Classic' },
          { value: 'dot', label: 'Dot' },
          { value: 'crosshair', label: 'Crosshair' },
        ]}
      />
    ),
  },
  {
    id: 'cursor.color',
    studio: 'cursor',
    group: 'Pointer',
    title: 'Pointer color',
    keywords: 'cursor pointer color colour fill',
    render: ({ data, set, advanced }) => (
      <ColorPick
        label="pointer color"
        value={data.cursor.color}
        onChange={(hex) => set('cursor', { color: hex })}
        advanced={advanced}
      />
    ),
  },
  {
    id: 'cursor.outline',
    studio: 'cursor',
    group: 'Pointer',
    title: 'Pointer outline',
    keywords: 'cursor pointer outline border color',
    render: ({ data, set, advanced }) => (
      <ColorPick
        label="pointer outline"
        value={data.cursor.outline}
        onChange={(hex) => set('cursor', { outline: hex })}
        advanced={advanced}
      />
    ),
  },
  {
    id: 'cursor.size',
    studio: 'cursor',
    group: 'Pointer',
    title: 'Pointer size',
    keywords: 'cursor pointer size big small',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Pointer size"
        {...LIMITS.cursorSize}
        value={data.cursor.size}
        onChange={(v) => set('cursor', { size: v })}
        format={(v) => `${v}px`}
        advanced={advanced}
        unit="px"
      />
    ),
  },
  {
    id: 'cursor.ripple',
    studio: 'cursor',
    group: 'Click effects',
    title: 'Click ripples',
    hint: 'A soft ring where you click.',
    keywords: 'cursor click ripple effect ring',
    render: ({ data, set }) => (
      <Toggle
        label="Click ripples"
        checked={data.cursor.ripple}
        onChange={(v) => set('cursor', { ripple: v })}
      />
    ),
  },
  {
    id: 'cursor.ripplecolor',
    studio: 'cursor',
    group: 'Click effects',
    title: 'Ripple color',
    keywords: 'cursor click ripple color',
    render: ({ data, set, advanced }) => (
      <span className="flex items-center gap-3">
        <button
          aria-label="ripple color match accent"
          aria-pressed={data.cursor.rippleColor === 'accent'}
          onClick={() => set('cursor', { rippleColor: 'accent' })}
          className={`rounded-full px-3 py-1 text-xs ${data.cursor.rippleColor === 'accent' ? 'bg-[var(--accent)] text-white' : 'bg-white/10 hover:bg-white/20'}`}
        >
          Match accent
        </button>
        <ColorPick
          label="ripple color"
          value={
            isHexColor(data.cursor.rippleColor)
              ? data.cursor.rippleColor
              : accentColor(data.appearance.accent)
          }
          onChange={(hex) => set('cursor', { rippleColor: hex })}
          advanced={advanced}
        />
      </span>
    ),
  },

  // ------------------------------------------------------------------ Sounds
  {
    id: 'sounds.enabled',
    studio: 'sounds',
    group: 'Sounds',
    title: 'Play sounds',
    hint: 'Soft sounds when windows open, close and minimize.',
    keywords: 'sounds audio enable effects windows',
    render: ({ data, set }) => (
      <Toggle
        label="Play sounds"
        checked={data.sounds.enabled}
        onChange={(v) => set('sounds', { enabled: v })}
      />
    ),
  },
  {
    id: 'sounds.pack',
    studio: 'sounds',
    group: 'Sounds',
    title: 'Sound pack',
    keywords: 'sound pack soft click chime retro style',
    render: ({ data, set }) => (
      <Segmented<SoundPack>
        label="Sound pack"
        value={data.sounds.pack}
        onChange={(v) => set('sounds', { pack: v })}
        options={[
          { value: 'soft', label: 'Soft' },
          { value: 'click', label: 'Click' },
          { value: 'chime', label: 'Chime' },
          { value: 'retro', label: 'Retro' },
        ]}
      />
    ),
  },
  {
    id: 'sounds.volume',
    studio: 'sounds',
    group: 'Sounds',
    title: 'Volume',
    keywords: 'sound volume loud quiet',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Volume"
        {...LIMITS.volume}
        value={data.sounds.volume}
        onChange={(v) => set('sounds', { volume: v })}
        format={pct}
        advanced={advanced}
        toField={(v) => Math.round(v * 100)}
        fromField={(n) => n / 100}
        unit="%"
      />
    ),
  },
  {
    id: 'sounds.clicks',
    studio: 'sounds',
    group: 'Sounds',
    title: 'Click sounds',
    hint: 'A tiny tick when you press a button.',
    keywords: 'sound click tick button',
    render: ({ data, set }) => (
      <Toggle
        label="Click sounds"
        checked={data.sounds.clicks}
        onChange={(v) => set('sounds', { clicks: v })}
      />
    ),
  },
  {
    id: 'sounds.test',
    studio: 'sounds',
    group: 'Try them',
    title: 'Preview sounds',
    hint: 'Plays at your chosen pack and volume, even while sounds are off.',
    keywords: 'sound test preview play listen',
    render: ({ data }) => (
      <span className="flex flex-wrap gap-2">
        {SOUND_TESTS.map(([ev, label]) => (
          <Button
            key={ev}
            label={`Play ${label}`}
            onClick={() =>
              playSound(ev, {
                ...data.sounds,
                enabled: true,
                clicks: true,
                volume: data.sounds.volume || 0.5,
              })
            }
          >
            {label}
          </Button>
        ))}
      </span>
    ),
  },

  // ------------------------------------------------------------------ Interface
  {
    id: 'interface.scale',
    studio: 'interface',
    group: 'Text & size',
    title: 'Interface size',
    hint: 'Scales text and spacing across NovaOS.',
    keywords: 'interface size scale zoom text bigger smaller accessibility',
    render: ({ data, set, advanced }) => (
      <AdvSlider
        label="Interface size"
        {...LIMITS.uiScale}
        value={data.accessibility.uiScale}
        onChange={(v) => set('accessibility', { uiScale: v })}
        format={pct}
        advanced={advanced}
        toField={(v) => Math.round(v * 100)}
        fromField={(n) => n / 100}
        unit="%"
      />
    ),
  },
  {
    id: 'interface.font',
    studio: 'interface',
    group: 'Text & size',
    title: 'Font',
    hint: 'Uses fonts already on your computer.',
    keywords: 'font typeface text family serif mono rounded sans',
    render: ({ data, set }) => (
      <Segmented<FontChoice>
        label="Font"
        value={data.interface.font}
        onChange={(v) => set('interface', { font: v })}
        options={(Object.keys(FONTS) as FontChoice[]).map((f) => ({
          value: f,
          label:
            f === 'system'
              ? 'System'
              : f === 'sans'
                ? 'Sans'
                : f === 'serif'
                  ? 'Serif'
                  : f === 'mono'
                    ? 'Mono'
                    : 'Rounded',
        }))}
      />
    ),
  },
  {
    id: 'interface.contrast',
    studio: 'interface',
    group: 'Vision',
    title: 'High contrast',
    hint: 'Solid window backgrounds and brighter borders and text.',
    keywords: 'high contrast accessibility vision borders',
    render: ({ data, set }) => (
      <Toggle
        label="High contrast"
        checked={data.accessibility.highContrast}
        onChange={(v) => set('accessibility', { highContrast: v })}
      />
    ),
  },
  {
    id: 'studio.sidebar',
    studio: 'interface',
    group: 'Nova Studio itself',
    title: 'Sidebar',
    hint: 'Compact shows only icons.',
    keywords: 'studio layout sidebar compact full navigation',
    render: ({ data, set }) => (
      <Segmented
        label="Studio sidebar"
        value={data.studio.sidebar}
        onChange={(v) => set('studio', { sidebar: v })}
        options={[
          { value: 'full', label: 'Full' },
          { value: 'compact', label: 'Compact' },
        ]}
      />
    ),
  },
  {
    id: 'studio.preview',
    studio: 'interface',
    group: 'Nova Studio itself',
    title: 'Show live preview',
    keywords: 'studio layout preview show hide',
    render: ({ data, set }) => (
      <Toggle
        label="Show live preview"
        checked={data.studio.showPreview}
        onChange={(v) => set('studio', { showPreview: v })}
      />
    ),
  },
  {
    id: 'studio.previewpos',
    studio: 'interface',
    group: 'Nova Studio itself',
    title: 'Preview position',
    keywords: 'studio layout preview position top side',
    render: ({ data, set }) => (
      <Segmented
        label="Preview position"
        value={data.studio.previewPosition}
        onChange={(v) => set('studio', { previewPosition: v })}
        options={[
          { value: 'top', label: 'Top' },
          { value: 'side', label: 'Side' },
        ]}
      />
    ),
  },
];

export const controlById = (id: string): ControlDef | undefined =>
  CONTROLS.find((c) => c.id === id);

/** Controls matching a search, across every studio. */
export function searchControls(query: string, studioTitle: (id: StudioId) => string): ControlDef[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const words = q.split(/\s+/);
  return CONTROLS.filter((c) => {
    const hay =
      `${c.title} ${c.hint ?? ''} ${c.keywords} ${c.group} ${studioTitle(c.studio)}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}
