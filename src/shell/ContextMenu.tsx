import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';

export interface MenuItem {
  label: string;
  onClick: () => void;
  danger?: boolean;
}

interface OpenState {
  x: number;
  y: number;
  items: MenuItem[];
}

const WIDTH = 210;
const ROW = 32;

/**
 * A right-click menu. `open(event, items)` shows it at the pointer; it closes on any outside
 * click, Escape, scroll or focus loss, and always stays inside the screen.
 */
export function useContextMenu(): {
  open: (e: React.MouseEvent, items: MenuItem[]) => void;
  menu: ReactNode;
} {
  const [state, setState] = useState<OpenState | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setState(null), []);

  const open = useCallback((e: React.MouseEvent, items: MenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    if (items.length === 0) return;
    setState({ x: e.clientX, y: e.clientY, items });
  }, []);

  useEffect(() => {
    if (!state) return;
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('blur', close);
    window.addEventListener('wheel', close, { passive: true });
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('blur', close);
      window.removeEventListener('wheel', close);
    };
  }, [state, close]);

  const menu = state && (
    <div
      ref={ref}
      role="menu"
      aria-label="Context menu"
      style={{
        left: Math.max(4, Math.min(state.x, window.innerWidth - WIDTH - 4)),
        top: Math.max(4, Math.min(state.y, window.innerHeight - state.items.length * ROW - 12)),
        width: WIDTH,
      }}
      className="glass fixed z-[200001] rounded-lg border border-white/15 bg-[var(--surface)] p-1 text-xs shadow-2xl"
    >
      {state.items.map((item) => (
        <button
          key={item.label}
          role="menuitem"
          onClick={() => {
            close();
            item.onClick();
          }}
          className={`flex h-8 w-full items-center rounded-md px-3 text-left ${
            item.danger ? 'text-rose-300 hover:bg-rose-500/20' : 'hover:bg-white/10'
          }`}
        >
          {item.label}
        </button>
      ))}
    </div>
  );

  return { open, menu };
}
