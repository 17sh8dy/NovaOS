import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { isTauri, native } from '../../../native/bridge';
import {
  ACCENTS,
  LIMITS,
  WALLPAPERS,
  WALLPAPER_DIR,
  WALLPAPER_ROOT,
  accentColor,
  isHexColor,
  isWallpaperFileName,
  selectionColor,
  wallpaperFile,
  type AccentName,
  type Corners,
  type WallpaperFit,
  type WallpaperName,
} from '../../../settings/schema';
import { useSettings } from '../../../settings/store';
import { useWallpaperImage, wallpaperStyle } from '../../../settings/wallpaper';
import { errorMessage, filesApi } from '../../files/filesApi';
import { Button, Group, Page, Row, Segmented, Slider, Toggle } from '../ui';

const SELECTION_SWATCHES = [
  '#8b5cf6',
  '#3b82f6',
  '#14b8a6',
  '#22c55e',
  '#f59e0b',
  '#f43f5e',
  '#ffffff',
];

/** One custom wallpaper thumbnail. Loads its own image through the sandboxed filesystem. */
function CustomThumb({
  file,
  active,
  onPick,
  onRemove,
}: {
  file: string;
  active: boolean;
  onPick: () => void;
  onRemove: () => void;
}) {
  const { url, status } = useWallpaperImage(file);
  return (
    <div className="group relative">
      <button
        aria-label={`wallpaper ${file}`}
        aria-pressed={active}
        title={file}
        onClick={onPick}
        style={
          url
            ? {
                backgroundImage: `url("${url}")`,
                backgroundSize: 'cover',
                backgroundPosition: 'center',
              }
            : { background: '#1b2030' }
        }
        className={`h-14 w-full rounded-lg ${active ? 'ring-2 ring-white' : 'ring-1 ring-white/20'}`}
      >
        {status === 'failed' && <span className="text-[10px] text-white/60">unavailable</span>}
      </button>
      <button
        aria-label={`Remove wallpaper ${file}`}
        title="Remove (goes to the Recycle Bin)"
        onClick={onRemove}
        className="absolute -top-1.5 -right-1.5 hidden h-5 w-5 place-items-center rounded-full bg-black/80 ring-1 ring-white/30 group-hover:grid focus:grid"
      >
        <X size={11} />
      </button>
    </div>
  );
}

export function Appearance() {
  const { data, set } = useSettings();
  const a = data.appearance;
  const customAccent = isHexColor(a.accent);
  const [hex, setHex] = useState(customAccent ? a.accent : '#8b5cf6');
  const [selHex, setSelHex] = useState(isHexColor(a.selectionColor) ? a.selectionColor : '#8b5cf6');
  const [customFiles, setCustomFiles] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const desktopApp = isTauri();
  const activeFile = wallpaperFile(a.wallpaper);
  const previewImage = useWallpaperImage(activeFile);

  const loadCustom = () =>
    filesApi
      .list(WALLPAPER_ROOT, WALLPAPER_DIR)
      .then((l) =>
        setCustomFiles(
          l.entries.filter((e) => !e.is_dir && isWallpaperFileName(e.name)).map((e) => e.name),
        ),
      )
      .catch(() => setCustomFiles([]));

  useEffect(() => {
    if (desktopApp) void loadCustom();
  }, [desktopApp]);

  const addWallpaper = async () => {
    setError(null);
    try {
      const file = await native<string | null>('wallpaper_add');
      if (!file) return; // dialog cancelled
      await loadCustom();
      set('appearance', { wallpaper: `file:${file}` });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const removeWallpaper = async (file: string) => {
    setError(null);
    try {
      await filesApi.trash(WALLPAPER_ROOT, `${WALLPAPER_DIR}/${file}`); // Recycle Bin, so it is recoverable
      if (activeFile === file) set('appearance', { wallpaper: 'aurora' });
      await loadCustom();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const preview = wallpaperStyle(a, previewImage);

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
              className={`ml-2 flex items-center gap-2 rounded-lg bg-black/30 px-2 py-1 text-xs ${customAccent ? 'ring-1 ring-white' : ''}`}
            >
              Custom
              <input
                type="color"
                aria-label="custom accent color"
                value={customAccent ? a.accent : hex}
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
        <Row label="Built-in" stacked>
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
        <Row
          label="Your wallpapers"
          hint="Pick a PNG, JPG, WebP, GIF or BMP (up to 30 MB). It is copied into NovaOS; the original is never changed."
          stacked
        >
          {desktopApp ? (
            <div className="space-y-3">
              {customFiles.length > 0 && (
                <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
                  {customFiles.map((f) => (
                    <CustomThumb
                      key={f}
                      file={f}
                      active={activeFile === f}
                      onPick={() => set('appearance', { wallpaper: `file:${f}` })}
                      onRemove={() => void removeWallpaper(f)}
                    />
                  ))}
                </div>
              )}
              <Button onClick={() => void addWallpaper()}>Add wallpaper…</Button>
              {error && (
                <p role="alert" className="text-xs text-rose-400">
                  {error}
                </p>
              )}
            </div>
          ) : (
            <p className="text-xs text-white/50">
              Custom wallpapers are available in the NovaOS desktop app.
            </p>
          )}
        </Row>
        <Row label="Fit" hint="How a picture fills the screen.">
          <Segmented<WallpaperFit>
            label="Wallpaper fit"
            value={a.wallpaperFit}
            onChange={(v) => set('appearance', { wallpaperFit: v })}
            options={[
              { value: 'fill', label: 'Fill' },
              { value: 'fit', label: 'Fit' },
              { value: 'stretch', label: 'Stretch' },
              { value: 'tile', label: 'Tile' },
              { value: 'center', label: 'Center' },
            ]}
          />
        </Row>
        <Row label="Dim wallpaper" hint="Darkens the picture so icons are easier to read.">
          <Slider
            label="Dim wallpaper"
            {...LIMITS.wallpaperDim}
            value={a.wallpaperDim}
            onChange={(v) => set('appearance', { wallpaperDim: v })}
            format={(v) => (v === 0 ? 'Off' : `${Math.round(v * 100)}%`)}
          />
        </Row>
      </Group>

      <Group title="Selection rectangle">
        <Row label="Drag to select" hint="Drag on the empty desktop to draw a box around icons.">
          <Toggle
            label="Selection rectangle"
            checked={a.selectionEnabled}
            onChange={(v) => set('appearance', { selectionEnabled: v })}
          />
        </Row>
        <Row label="Color" hint="Used for the box and for selected icons." stacked>
          <div className="flex flex-wrap items-center gap-2">
            <button
              aria-label="selection color match accent"
              aria-pressed={a.selectionColor === 'accent'}
              onClick={() => set('appearance', { selectionColor: 'accent' })}
              className={`rounded-full px-3 py-1 text-xs ${a.selectionColor === 'accent' ? 'bg-[var(--accent)] text-white' : 'bg-white/10 hover:bg-white/20'}`}
            >
              Match accent
            </button>
            {SELECTION_SWATCHES.map((c) => (
              <button
                key={c}
                aria-label={`selection color ${c}`}
                aria-pressed={a.selectionColor === c}
                onClick={() => set('appearance', { selectionColor: c })}
                style={{ background: c }}
                className={`h-6 w-6 rounded-full ${a.selectionColor === c ? 'ring-2 ring-white ring-offset-2 ring-offset-black/60' : ''}`}
              />
            ))}
            <label
              className={`flex items-center gap-2 rounded-lg bg-black/30 px-2 py-1 text-xs ${isHexColor(a.selectionColor) && !SELECTION_SWATCHES.includes(a.selectionColor) ? 'ring-1 ring-white' : ''}`}
            >
              Custom
              <input
                type="color"
                aria-label="custom selection color"
                value={isHexColor(a.selectionColor) ? a.selectionColor : selHex}
                onChange={(e) => {
                  setSelHex(e.target.value);
                  set('appearance', { selectionColor: e.target.value });
                }}
                className="h-6 w-8 cursor-pointer rounded border-0 bg-transparent p-0"
              />
            </label>
          </div>
        </Row>
        <Row label="Fill strength" hint="How see-through the box is.">
          <Slider
            label="Selection fill"
            {...LIMITS.selectionOpacity}
            value={a.selectionOpacity}
            onChange={(v) => set('appearance', { selectionOpacity: v })}
            format={(v) => `${Math.round(v * 100)}%`}
          />
        </Row>
        <div className="px-4 pb-3">
          <div
            aria-label="Selection preview"
            className="relative h-14 overflow-hidden rounded-lg border border-white/10"
            style={{ background: '#0b0d1a' }}
          >
            <div
              className="absolute top-2 left-4 h-10 w-40 rounded-sm"
              style={{
                border: `1px solid ${selectionColor(a)}`,
                background: `color-mix(in srgb, ${selectionColor(a)} ${Math.round(a.selectionOpacity * 100)}%, transparent)`,
              }}
            />
          </div>
        </div>
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
        style={{ ...preview, borderRadius: 'var(--radius)' }}
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
