import { useState } from 'react';
import { LayoutGrid, Trash2 } from 'lucide-react';
import { getApp, type AppManifest } from '../../kernel/appRegistry';
import { ICON_PX, taskbarHeight, wallpaperFile, type SettingsData } from '../../settings/schema';
import { themeAttrs, themeStyle } from '../../settings/theme';
import { useWallpaperImage } from '../../settings/wallpaper';

interface Ripple {
  id: number;
  x: number;
  y: number;
}

/**
 * A miniature, interactive desktop that renders the given settings with the very same theme
 * function the real desktop uses, so what you see here is what you get. Hover it to try your
 * pointer; click it to try ripples.
 */
export function MiniDesktop({ data, width }: { data: SettingsData; width: number }) {
  const { desktop: d, appearance: a, cursor: c, accessibility: x, general } = data;
  const image = useWallpaperImage(wallpaperFile(a.wallpaper));
  const [ripples, setRipples] = useState<Ripple[]>([]);

  const s = width / 1000; // design pixels -> preview pixels
  const height = Math.round((width * 9) / 16);
  const bar = taskbarHeight(d.taskbarSize) * s;
  const floating = d.taskbarStyle === 'floating';
  const margin = floating ? 8 * s : 0;
  const top = d.taskbarPosition === 'top';
  const reserved = bar + margin * 2;
  const iconPx = ICON_PX[d.iconSize] * s * 1.1;
  const font = Math.max(6, 11 * s);
  const taskApps = d.taskbarPinned
    .map((id) => getApp(id))
    .filter((app): app is AppManifest => app !== undefined && !app.hidden)
    .slice(0, 6);
  const deskApps = d.desktopPinned
    .map((id) => getApp(id))
    .filter((app): app is AppManifest => app !== undefined && !app.hidden)
    .slice(0, 4);

  const onDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!c.ripple || x.reduceMotion) return;
    const r = e.currentTarget.getBoundingClientRect();
    const id = Date.now() + Math.random();
    setRipples((cur) => [...cur.slice(-3), { id, x: e.clientX - r.left, y: e.clientY - r.top }]);
    window.setTimeout(() => setRipples((cur) => cur.filter((q) => q.id !== id)), 900);
  };

  return (
    <div
      data-testid="mini-desktop"
      aria-label="Live preview"
      {...themeAttrs(data)}
      onPointerDown={onDown}
      className="relative shrink-0 overflow-hidden border border-white/10 text-white select-none"
      style={{ ...themeStyle(data, image), width, height, borderRadius: 12 }}
    >
      {/* desktop icons */}
      <div
        className="absolute flex flex-col"
        style={{
          left: 14 * s,
          top: (top ? reserved : 0) + 14 * s,
          gap: `calc(var(--icon-gap) * ${s})`,
        }}
      >
        {[
          {
            key: 'bin',
            label: 'Recycle Bin',
            node: <Trash2 size={iconPx} className="text-white/80" />,
          },
        ]
          .concat(
            d.showDesktopIcons
              ? deskApps.map((app) => ({
                  key: app.id,
                  label: app.title.replace('Nova ', ''),
                  node: (
                    <span style={{ color: 'var(--accent)' }}>
                      <app.icon size={iconPx} />
                    </span>
                  ),
                }))
              : [],
          )
          .map((i) => (
            <div
              key={i.key}
              className="flex flex-col items-center"
              style={{ width: 64 * s, gap: 2 * s }}
            >
              {i.node}
              {d.iconLabels && <span style={{ fontSize: font, lineHeight: 1.1 }}>{i.label}</span>}
            </div>
          ))}
      </div>

      {/* a background window and a focused window */}
      <div
        className="absolute border"
        style={{
          left: '30%',
          top: `calc(${top ? reserved : 0}px + 11%)`,
          width: '52%',
          height: '50%',
          background: 'var(--surface)',
          borderColor: 'var(--border)',
          borderRadius: `calc(var(--radius) * ${s})`,
          backdropFilter: `blur(calc(var(--blur) * ${s}))`,
          boxShadow: 'var(--shadow)',
          opacity: 0.85,
        }}
      />
      <div
        data-testid="mini-window"
        className="absolute flex flex-col overflow-hidden border"
        style={{
          left: '24%',
          top: `calc(${top ? reserved : 0}px + 20%)`,
          width: '52%',
          height: '50%',
          background: 'var(--surface)',
          borderColor: 'var(--active-border)',
          borderRadius: `calc(var(--radius) * ${s})`,
          backdropFilter: `blur(calc(var(--blur) * ${s}))`,
          boxShadow: 'var(--shadow)',
        }}
      >
        <div
          className="flex items-center gap-1 bg-white/5 px-2"
          style={{ height: `calc(var(--titlebar) * ${s})`, fontSize: font }}
        >
          <span style={{ color: 'var(--accent)' }}>●</span>
          <span className="flex-1 truncate">Nova Notes</span>
          <span className="text-white/60">— □ ✕</span>
        </div>
        <div className="flex-1 space-y-1 p-2" style={{ fontSize: font }}>
          <div>Hello from NovaOS</div>
          <div className="h-1 w-3/4 rounded bg-white/15" />
          <div className="h-1 w-1/2 rounded bg-white/15" />
          <span
            className="mt-1 inline-block rounded px-2 py-0.5 text-white"
            style={{ background: 'var(--accent)', fontSize: font }}
          >
            Button
          </span>
        </div>
      </div>

      {/* selection rectangle sample */}
      {a.selectionEnabled && (
        <div
          className="absolute"
          style={{
            left: '3%',
            top: `calc(${top ? reserved : 0}px + 56%)`,
            width: '16%',
            height: '18%',
            border: '1px solid var(--selection)',
            background: `color-mix(in srgb, var(--selection) ${Math.round(a.selectionOpacity * 100)}%, transparent)`,
          }}
        />
      )}

      {/* taskbar */}
      <div
        data-testid="mini-taskbar"
        className="absolute flex items-center gap-1 px-2"
        style={{
          left: margin,
          right: margin,
          [top ? 'top' : 'bottom']: margin,
          height: bar,
          background: 'var(--taskbar-bg)',
          backdropFilter: `blur(calc(var(--blur) * ${s}))`,
          borderRadius: floating ? `calc((var(--radius) + 4px) * ${s})` : 0,
          border: floating ? '1px solid rgb(255 255 255 / 0.1)' : undefined,
          [top ? 'borderBottom' : 'borderTop']: floating
            ? undefined
            : '1px solid rgb(255 255 255 / 0.1)',
          fontSize: font,
        }}
      >
        <LayoutGrid size={Math.max(8, 18 * s * 1.1)} style={{ color: 'var(--accent)' }} />
        <span className="mx-0.5 h-3 w-px bg-white/15" />
        {taskApps.map((app) => (
          <span key={app.id}>
            <app.icon size={Math.max(8, 16 * s * 1.1)} />
          </span>
        ))}
        <span className="mx-0.5 h-3 w-px bg-white/15" />
        <span className="flex items-center gap-1 rounded bg-white/15 px-1.5 py-0.5">
          <span style={{ color: 'var(--accent)' }}>●</span>
          {d.taskbarLabels && <span>Nova Notes</span>}
        </span>
        <span className="flex-1" />
        <span className="text-white/70">{general.clockFormat === '24h' ? '17:30' : '5:30 PM'}</span>
      </div>

      {ripples.map((r) => (
        <span
          key={r.id}
          aria-hidden="true"
          className="pointer-events-none absolute rounded-full"
          style={{
            left: r.x - 16,
            top: r.y - 16,
            width: 32,
            height: 32,
            border: '2px solid var(--ripple)',
            animation: `nova-ripple ${0.7 / data.motion.speed}s ease-out forwards`,
          }}
        />
      ))}
    </div>
  );
}
