import { useEffect, useState } from 'react';
import { filesApi } from '../apps/files/filesApi';
import { isTauri } from '../native/bridge';
import {
  WALLPAPERS,
  WALLPAPER_DIR,
  WALLPAPER_ROOT,
  wallpaperFile,
  type SettingsData,
  type WallpaperFit,
} from './schema';

const MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
};

export const BASE_COLOR = '#0b0d1a';

const FIT_CSS: Record<WallpaperFit, { size: string; repeat: string; position: string }> = {
  fill: { size: 'cover', repeat: 'no-repeat', position: 'center' },
  fit: { size: 'contain', repeat: 'no-repeat', position: 'center' },
  stretch: { size: '100% 100%', repeat: 'no-repeat', position: 'center' },
  tile: { size: 'auto', repeat: 'repeat', position: 'top left' },
  center: { size: 'auto', repeat: 'no-repeat', position: 'center' },
};

export type ImageStatus = 'idle' | 'loading' | 'ready' | 'failed';

/**
 * Loads a custom wallpaper (a file in the Wallpapers folder) as an object URL. The bytes come
 * through the sandboxed filesystem, never a raw path. Revokes the URL when no longer needed.
 */
export function useWallpaperImage(file: string | null): {
  url: string | null;
  status: ImageStatus;
} {
  const [state, setState] = useState<{
    file: string | null;
    url: string | null;
    status: ImageStatus;
  }>({ file: null, url: null, status: 'idle' });

  useEffect(() => {
    if (!file) return;
    if (!isTauri()) {
      setState({ file, url: null, status: 'failed' });
      return;
    }
    let alive = true;
    let objectUrl: string | null = null;
    setState({ file, url: null, status: 'loading' });
    filesApi
      .readBinary(WALLPAPER_ROOT, `${WALLPAPER_DIR}/${file}`)
      .then((buf) => {
        if (!alive) return;
        const ext = file.split('.').pop()?.toLowerCase() ?? '';
        objectUrl = URL.createObjectURL(
          new Blob([buf], { type: MIME[ext] ?? 'application/octet-stream' }),
        );
        setState({ file, url: objectUrl, status: 'ready' });
        // Make sure it actually decodes; a corrupt image falls back to a preset instead of a blank screen.
        const probe = new Image();
        probe.onerror = () => alive && setState({ file, url: null, status: 'failed' });
        probe.src = objectUrl;
      })
      .catch(() => alive && setState({ file, url: null, status: 'failed' }));
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [file]);

  if (!file) return { url: null, status: 'idle' };
  // Never show a previous file's image for a newly chosen one.
  return state.file === file
    ? { url: state.url, status: state.status }
    : { url: null, status: 'loading' };
}

/** CSS `background` pieces for the desktop wallpaper, including the dimming layer. */
export function wallpaperStyle(
  appearance: SettingsData['appearance'],
  image: { url: string | null; status: ImageStatus },
): React.CSSProperties {
  const file = wallpaperFile(appearance.wallpaper);
  const dim =
    appearance.wallpaperDim > 0
      ? `linear-gradient(rgb(0 0 0 / ${appearance.wallpaperDim}), rgb(0 0 0 / ${appearance.wallpaperDim}))`
      : null;

  if (file && image.status === 'ready' && image.url) {
    const f = FIT_CSS[appearance.wallpaperFit];
    const layers = [dim, `url("${image.url}")`].filter(Boolean).join(', ');
    return {
      backgroundColor: BASE_COLOR,
      backgroundImage: layers,
      backgroundSize: dim ? `auto, ${f.size}` : f.size,
      backgroundRepeat: dim ? `no-repeat, ${f.repeat}` : f.repeat,
      backgroundPosition: dim ? `center, ${f.position}` : f.position,
    };
  }
  // A custom wallpaper that is still loading shows a plain base; a failed one falls back to the default.
  const preset =
    file && image.status === 'failed'
      ? WALLPAPERS.aurora
      : file
        ? BASE_COLOR
        : (WALLPAPERS[appearance.wallpaper as keyof typeof WALLPAPERS] ?? WALLPAPERS.aurora);
  return { background: dim ? `${dim}, ${preset}` : preset };
}
