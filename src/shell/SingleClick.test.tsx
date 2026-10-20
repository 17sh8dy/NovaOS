import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('../native/bridge', () => ({
  isTauri: () => true,
  native: vi.fn(),
}));

import { Desktop } from './Desktop';
import { registerBuiltinApps } from './builtinApps';
import { native } from '../native/bridge';
import { useWindows } from '../kernel/windowStore';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';
import { DEFAULTS } from '../settings/schema';
import { useSettings } from '../settings/store';

const nativeMock = vi.mocked(native);

beforeAll(() => {
  registerBuiltinApps();
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

beforeEach(() => {
  vi.clearAllMocks();
  nativeMock.mockResolvedValue(undefined);
  localStorage.clear();
  useSettings.setState({ data: DEFAULTS, loaded: false });
  useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
  useShell.setState({ startOpen: false });
  useDisplay.setState({ mode: 'windowed', error: null });
});

const openWindows = () => Object.values(useWindows.getState().windows).map((w) => w.appId);

describe('open with a single click', () => {
  it('off (default): a desktop icon needs a double click', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Nova Notes shortcut'));
    expect(openWindows()).toEqual([]);
    await user.dblClick(screen.getByLabelText('Nova Notes shortcut'));
    expect(openWindows()).toEqual(['notes']);
  });

  it('on: one click opens a desktop icon, and a double click does not open it twice', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('general', { singleClickOpen: true });
    render(<Desktop />);
    await user.click(screen.getByLabelText('Nova Notes shortcut'));
    expect(openWindows()).toEqual(['notes']);
    // Notes is a singleton, so a double click (two clicks) still leaves exactly one window.
    await user.dblClick(screen.getByLabelText('Nova Notes shortcut'));
    expect(openWindows()).toEqual(['notes']);
  });

  it('taskbar and Start menu already open with one click in both modes', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Open Nova Calculator'));
    expect(openWindows()).toEqual(['calculator']);
    await user.click(screen.getByLabelText('Start'));
    await user.click(screen.getByRole('menuitem', { name: 'Nova Notes' }));
    expect(openWindows()).toContain('notes');
  });

  it('window chrome keeps its double-click-to-maximize regardless of the setting', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('general', { singleClickOpen: true });
    render(<Desktop />);
    await user.click(screen.getByLabelText('Nova Notes shortcut'));
    const win = screen.getByTestId('window-notes');
    const titlebar = win.querySelector('header') as HTMLElement;
    await user.click(titlebar);
    expect(Object.values(useWindows.getState().windows)[0].maximized).toBe(false);
    await user.dblClick(titlebar);
    expect(Object.values(useWindows.getState().windows)[0].maximized).toBe(true);
  });
});

describe('Recycle Bin', () => {
  it('is on the desktop by default and opens the host Recycle Bin (double click, single off)', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    const bin = screen.getByLabelText('Recycle Bin');
    await user.click(bin);
    expect(nativeMock).not.toHaveBeenCalledWith('open_recycle_bin');
    await user.dblClick(bin);
    expect(nativeMock).toHaveBeenCalledWith('open_recycle_bin');
  });

  it('opens with one click when single-click is on', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('general', { singleClickOpen: true });
    render(<Desktop />);
    await user.click(screen.getByLabelText('Recycle Bin'));
    expect(nativeMock).toHaveBeenCalledWith('open_recycle_bin');
  });

  it('stays on the desktop when desktop icons are hidden, and every app is unpinned', () => {
    useSettings.getState().set('desktop', {
      showDesktopIcons: false,
      unpinned: ['files', 'settings', 'notes', 'calculator'],
    });
    render(<Desktop />);
    expect(screen.getByLabelText('Recycle Bin')).toBeInTheDocument();
    expect(screen.queryByLabelText('Nova Files shortcut')).not.toBeInTheDocument();
  });

  it('cannot be removed by importing hostile settings', () => {
    useSettings
      .getState()
      .importJson(
        '{"app":"novaos","version":1,"settings":{"desktop":{"showDesktopIcons":false,"unpinned":["recycle-bin","recycle","bin"]}}}',
      );
    render(<Desktop />);
    expect(screen.getByLabelText('Recycle Bin')).toBeInTheDocument();
  });

  it('shows the reason when the bin cannot be opened, then clears it', async () => {
    const user = userEvent.setup();
    nativeMock.mockRejectedValue('could not open the Recycle Bin: not found');
    render(<Desktop />);
    await user.dblClick(screen.getByLabelText('Recycle Bin'));
    expect(await screen.findByRole('status')).toHaveTextContent('could not open the Recycle Bin');
    await waitFor(() => expect(screen.queryByRole('status')).not.toBeInTheDocument(), {
      timeout: 6000,
    });
  }, 10000);
});
