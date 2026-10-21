import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { NovaCursor, useNovaPointer } from './NovaCursor';
import { Desktop } from './Desktop';
import { registerBuiltinApps } from './builtinApps';
import { drawShape, shapeFor, toDataUri } from './cursorShapes';
import { useWindows } from '../kernel/windowStore';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';
import { DEFAULTS, sanitize } from '../settings/schema';
import { useSettings } from '../settings/store';
import { renderHook } from '@testing-library/react';

const cursor = DEFAULTS.cursor;

const move = (
  target: Element | Window,
  x: number,
  y: number,
  init: MouseEventInit & { pointerType?: string } = {},
) => {
  const e = new MouseEvent('pointermove', { bubbles: true, clientX: x, clientY: y, ...init });
  if (init.pointerType) Object.defineProperty(e, 'pointerType', { value: init.pointerType });
  fireEvent(target, e);
};

describe('who draws the pointer', () => {
  it('automatic means NovaOS in Immersive Mode only; Always NovaOS and Windows are unconditional', () => {
    const resolve = (engine: 'auto' | 'nova' | 'system', mode: string) =>
      renderHook(() => useNovaPointer(engine, mode)).result.current;
    expect(resolve('auto', 'windowed')).toBe(false);
    expect(resolve('auto', 'immersive')).toBe(true);
    expect(resolve('nova', 'windowed')).toBe(true);
    expect(resolve('nova', 'immersive')).toBe(true);
    expect(resolve('system', 'immersive')).toBe(false);
    expect(resolve('system', 'windowed')).toBe(false);
  });

  it('defaults to automatic, and rejects unknown engines', () => {
    expect(DEFAULTS.cursor.engine).toBe('auto');
    expect(sanitize({ cursor: { engine: 'nova' } }).cursor.engine).toBe('nova');
    for (const bad of ['hardware', '', 5, null, undefined])
      expect(sanitize({ cursor: { engine: bad } }).cursor.engine).toBe('auto');
  });
});

describe('pointer shapes', () => {
  const el = (html: string) => {
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.appendChild(host);
    return host.firstElementChild as Element;
  };

  it('picks the shape the system would', () => {
    expect(shapeFor(el('<button>x</button>'))).toBe('arrow');
    expect(shapeFor(el('<div class="cursor-default">x</div>'))).toBe('arrow');
    expect(shapeFor(el('<textarea></textarea>'))).toBe('text');
    expect(shapeFor(el('<input type="text" />'))).toBe('text');
    expect(shapeFor(el('<input />'))).toBe('text');
    expect(shapeFor(el('<div contenteditable="true">x</div>'))).toBe('text');
    expect(shapeFor(el('<input type="range" />'))).toBe('arrow');
    expect(shapeFor(el('<input type="checkbox" />'))).toBe('arrow');
    expect(shapeFor(el('<input disabled />'))).toBe('arrow');
    expect(shapeFor(el('<a href="https://x.test">x</a>'))).toBe('pointer');
    expect(shapeFor(el('<span><a href="/y"><b>inner</b></a></span>').querySelector('b'))).toBe(
      'pointer',
    );
    expect(shapeFor(el('<a>no href</a>'))).toBe('arrow');
    expect(shapeFor(el('<div class="absolute cursor-n-resize"></div>'))).toBe('resize-ns');
    expect(shapeFor(el('<div class="cursor-s-resize"></div>'))).toBe('resize-ns');
    expect(shapeFor(el('<div class="cursor-e-resize"></div>'))).toBe('resize-ew');
    expect(shapeFor(el('<div class="cursor-w-resize"></div>'))).toBe('resize-ew');
    expect(shapeFor(el('<div class="cursor-ne-resize"></div>'))).toBe('resize-nesw');
    expect(shapeFor(el('<div class="cursor-sw-resize"></div>'))).toBe('resize-nesw');
    expect(shapeFor(el('<div class="cursor-nw-resize"></div>'))).toBe('resize-nwse');
    expect(shapeFor(el('<div class="cursor-se-resize"></div>'))).toBe('resize-nwse');
    expect(shapeFor(null)).toBe('arrow');
    expect(shapeFor(window)).toBe('arrow');
  });

  it('draws every shape in the chosen colors, with its hotspot inside the image', () => {
    const shapes = [
      'arrow',
      'text',
      'pointer',
      'resize-ew',
      'resize-ns',
      'resize-nesw',
      'resize-nwse',
    ] as const;
    for (const style of ['classic', 'dot', 'crosshair'] as const) {
      for (const shape of shapes) {
        const d = drawShape(shape, { color: '#ff00aa', outline: '#001122', size: 40, style });
        expect(d.svg).toContain('#ff00aa');
        expect(d.svg).toContain('#001122');
        expect(d.svg).toContain('width="40"');
        expect(d.svg).not.toMatch(/script|onload|javascript/i);
        expect(d.hotspot[0]).toBeGreaterThanOrEqual(0);
        expect(d.hotspot[0]).toBeLessThanOrEqual(40);
        expect(d.hotspot[1]).toBeGreaterThanOrEqual(0);
        expect(d.hotspot[1]).toBeLessThanOrEqual(40);
      }
    }
    // the arrow points at its tip; the dot and the rest at their middle
    expect(drawShape('arrow', { ...cursor, size: 32, style: 'classic' }).hotspot).toEqual([4, 2]);
    expect(drawShape('arrow', { ...cursor, size: 32, style: 'dot' }).hotspot).toEqual([16, 16]);
    expect(drawShape('text', { ...cursor, size: 32, style: 'classic' }).hotspot).toEqual([16, 16]);
    expect(toDataUri('<svg a="1 2"/>')).toMatch(/^data:image\/svg\+xml,%3Csvg/);
  });
});

describe('NovaCursor', () => {
  it('draws nothing while inactive', () => {
    const onReady = vi.fn();
    render(<NovaCursor active={false} cursor={cursor} onReady={onReady} />);
    move(window, 10, 10);
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
    expect(onReady).not.toHaveBeenCalledWith(true);
  });

  it('only reports ready (so the native pointer may be hidden) after it has a position to draw', () => {
    const onReady = vi.fn();
    render(<NovaCursor active cursor={cursor} onReady={onReady} />);
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
    expect(onReady).not.toHaveBeenCalledWith(true); // nothing is hidden before there is something to show
    act(() => move(document.body, 100, 80));
    expect(screen.getByTestId('nova-cursor')).toBeInTheDocument();
    expect(onReady).toHaveBeenCalledWith(true);
    expect(onReady).toHaveBeenCalledTimes(1); // exactly once: true, on the first move
  });

  it('follows the pointer, offset by the hotspot', () => {
    render(<NovaCursor active cursor={{ ...cursor, size: 32 }} onReady={vi.fn()} />);
    act(() => move(document.body, 100, 80));
    const img = screen.getByTestId('nova-cursor');
    expect(img.style.transform).toBe('translate3d(96px, 78px, 0)'); // classic arrow tip is (4,2)
    act(() => move(document.body, 300, 200));
    expect(img.style.transform).toBe('translate3d(296px, 198px, 0)');
    expect(img.style.width).toBe('32px');
    expect(img.getAttribute('aria-hidden')).toBe('true');
    expect(img.style.pointerEvents).toBe(''); // via class; it must never intercept clicks
    expect(img.className).toContain('pointer-events-none');
  });

  it('changes shape over text and links, and back', () => {
    render(
      <div>
        <textarea data-testid="t" />
        <a href="https://x.test" data-testid="l">
          link
        </a>
        <p data-testid="p">plain</p>
        <NovaCursor active cursor={cursor} onReady={vi.fn()} />
      </div>,
    );
    act(() => move(screen.getByTestId('p'), 5, 5));
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('data-shape', 'arrow');
    act(() => move(screen.getByTestId('t'), 6, 6));
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('data-shape', 'text');
    act(() => move(screen.getByTestId('l'), 7, 7));
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('data-shape', 'pointer');
    act(() => move(screen.getByTestId('p'), 8, 8));
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('data-shape', 'arrow');
  });

  it('uses the chosen style, color and size; the system style falls back to the classic arrow', () => {
    const { rerender } = render(
      <NovaCursor active cursor={{ ...cursor, style: 'system' }} onReady={vi.fn()} />,
    );
    act(() => move(document.body, 10, 10));
    const uri = () =>
      decodeURIComponent((screen.getByTestId('nova-cursor') as HTMLImageElement).src);
    expect(uri()).toContain('<path d="M4 2 L4 24'); // classic arrow
    rerender(
      <NovaCursor
        active
        cursor={{ ...cursor, style: 'dot', color: '#00ff00', size: 24 }}
        onReady={vi.fn()}
      />,
    );
    expect(uri()).toContain('<circle');
    expect(uri()).toContain('#00ff00');
    expect(screen.getByTestId('nova-cursor').style.width).toBe('24px');
    // after a size change under a still mouse, it re-centers on the same spot
    expect(screen.getByTestId('nova-cursor').style.transform).toBe('translate3d(-2px, -2px, 0)'); // dot hotspot (12,12) at (10,10)
  });

  it('ignores touch', () => {
    render(<NovaCursor active cursor={cursor} onReady={vi.fn()} />);
    act(() => move(document.body, 10, 10, { pointerType: 'touch' }));
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
  });

  it('hides when the mouse leaves the window and comes back on the next move; focus loss never hides it', () => {
    render(<NovaCursor active cursor={cursor} onReady={vi.fn()} />);
    act(() => move(document.body, 10, 10));
    act(() => void window.dispatchEvent(new Event('blur')));
    expect(screen.getByTestId('nova-cursor').style.display).toBe('block'); // still drawn: the native one is hidden too
    act(() => void document.documentElement.dispatchEvent(new Event('mouseleave')));
    expect(screen.getByTestId('nova-cursor').style.display).toBe('none');
    act(() => move(document.body, 20, 20));
    expect(screen.getByTestId('nova-cursor').style.display).toBe('block');
  });

  it('goes away and un-hides the native pointer when deactivated', () => {
    const onReady = vi.fn();
    const { rerender } = render(<NovaCursor active cursor={cursor} onReady={onReady} />);
    act(() => move(document.body, 10, 10));
    expect(onReady).toHaveBeenLastCalledWith(true);
    rerender(<NovaCursor active={false} cursor={cursor} onReady={onReady} />);
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
    expect(onReady).toHaveBeenLastCalledWith(false);
    act(() => move(document.body, 30, 30));
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
  });
});

describe('on the desktop', () => {
  beforeAll(() => {
    registerBuiltinApps();
    globalThis.ResizeObserver ??= class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  });

  beforeEach(() => {
    localStorage.clear();
    useSettings.setState({ data: DEFAULTS, loaded: false });
    useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
    useShell.setState({ startOpen: false });
    useDisplay.setState({ mode: 'windowed', error: null });
  });

  const engine = () => screen.getByTestId('desktop').getAttribute('data-cursor-engine');
  const moveOnDesktop = () => act(() => move(screen.getByTestId('desktop-layer'), 50, 50));

  it('windowed + automatic: the Windows pointer is untouched', () => {
    render(<Desktop />);
    moveOnDesktop();
    expect(engine()).toBe('system');
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
  });

  it('Immersive Mode: NovaOS draws its own pointer, and the native one is hidden only once it shows', () => {
    render(<Desktop />);
    act(() => useDisplay.setState({ mode: 'immersive' }));
    expect(engine()).toBe('system'); // not hidden yet: nothing drawn to replace it
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
    moveOnDesktop();
    expect(engine()).toBe('nova');
    expect(screen.getByTestId('nova-cursor')).toBeInTheDocument();
  });

  it('leaving Immersive Mode gives the Windows pointer straight back', () => {
    render(<Desktop />);
    act(() => useDisplay.setState({ mode: 'immersive' }));
    moveOnDesktop();
    expect(engine()).toBe('nova');
    act(() => useDisplay.setState({ mode: 'windowed' }));
    expect(engine()).toBe('system');
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
  });

  it('"Always NovaOS" works windowed; "Windows" never uses it, even in Immersive', () => {
    useSettings.getState().set('cursor', { engine: 'nova' });
    const { unmount } = render(<Desktop />);
    moveOnDesktop();
    expect(engine()).toBe('nova');
    unmount();

    useSettings.getState().set('cursor', { engine: 'system' });
    useDisplay.setState({ mode: 'immersive' });
    render(<Desktop />);
    moveOnDesktop();
    expect(engine()).toBe('system');
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
  });

  it('the pointer keeps working across window drags and over text', () => {
    useSettings.getState().set('cursor', { engine: 'nova' });
    render(<Desktop />);
    act(() => void window.dispatchEvent(new Event('focus')));
    moveOnDesktop();
    const img = screen.getByTestId('nova-cursor');
    act(() => move(screen.getByTestId('desktop-layer'), 400, 300));
    expect(img.style.transform).toContain('396px');
  });
});
