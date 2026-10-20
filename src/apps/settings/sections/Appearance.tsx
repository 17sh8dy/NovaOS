import { useState } from 'react';
import {
  ACCENTS,
  LIMITS,
  WALLPAPERS,
  accentColor,
  isHexColor,
  type AccentName,
  type Corners,
  type WallpaperName,
} from '../../../settings/schema';
import { useSettings } from '../../../settings/store';
import { Group, Page, Row, Segmented, Slider } from '../ui';

export function Appearance() {
  const { data, set } = useSettings();
  const a = data.appearance;
  const custom = isHexColor(a.accent);
  const [hex, setHex] = useState(custom ? a.accent : '#8b5cf6');

  return (
    <Page title="Appearance" description="Colors, wallpaper and how windows look.">
      <Group title="Colors">
        <Row
          label="Accent color"
          hint="Used for highlights, buttons and the focused window."
          stacked
        >
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
            <label
              className={`ml-2 flex items-center gap-2 rounded-lg bg-black/30 px-2 py-1 text-xs ${custom ? 'ring-1 ring-white' : ''}`}
            >
              Custom
              <input
                type="color"
                aria-label="custom accent color"
                value={custom ? a.accent : hex}
                onChange={(e) => {
                  setHex(e.target.value);
                  set('appearance', { accent: e.target.value });
                }}
                className="h-6 w-8 cursor-pointer rounded border-0 bg-transparent p-0"
              />
            </label>
          </div>
        </Row>
      </Group>

      <Group title="Wallpaper">
        <Row label="Wallpaper" stacked>
          <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
            {(Object.keys(WALLPAPERS) as WallpaperName[]).map((name) => (
              <button
                key={name}
                aria-label={`wallpaper ${name}`}
                aria-pressed={a.wallpaper === name}
                onClick={() => set('appearance', { wallpaper: name })}
                style={{ background: WALLPAPERS[name] }}
                className={`h-14 rounded-lg ${a.wallpaper === name ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
                title={name}
              />
            ))}
          </div>
        </Row>
      </Group>

      <Group title="Windows">
        <Row label="Window opacity" hint="Lower values let the wallpaper show through.">
          <Slider
            label="Window opacity"
            {...LIMITS.windowOpacity}
            value={a.windowOpacity}
            onChange={(v) => set('appearance', { windowOpacity: v })}
            format={(v) => `${Math.round(v * 100)}%`}
          />
        </Row>
        <Row label="Background blur" hint="Frosted-glass strength behind windows and the taskbar.">
          <Slider
            label="Background blur"
            {...LIMITS.blur}
            value={a.blur}
            onChange={(v) => set('appearance', { blur: v })}
            format={(v) => (v === 0 ? 'Off' : `${v}px`)}
          />
        </Row>
        <Row label="Corner style">
          <Segmented<Corners>
            label="Corner style"
            value={a.corners}
            onChange={(v) => set('appearance', { corners: v })}
            options={[
              { value: 'sharp', label: 'Sharp' },
              { value: 'rounded', label: 'Rounded' },
              { value: 'round', label: 'Round' },
            ]}
          />
        </Row>
      </Group>

      <div
        aria-label="Preview"
        className="overflow-hidden border border-white/10"
        style={{ background: WALLPAPERS[a.wallpaper], borderRadius: 'var(--radius)' }}
      >
        <div className="p-4">
          <div
            className="glass max-w-xs border border-white/15 p-3 text-xs"
            style={{ background: 'var(--surface)', borderRadius: 'var(--radius)' }}
          >
            <div className="mb-2 font-semibold">Preview window</div>
            <span
              className="inline-block rounded-md px-3 py-1 text-white"
              style={{ background: accentColor(a.accent) }}
            >
              Accent button
            </span>
          </div>
        </div>
      </div>
    </Page>
  );
}
