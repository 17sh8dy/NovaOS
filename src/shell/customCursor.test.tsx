import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { NovaCursor, useNovaPointer } from './NovaCursor';
import type { LoadedCursor } from '../settings/customCursor';
import { DEFAULTS } from '../settings/schema';

const cursor = DEFAULTS.cursor;

const move = (target: Element | Window, x: number, y: number) =>
  fireEvent(target, new MouseEvent('pointermove', { bubbles: true, clientX: x, clientY: y }));

const still = (hotspot: [number, number] | null = [4, 6]): LoadedCursor => ({
  frames: [{ url: 'blob:still', width: 32, height: 32, hotspot }],
  sequence: [0],
  delays: [1000],
});

const animated = (): LoadedCursor => ({
  frames: [
    { url: 'blob:f0', width: 32, height: 32, hotspot: [0, 0] },
    { url: 'blob:f1', width: 32, height: 32, hotspot: [0, 0] },
    { url: 'blob:f2', width: 32, height: 32, hotspot: [0, 0] },
  ],
  sequence: [0, 1, 2, 1],
  delays: [100, 100, 100, 300],
});

describe('who draws the pointer, with a custom cursor file', () => {
  const resolve = (engine: 'auto' | 'nova' | 'system', mode: string, custom: boolean) =>
    renderHook(() => useNovaPointer(engine, mode, custom)).result.current;

  it('NovaOS draws it whenever a custom cursor is loaded, windowed or not', () => {
    expect(resolve('auto', 'windowed', true)).toBe(true);
    expect(resolve('nova', 'windowed', true)).toBe(true);
    expect(resolve('auto', 'windowed', false)).toBe(false);
  });

  it('but choosing Windows always wins', () => {
    expect(resolve('system', 'windowed', true)).toBe(false);
    expect(resolve('system', 'immersive', true)).toBe(false);
  });

  it('Immersive Mode behaves as before', () => {
    expect(resolve('auto', 'immersive', false)).toBe(true);
    expect(resolve('system', 'immersive', false)).toBe(false);
  });
});

describe('NovaCursor with a custom file', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('draws the file instead of the built-in arrow, with the hotspot from the file', () => {
    render(<NovaCursor active cursor={cursor} onReady={vi.fn()} custom={still([4, 6])} />);
    act(() => move(document.body, 100, 80));
    const img = screen.getByTestId('nova-cursor');
    expect(img).toHaveAttribute('src', 'blob:still');
    expect(img.style.transform).toBe('translate3d(96px, 74px, 0)');
    expect(img.style.width).toBe('32px');
  });

  it('scales with the Pointer size setting', () => {
    render(
      <NovaCursor
        active
        cursor={{ ...cursor, size: 48 }}
        onReady={vi.fn()}
        custom={still([4, 6])}
      />,
    );
    act(() => move(document.body, 100, 100));
    const img = screen.getByTestId('nova-cursor');
    expect(img.style.width).toBe('48px');
    expect(img.style.transform).toBe('translate3d(94px, 91px, 0)'); // hotspot (6, 9) at 1.5x
  });

  it('a PNG or GIF points where the setting says: corner or center', () => {
    const { rerender } = render(
      <NovaCursor active cursor={cursor} onReady={vi.fn()} custom={still(null)} />,
    );
    act(() => move(document.body, 100, 100));
    expect(screen.getByTestId('nova-cursor').style.transform).toBe('translate3d(100px, 100px, 0)');
    rerender(
      <NovaCursor
        active
        cursor={{ ...cursor, customHotspot: 'center' }}
        onReady={vi.fn()}
        custom={still(null)}
      />,
    );
    expect(screen.getByTestId('nova-cursor').style.transform).toBe('translate3d(84px, 84px, 0)');
  });

  it('keeps the built-in I-beam over text unless it is told to use the file everywhere', () => {
    const field = document.createElement('input');
    document.body.appendChild(field);
    const { rerender } = render(
      <NovaCursor active cursor={cursor} onReady={vi.fn()} custom={still()} />,
    );
    act(() => move(field, 50, 50));
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('data-shape', 'text');
    expect(screen.getByTestId('nova-cursor').getAttribute('src')).not.toBe('blob:still');
    rerender(
      <NovaCursor
        active
        cursor={{ ...cursor, customEverywhere: true }}
        onReady={vi.fn()}
        custom={still()}
      />,
    );
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('src', 'blob:still');
    field.remove();
  });

  it('plays an animated cursor frame by frame, following its sequence and timing', () => {
    render(<NovaCursor active cursor={cursor} onReady={vi.fn()} custom={animated()} />);
    act(() => move(document.body, 10, 10));
    const src = () => screen.getByTestId('nova-cursor').getAttribute('src');
    expect(src()).toBe('blob:f0');
    act(() => void vi.advanceTimersByTime(100));
    expect(src()).toBe('blob:f1');
    act(() => void vi.advanceTimersByTime(100));
    expect(src()).toBe('blob:f2');
    act(() => void vi.advanceTimersByTime(100));
    expect(src()).toBe('blob:f1'); // the sequence goes 0, 1, 2, 1 ...
    act(() => void vi.advanceTimersByTime(299));
    expect(src()).toBe('blob:f1');
    act(() => void vi.advanceTimersByTime(1));
    expect(src()).toBe('blob:f0'); // ... and loops
  });

  it('holds still on the first frame when animation is off (Reduce motion)', () => {
    render(
      <NovaCursor active cursor={cursor} onReady={vi.fn()} custom={animated()} animate={false} />,
    );
    act(() => move(document.body, 10, 10));
    act(() => void vi.advanceTimersByTime(5000));
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('src', 'blob:f0');
  });

  it('stops its timer when it goes away', () => {
    const { unmount } = render(
      <NovaCursor active cursor={cursor} onReady={vi.fn()} custom={animated()} />,
    );
    act(() => move(document.body, 10, 10));
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('still hides the native pointer only once it has a position to draw', () => {
    const onReady = vi.fn();
    render(<NovaCursor active cursor={cursor} onReady={onReady} custom={still()} />);
    expect(onReady).not.toHaveBeenCalledWith(true);
    act(() => move(document.body, 5, 5));
    expect(onReady).toHaveBeenCalledWith(true);
  });

  it('without a custom file, it draws exactly the built-in pointer as before', () => {
    render(<NovaCursor active cursor={cursor} onReady={vi.fn()} custom={null} />);
    act(() => move(document.body, 100, 80));
    expect(screen.getByTestId('nova-cursor').getAttribute('src')).toMatch(/^data:image\/svg\+xml/);
  });
});
