import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';

const custom = vi.hoisted(() => ({
  value: { cursor: null, status: 'idle' } as {
    cursor: unknown;
    status: 'idle' | 'loading' | 'ready' | 'failed';
  },
}));
vi.mock('../../settings/customCursor', async (orig) => ({
  ...(await orig<typeof import('../../settings/customCursor')>()),
  useCustomCursor: () => custom.value,
}));

import { Desktop } from '../Desktop';
import { registerBuiltinApps } from '../builtinApps';
import { useDisplay } from '../displayMode';
import { useShell } from '../shellStore';
import { launchApp } from '../../kernel/launcher';
import { useWindows } from '../../kernel/windowStore';
import { BITS_DEFAULTS, sanitizeBits } from '../../settings/bits';
import { DEFAULTS, patchSection } from '../../settings/schema';
import { useSettings } from '../../settings/store';

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
  custom.value = { cursor: null, status: 'idle' };
  useSettings.setState({ data: DEFAULTS, loaded: false });
  useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
  useShell.setState({ startOpen: false });
  useDisplay.setState({ mode: 'windowed', error: null });
});

const withBits = (patch: Record<string, object>, a11y: object = {}) => {
  const bits = sanitizeBits({ ...BITS_DEFAULTS, ...patch });
  let d = patchSection(DEFAULTS, 'bits', bits);
  d = patchSection(d, 'accessibility', a11y);
  useSettings.setState({ data: d, loaded: true });
};

const desktop = () => screen.getByTestId('desktop');
const move = (x: number, y: number) =>
  act(() => {
    fireEvent(
      screen.getByTestId('desktop-layer'),
      new MouseEvent('pointermove', { bubbles: true, clientX: x, clientY: y }),
    );
  });

describe('React Bits on the desktop', () => {
  it('with every effect off, nothing extra is on the page', () => {
    render(<Desktop />);
    expect(desktop().getAttribute('data-fx')).toBe('');
    expect(screen.queryByTestId('elastic-mesh')).not.toBeInTheDocument();
    expect(screen.queryByTestId('bits-overlay')).not.toBeInTheDocument();
  });

  it('Elastic Mesh adds one click-through canvas under the desktop layer', () => {
    withBits({ elasticMesh: { ...BITS_DEFAULTS.elasticMesh, enabled: true } });
    render(<Desktop />);
    const mesh = screen.getByTestId('elastic-mesh');
    expect(mesh).toHaveClass('pointer-events-none');
    // It comes before the layer in the page, so windows and icons paint above it.
    expect(
      mesh.compareDocumentPosition(screen.getByTestId('desktop-layer')) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it('cursor effects add one overlay canvas', () => {
    withBits({ clickSpark: { ...BITS_DEFAULTS.clickSpark, enabled: true } });
    render(<Desktop />);
    expect(screen.getByTestId('bits-overlay')).toBeInTheDocument();
  });

  it('turns on the CSS-only effects through the data-fx words', () => {
    withBits({
      chromaGrid: { ...BITS_DEFAULTS.chromaGrid, enabled: true },
      magicBento: { ...BITS_DEFAULTS.magicBento, enabled: true },
    });
    render(<Desktop />);
    expect(desktop().getAttribute('data-fx')).toBe('chroma bento');
  });

  it('Reduce motion keeps all of it off, without touching the saved settings', () => {
    withBits(
      {
        elasticMesh: { ...BITS_DEFAULTS.elasticMesh, enabled: true },
        clickSpark: { ...BITS_DEFAULTS.clickSpark, enabled: true },
        chromaGrid: { ...BITS_DEFAULTS.chromaGrid, enabled: true },
      },
      { reduceMotion: true },
    );
    render(<Desktop />);
    expect(screen.queryByTestId('elastic-mesh')).not.toBeInTheDocument();
    expect(screen.queryByTestId('bits-overlay')).not.toBeInTheDocument();
    expect(desktop().getAttribute('data-fx')).toBe('');
    expect(useSettings.getState().data.bits.clickSpark.enabled).toBe(true);
  });

  it('desktop icons, taskbar buttons, windows and Start tiles opt in with data-bit', async () => {
    withBits({});
    render(<Desktop />);
    expect(screen.getByRole('button', { name: 'Nova Files shortcut' })).toHaveAttribute(
      'data-bit',
      'magnet chroma',
    );
    expect(screen.getByRole('button', { name: 'Open Nova Files' })).toHaveAttribute(
      'data-bit',
      'magnet chroma',
    );
    expect(
      screen.getByRole('button', { name: 'Open Nova Files' }).querySelector('svg'),
    ).toHaveAttribute('data-bit', 'dock');
    await act(async () => void (await launchApp('calculator')));
    expect(screen.getByTestId('window-calculator')).toHaveAttribute('data-bit', 'glow');
    act(() => useShell.setState({ startOpen: true }));
    expect(screen.getByRole('menu', { name: 'Start menu' })).toHaveAttribute('data-bit', 'glow');
    expect(screen.getByRole('menuitem', { name: 'Nova Files' }).parentElement).toHaveAttribute(
      'data-bit',
      'bento chroma',
    );
  });

  it('the Start menu items glide in only when Animated List is on', () => {
    withBits({});
    const { unmount } = render(<Desktop />);
    act(() => useShell.setState({ startOpen: true }));
    expect(
      screen.getByRole('menuitem', { name: 'Nova Files' }).parentElement?.className,
    ).not.toContain('bit-li');
    unmount();
    useShell.setState({ startOpen: false });

    withBits({ animatedList: { ...BITS_DEFAULTS.animatedList, enabled: true, style: 'fade' } });
    render(<Desktop />);
    act(() => useShell.setState({ startOpen: true }));
    const tile = screen.getByRole('menuitem', { name: 'Nova Files' }).parentElement;
    expect(tile?.className).toContain('bit-li-fade');
    expect(tile?.style.getPropertyValue('--i')).toBe('0');
  });

  it('window titles use Fold Text only when it is on for titles', async () => {
    withBits({ foldText: { ...BITS_DEFAULTS.foldText, enabled: true, targets: ['titles'] } });
    render(<Desktop />);
    await act(async () => void (await launchApp('calculator')));
    const win = screen.getByTestId('window-calculator');
    expect(win.querySelectorAll('.bit-fold-l').length).toBeGreaterThan(0);
    // The clock and icon names are untouched: they were not chosen.
    expect(screen.getByLabelText('clock').querySelector('.bit-fold-l')).toBeNull();
  });
});

describe('a custom cursor file on the desktop', () => {
  const frame = { url: 'blob:mine', width: 32, height: 32, hotspot: [2, 2] as [number, number] };
  const loaded = { frames: [frame], sequence: [0], delays: [1000] };
  const move2 = () => move(60, 60);

  it('is drawn by NovaOS even in a normal window, and hides the Windows pointer only once drawn', () => {
    custom.value = { cursor: loaded, status: 'ready' };
    useSettings.setState({
      data: patchSection(DEFAULTS, 'cursor', { customFile: 'mine.cur' }),
      loaded: false,
    });
    render(<Desktop />);
    expect(desktop().getAttribute('data-cursor-engine')).toBe('system'); // nothing drawn yet
    move2();
    expect(desktop().getAttribute('data-cursor-engine')).toBe('nova');
    expect(screen.getByTestId('nova-cursor')).toHaveAttribute('src', 'blob:mine');
  });

  it('while it is still loading, or if it fails, the normal Windows pointer stays', () => {
    useSettings.setState({
      data: patchSection(DEFAULTS, 'cursor', { customFile: 'mine.cur' }),
      loaded: false,
    });
    for (const status of ['loading', 'failed'] as const) {
      custom.value = { cursor: null, status };
      const { unmount } = render(<Desktop />);
      move2();
      expect(desktop().getAttribute('data-cursor-engine'), status).toBe('system');
      expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
      unmount();
    }
  });

  it('choosing "Windows" for the pointer engine always wins over a custom file', () => {
    custom.value = { cursor: loaded, status: 'ready' };
    useSettings.setState({
      data: patchSection(DEFAULTS, 'cursor', { customFile: 'mine.cur', engine: 'system' }),
      loaded: false,
    });
    render(<Desktop />);
    move2();
    expect(desktop().getAttribute('data-cursor-engine')).toBe('system');
    expect(screen.queryByTestId('nova-cursor')).not.toBeInTheDocument();
  });
});

describe('rearranging desktop icons', () => {
  const names = () =>
    screen
      .getAllByRole('button', { name: /shortcut$/ })
      .map((b) => b.getAttribute('aria-label')?.replace(' shortcut', ''));

  const drag = (from: HTMLElement, to: HTMLElement | null, distance = 40) => {
    document.elementFromPoint = () => to;
    fireEvent.pointerDown(from, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(window, { clientX: 10, clientY: 10 + distance });
    fireEvent.pointerUp(window, { clientX: 10, clientY: 10 + distance });
  };

  it('dragging an icon onto another moves it there, and it is remembered', () => {
    render(<Desktop />);
    expect(names()[0]).toBe('Nova Files');
    const files = screen.getByRole('button', { name: 'Nova Files shortcut' });
    const calc = screen.getByRole('button', { name: 'Nova Calculator shortcut' });
    drag(files, calc);
    expect(useSettings.getState().data.desktop.desktopPinned).toEqual([
      'notes',
      'calculator',
      'files',
      'studio',
      'settings',
    ]);
    expect(names().slice(0, 3)).toEqual(['Nova Notes', 'Nova Calculator', 'Nova Files']);
  });

  it('a drag is not a click: nothing opens', () => {
    render(<Desktop />);
    const files = screen.getByRole('button', { name: 'Nova Files shortcut' });
    drag(files, screen.getByRole('button', { name: 'Nova Notes shortcut' }));
    fireEvent.click(files);
    expect(Object.keys(useWindows.getState().windows)).toHaveLength(0);
  });

  it('a tiny wobble while clicking is not a drag', () => {
    render(<Desktop />);
    const files = screen.getByRole('button', { name: 'Nova Files shortcut' });
    drag(files, screen.getByRole('button', { name: 'Nova Notes shortcut' }), 3);
    expect(useSettings.getState().data.desktop.desktopPinned[0]).toBe('files');
  });

  it('dropping on itself, on empty desktop, or with the right button changes nothing', () => {
    render(<Desktop />);
    const files = screen.getByRole('button', { name: 'Nova Files shortcut' });
    drag(files, files);
    drag(files, null);
    document.elementFromPoint = () => screen.getByRole('button', { name: 'Nova Notes shortcut' });
    fireEvent.pointerDown(files, { button: 2, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(window, { clientX: 10, clientY: 80 });
    fireEvent.pointerUp(window);
    expect(useSettings.getState().data.desktop.desktopPinned[0]).toBe('files');
  });

  it('the Recycle Bin stays put and cannot be dragged or dropped on', () => {
    render(<Desktop />);
    const bin = screen.getByRole('button', { name: 'Recycle Bin' });
    const files = screen.getByRole('button', { name: 'Nova Files shortcut' });
    drag(bin, files);
    drag(files, bin);
    expect(useSettings.getState().data.desktop.desktopPinned[0]).toBe('files');
  });
});
