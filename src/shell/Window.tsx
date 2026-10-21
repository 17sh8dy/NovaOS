import { useRef } from 'react';
import { motion } from 'motion/react';
import { Minus, Square, Copy, X } from 'lucide-react';
import { getApp } from '../kernel/appRegistry';
import { MIN_WINDOW, useWindows, type NovaWindow, type Size } from '../kernel/windowStore';
import { windowMotion } from '../settings/motion';
import { useSettings } from '../settings/store';

type Edge = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';
const EDGES: Edge[] = ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'];

const EDGE_STYLE: Record<Edge, string> = {
  n: 'top-0 left-2 right-2 h-1 cursor-n-resize',
  s: 'bottom-0 left-2 right-2 h-1 cursor-s-resize',
  e: 'right-0 top-2 bottom-2 w-1 cursor-e-resize',
  w: 'left-0 top-2 bottom-2 w-1 cursor-w-resize',
  ne: 'top-0 right-0 h-2 w-2 cursor-ne-resize',
  nw: 'top-0 left-0 h-2 w-2 cursor-nw-resize',
  se: 'bottom-0 right-0 h-2 w-2 cursor-se-resize',
  sw: 'bottom-0 left-0 h-2 w-2 cursor-sw-resize',
};

export function Window({ win, desktop }: { win: NovaWindow; desktop: Size }) {
  const { focus, close, minimize, toggleMaximize, move, setRect, activeId } = useWindows();
  const app = getApp(win.appId);
  const drag = useRef<{ px: number; py: number; x: number; y: number } | null>(null);
  const active = activeId === win.id;
  const motionSettings = useSettings((s) => s.data.motion);

  const startDrag = (e: React.PointerEvent) => {
    if (e.button !== 0 || win.maximized) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag.current = { px: e.clientX, py: e.clientY, x: win.rect.x, y: win.rect.y };
  };
  const onDrag = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    move(win.id, d.x + e.clientX - d.px, d.y + e.clientY - d.py, desktop);
  };
  const endDrag = () => {
    drag.current = null;
  };

  const startResize = (edge: Edge) => (e: React.PointerEvent) => {
    if (e.button !== 0 || win.maximized) return;
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const start = { px: e.clientX, py: e.clientY, ...win.rect };
    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - start.px;
      const dy = ev.clientY - start.py;
      let { x, y, width, height } = start;
      if (edge.includes('e')) width = start.width + dx;
      if (edge.includes('s')) height = start.height + dy;
      if (edge.includes('w')) {
        width = Math.max(MIN_WINDOW.width, start.width - dx);
        x = start.x + start.width - width; // keep the opposite edge anchored
      }
      if (edge.includes('n')) {
        height = Math.max(MIN_WINDOW.height, start.height - dy);
        y = Math.max(0, start.y + start.height - height);
        height = start.y + start.height - y;
      }
      setRect(win.id, { x, y, width, height });
    };
    const onUp = () => {
      el.removeEventListener('pointermove', onMove);
      el.removeEventListener('pointerup', onUp);
      el.removeEventListener('pointercancel', onUp);
    };
    el.addEventListener('pointermove', onMove);
    el.addEventListener('pointerup', onUp);
    el.addEventListener('pointercancel', onUp);
  };

  if (!app) return null;
  const Body = app.component;
  const Icon = app.icon;
  const style = win.maximized
    ? { left: 0, top: 0, width: '100%', height: '100%', zIndex: win.z }
    : {
        left: win.rect.x,
        top: win.rect.y,
        width: win.rect.width,
        height: win.rect.height,
        zIndex: win.z,
      };

  return (
    <motion.section
      role="dialog"
      aria-label={win.title}
      data-testid={`window-${win.appId}`}
      data-active={active}
      {...windowMotion(motionSettings.windowAnimation, win.minimized, motionSettings.speed)}
      style={{
        ...style,
        pointerEvents: win.minimized ? 'none' : 'auto',
        visibility: win.minimized ? 'hidden' : 'visible',
        borderColor: active ? 'var(--active-border)' : 'var(--border)',
        boxShadow: win.maximized ? 'none' : 'var(--shadow)',
      }}
      onPointerDown={() => focus(win.id)}
      className={`absolute flex flex-col overflow-hidden border bg-[var(--surface)] glass ${
        win.maximized ? 'rounded-none' : 'rounded-[var(--radius)]'
      }`}
    >
      <header
        style={{ height: 'var(--titlebar)' }}
        className="flex shrink-0 cursor-default items-center gap-2 bg-white/5 pr-1 pl-3 select-none"
        onPointerDown={startDrag}
        onPointerMove={onDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onDoubleClick={() => toggleMaximize(win.id)}
      >
        <Icon size={14} className="text-[var(--accent)]" />
        <span className="flex-1 truncate text-xs">{win.title}</span>
        <div
          className="flex"
          onPointerDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <TitleButton label="Minimize" onClick={() => minimize(win.id)}>
            <Minus size={14} />
          </TitleButton>
          <TitleButton
            label={win.maximized ? 'Restore' : 'Maximize'}
            onClick={() => toggleMaximize(win.id)}
          >
            {win.maximized ? <Copy size={12} /> : <Square size={12} />}
          </TitleButton>
          <TitleButton label="Close" danger onClick={() => close(win.id)}>
            <X size={14} />
          </TitleButton>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-hidden">
        <Body windowId={win.id} args={win.args} />
      </div>
      {!win.maximized &&
        EDGES.map((edge) => (
          <div
            key={edge}
            className={`absolute ${EDGE_STYLE[edge]}`}
            onPointerDown={startResize(edge)}
          />
        ))}
    </motion.section>
  );
}

function TitleButton(props: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      aria-label={props.label}
      title={props.label}
      onClick={props.onClick}
      className={`grid h-7 w-9 place-items-center rounded-md ${
        props.danger ? 'hover:bg-rose-500' : 'hover:bg-white/15'
      }`}
    >
      {props.children}
    </button>
  );
}
