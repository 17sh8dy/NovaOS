import type { CSSProperties } from 'react';
import { cursorCss } from './cursor';
import {
  CORNER_PX,
  FONTS,
  ICON_GAP_PX,
  SHADOWS,
  TITLEBAR_PX,
  accentColor,
  hexToRgbTriplet,
  rippleColor,
  selectionColor,
  type SettingsData,
} from './schema';
import { wallpaperStyle, type ImageStatus } from './wallpaper';

/**
 * Every customization turned into CSS. The real desktop and the Nova Studio live preview both
 * call this, so what you preview is exactly what you get.
 */
export function themeStyle(
  data: SettingsData,
  image: { url: string | null; status: ImageStatus },
): CSSProperties {
  const { appearance: a, accessibility: x, windows: w, desktop: d, interface: i, cursor: c } = data;
  const activeBorder =
    w.activeBorder === 'accent'
      ? 'color-mix(in srgb, var(--accent) 65%, transparent)'
      : w.activeBorder === 'subtle'
        ? 'rgb(255 255 255 / 0.28)'
        : 'rgb(255 255 255 / 0.1)';
  const cursor = cursorCss(c);
  return {
    ...wallpaperStyle(a, image),
    fontFamily: FONTS[i.font],
    ...(cursor ? { cursor } : {}),
    '--accent': accentColor(a.accent),
    '--selection': selectionColor(a),
    '--ripple': rippleColor(a, c),
    '--surface': `rgb(${hexToRgbTriplet(a.surfaceColor)} / ${x.highContrast ? 1 : a.windowOpacity})`,
    '--blur': `${a.blur}px`,
    '--radius': `${CORNER_PX[a.corners]}px`,
    '--shadow': SHADOWS[w.shadow],
    '--titlebar': `${TITLEBAR_PX[w.titleBar]}px`,
    '--active-border': activeBorder,
    '--border': x.highContrast ? 'rgb(255 255 255 / 0.55)' : 'rgb(255 255 255 / 0.1)',
    '--icon-gap': `${ICON_GAP_PX[d.iconSpacing]}px`,
    '--taskbar-bg': `rgb(0 0 0 / ${x.highContrast ? 1 : d.taskbarOpacity})`,
    '--cursor': cursor ?? 'default',
  } as CSSProperties;
}

/** Attributes the global stylesheet keys off (contrast, motion, custom cursor). */
export function themeAttrs(data: SettingsData): Record<string, string> {
  return {
    'data-contrast': data.accessibility.highContrast ? 'high' : 'normal',
    'data-motion': data.accessibility.reduceMotion ? 'reduced' : 'normal',
    'data-cursor': data.cursor.style === 'system' ? 'system' : 'custom',
  };
}
