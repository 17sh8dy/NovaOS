import type { LucideIcon } from 'lucide-react';
import { useOpenProps } from '../settings/store';

export interface DesktopIconProps {
  id: string;
  label: string;
  ariaLabel: string;
  title?: string;
  icon: LucideIcon;
  iconPx: number;
  /** Tint for the glyph; defaults to the accent color. */
  iconClass?: string;
  selected: boolean;
  /** Called on every click so the desktop can track the selection. */
  onSelect: (id: string, additive: boolean) => void;
  open: () => void;
  /** Lets the desktop measure this icon for drag-selection. */
  register: (id: string, el: HTMLElement | null) => void;
}

/**
 * One icon on the desktop: apps and the Recycle Bin share it, so selection and "open with one
 * click or two" behave identically everywhere.
 */
export function DesktopIcon(p: DesktopIconProps) {
  const openProps = useOpenProps();
  const open = openProps(p.open);
  return (
    <button
      ref={(el) => p.register(p.id, el)}
      aria-label={p.ariaLabel}
      aria-pressed={p.selected}
      title={p.title}
      onClick={(e) => {
        p.onSelect(p.id, e.ctrlKey || e.metaKey);
        open.onClick?.();
      }}
      onDoubleClick={open.onDoubleClick}
      onKeyDown={(e) => e.key === 'Enter' && p.open()}
      style={
        p.selected
          ? {
              background: 'color-mix(in srgb, var(--selection) 28%, transparent)',
              boxShadow: 'inset 0 0 0 1px var(--selection)',
            }
          : undefined
      }
      className="flex w-20 flex-col items-center gap-1 rounded-lg p-2 text-[11px] hover:bg-white/10 focus:outline-none focus-visible:ring-1 focus-visible:ring-[var(--selection)]"
    >
      <p.icon size={p.iconPx} className={p.iconClass ?? 'text-[var(--accent)]'} />
      <span className="text-center leading-tight">{p.label}</span>
    </button>
  );
}
