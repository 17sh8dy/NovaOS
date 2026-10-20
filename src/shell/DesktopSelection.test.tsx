import { beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Desktop } from './Desktop';
import { registerBuiltinApps } from './builtinApps';
import { useWindows } from '../kernel/windowStore';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';
import { DEFAULTS } from '../settings/schema';
import { useSettings } from '../settings/store';

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

type R = { left: number; top: number; right: number; bottom: number };
const fakeRect = (r: R) => () =>
  ({
    ...r,
    x: r.left,
    y: r.top,
    width: r.right - r.left,
    height: r.bottom - r.top,
    toJSON() {},
  }) as DOMRect;

/** jsdom has no layout, so give the desktop and its icons known rectangles. */
function layout() {
  render(<Desktop />);
  const layer = screen.getByTestId('desktop-layer');
  layer.getBoundingClientRect = fakeRect({ left: 0, top: 0, right: 800, bottom: 600 });
  const slots: [string, R][] = [
    ['Recycle Bin', { left: 10, top: 10, right: 90, bottom: 90 }],
    ['Nova Files shortcut', { left: 10, top: 100, right: 90, bottom: 180 }],
    ['Nova Settings shortcut', { left: 10, top: 190, right: 90, bottom: 270 }],
    ['Nova Notes shortcut', { left: 10, top: 280, right: 90, bottom: 360 }],
    ['Nova Calculator shortcut', { left: 10, top: 370, right: 90, bottom: 450 }],
  ];
  for (const [name, r] of slots) screen.getByLabelText(name).getBoundingClientRect = fakeRect(r);
  return layer;
}

const ptr = (el: Element, type: string, x: number, y: number, extra: MouseEventInit = {}) =>
  fireEvent(
    el,
    new MouseEvent(type, {
      bubbles: true,
      cancelable: true,
      clientX: x,
      clientY: y,
      button: 0,
      ...extra,
    }),
  );

const isSelected = (name: string) =>
  screen.getByLabelText(name).getAttribute('aria-pressed') === 'true';

describe('desktop selection rectangle', () => {
  it('dragging on empty desktop draws a box and selects the icons it touches', () => {
    const layer = layout();
    ptr(layer, 'pointerdown', 300, 300);
    ptr(layer, 'pointermove', 50, 150);
    expect(screen.getByTestId('selection-box')).toBeInTheDocument();
    expect(isSelected('Nova Files shortcut')).toBe(true);
    expect(isSelected('Nova Settings shortcut')).toBe(true);
    expect(isSelected('Nova Notes shortcut')).toBe(true); // box reaches y=300, its top is 280
    expect(isSelected('Recycle Bin')).toBe(false);
    expect(isSelected('Nova Calculator shortcut')).toBe(false);
    ptr(layer, 'pointerup', 50, 150);
    expect(screen.queryByTestId('selection-box')).not.toBeInTheDocument();
    expect(isSelected('Nova Files shortcut')).toBe(true); // selection outlives the drag
  });

  it('the box is drawn in desktop coordinates and clamped to the desktop', () => {
    const layer = layout();
    ptr(layer, 'pointerdown', 700, 500);
    ptr(layer, 'pointermove', 2000, 2000); // far outside
    const el = screen.getByTestId('selection-box');
    expect(el.style.left).toBe('700px');
    expect(el.style.top).toBe('500px');
    expect(el.style.width).toBe('100px'); // clamped at the 800px edge
    expect(el.style.height).toBe('100px');
  });

  it('shrinking the box deselects what it no longer touches', () => {
    const layer = layout();
    ptr(layer, 'pointerdown', 300, 460);
    ptr(layer, 'pointermove', 50, 100);
    expect(isSelected('Nova Calculator shortcut')).toBe(true);
    expect(isSelected('Nova Files shortcut')).toBe(true);
    ptr(layer, 'pointermove', 50, 400); // box is now y 400..460: Calculator (370-450) still touched
    expect(isSelected('Nova Calculator shortcut')).toBe(true);
    expect(isSelected('Nova Notes shortcut')).toBe(false); // 280-360 is above the box now
    expect(isSelected('Nova Files shortcut')).toBe(false);
    ptr(layer, 'pointermove', 50, 455); // y 455..460: nothing touched
    expect(isSelected('Nova Calculator shortcut')).toBe(false);
  });

  it('a click or tiny wiggle never flashes a box', () => {
    const layer = layout();
    ptr(layer, 'pointerdown', 300, 300);
    ptr(layer, 'pointermove', 301, 301);
    expect(screen.queryByTestId('selection-box')).not.toBeInTheDocument();
  });

  it('clicking empty desktop clears the selection; Ctrl+drag adds to it', () => {
    const layer = layout();
    ptr(layer, 'pointerdown', 300, 300);
    ptr(layer, 'pointermove', 50, 150);
    ptr(layer, 'pointerup', 50, 150);
    expect(isSelected('Nova Files shortcut')).toBe(true);

    ptr(layer, 'pointerdown', 400, 100, { ctrlKey: true });
    ptr(layer, 'pointermove', 20, 20, { ctrlKey: true });
    ptr(layer, 'pointerup', 20, 20, { ctrlKey: true });
    expect(isSelected('Recycle Bin')).toBe(true);
    expect(isSelected('Nova Files shortcut')).toBe(true); // kept

    ptr(layer, 'pointerdown', 600, 550);
    ptr(layer, 'pointerup', 600, 550);
    expect(isSelected('Recycle Bin')).toBe(false);
    expect(isSelected('Nova Files shortcut')).toBe(false);
  });

  it('pressing on an icon or a window never starts a box', () => {
    const layer = layout();
    ptr(screen.getByLabelText('Nova Files shortcut'), 'pointerdown', 50, 150);
    ptr(layer, 'pointermove', 400, 400);
    expect(screen.queryByTestId('selection-box')).not.toBeInTheDocument();
  });

  it('right button does nothing', () => {
    const layer = layout();
    ptr(layer, 'pointerdown', 300, 300, { button: 2 });
    ptr(layer, 'pointermove', 50, 150);
    expect(screen.queryByTestId('selection-box')).not.toBeInTheDocument();
  });

  it('can be switched off, but clicking empty desktop still clears icon selection', () => {
    useSettings.getState().set('appearance', { selectionEnabled: false });
    const layer = layout();
    fireEvent.click(screen.getByLabelText('Nova Files shortcut'));
    expect(isSelected('Nova Files shortcut')).toBe(true);
    ptr(layer, 'pointerdown', 300, 300);
    ptr(layer, 'pointermove', 50, 150);
    expect(screen.queryByTestId('selection-box')).not.toBeInTheDocument();
    expect(isSelected('Nova Files shortcut')).toBe(false);
  });

  it('uses the chosen color and fill strength, following the accent by default', () => {
    useSettings.getState().set('appearance', { accent: 'teal', selectionOpacity: 0.35 });
    const layer = layout();
    expect(screen.getByTestId('desktop').style.getPropertyValue('--selection')).toBe('#14b8a6');
    ptr(layer, 'pointerdown', 300, 300);
    ptr(layer, 'pointermove', 50, 150);
    const box = screen.getByTestId('selection-box');
    expect(box.style.background).toContain('35%');
    expect(box.style.border).toContain('var(--selection)');

    act(() => useSettings.getState().set('appearance', { selectionColor: '#ff00aa' }));
    expect(screen.getByTestId('desktop').style.getPropertyValue('--selection')).toBe('#ff00aa');
    act(() =>
      useSettings.getState().set('appearance', { selectionColor: 'accent', accent: 'rose' }),
    );
    expect(screen.getByTestId('desktop').style.getPropertyValue('--selection')).toBe('#f43f5e');
  });

  it('plain clicking selects one icon, Ctrl+click toggles', () => {
    layout();
    fireEvent.click(screen.getByLabelText('Nova Files shortcut'));
    fireEvent.click(screen.getByLabelText('Nova Notes shortcut'));
    expect(isSelected('Nova Files shortcut')).toBe(false);
    expect(isSelected('Nova Notes shortcut')).toBe(true);
    fireEvent.click(screen.getByLabelText('Nova Files shortcut'), { ctrlKey: true });
    expect(isSelected('Nova Files shortcut')).toBe(true);
    fireEvent.click(screen.getByLabelText('Nova Files shortcut'), { ctrlKey: true });
    expect(isSelected('Nova Files shortcut')).toBe(false);
  });
});
