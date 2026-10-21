import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('../settings/sounds', async (orig) => ({
  ...(await orig<typeof import('../settings/sounds')>()),
  playSound: vi.fn(),
}));

import { Desktop } from './Desktop';
import { registerBuiltinApps } from './builtinApps';
import { Settings } from '../apps/settings/Settings';
import { useWindows } from '../kernel/windowStore';
import { launchApp } from '../kernel/launcher';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';
import { DEFAULTS } from '../settings/schema';
import { playSound } from '../settings/sounds';
import { useSettings } from '../settings/store';

const playMock = vi.mocked(playSound);
const data = () => useSettings.getState().data;
const set = useSettings.getState().set;

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
  localStorage.clear();
  useSettings.setState({ data: DEFAULTS, loaded: false });
  useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
  useShell.setState({ startOpen: false });
  useDisplay.setState({ mode: 'windowed', error: null });
});

const taskbar = () => screen.getByTestId('taskbar');
const taskbarLaunchers = () =>
  within(taskbar())
    .getAllByRole('button', { name: /^Open Nova/ })
    .map((b) => b.getAttribute('aria-label'));

describe('Nova Studio on the shell', () => {
  it('is pinned to the taskbar and the desktop by default, and opens', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    expect(screen.getByLabelText('Open Nova Studio')).toBeInTheDocument();
    expect(screen.getByLabelText('Nova Studio shortcut')).toBeInTheDocument();
    await user.click(screen.getByLabelText('Open Nova Studio'));
    expect(Object.values(useWindows.getState().windows).map((w) => w.appId)).toEqual(['studio']);
    expect(screen.getByTestId('studio')).toBeInTheDocument();
  });

  it('opens from Settings, and Settings points at it', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    // Settings keeps system pages only
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    for (const keep of [
      'General',
      'Files & Storage',
      'Keyboard shortcuts',
      'Privacy & Security',
      'Data & Backup',
      'About',
    ])
      expect(within(nav).getByText(keep)).toBeInTheDocument();
    for (const gone of ['Appearance', 'Desktop & Taskbar', 'Accessibility'])
      expect(within(nav).queryByText(gone)).not.toBeInTheDocument();
    expect(screen.getByText('Looking to customize NovaOS?')).toBeInTheDocument();
    await user.click(within(nav).getByRole('button', { name: 'Open Nova Studio' }));
    expect(Object.values(useWindows.getState().windows).map((w) => w.appId)).toContain('studio');
  });

  it('searching Settings for something that moved points to Studio', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    await user.type(screen.getByLabelText('Search settings'), 'wallpaper');
    expect(screen.getByText('“wallpaper” is in Nova Studio')).toBeInTheDocument();
    expect(screen.queryByText('No settings match.')).not.toBeInTheDocument();
    await user.click(screen.getAllByRole('button', { name: 'Open Nova Studio' })[0]);
    expect(Object.values(useWindows.getState().windows).map((w) => w.appId)).toContain('studio');
  });
});

describe('pin buttons in the Start menu', () => {
  it('pin and unpin each app on the taskbar and on the desktop, separately', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Start'));
    const menu = screen.getByRole('menu', { name: 'Start menu' });
    expect(
      within(menu).getByRole('button', { name: 'Unpin Nova Notes from taskbar' }),
    ).toHaveAttribute('aria-pressed', 'true');

    await user.click(within(menu).getByRole('button', { name: 'Unpin Nova Notes from taskbar' }));
    expect(screen.queryByLabelText('Open Nova Notes')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Nova Notes shortcut')).toBeInTheDocument(); // still on the desktop

    await user.click(within(menu).getByRole('button', { name: 'Unpin Nova Notes from desktop' }));
    expect(screen.queryByLabelText('Nova Notes shortcut')).not.toBeInTheDocument();

    await user.click(within(menu).getByRole('button', { name: 'Pin Nova Notes to taskbar' }));
    expect(screen.getByLabelText('Open Nova Notes')).toBeInTheDocument();
    await user.click(within(menu).getByRole('button', { name: 'Pin Nova Notes to desktop' }));
    expect(screen.getByLabelText('Nova Notes shortcut')).toBeInTheDocument();
  });

  it('an app that is not pinned anywhere still opens from the Start menu, and can be pinned', async () => {
    const user = userEvent.setup();
    set('desktop', { taskbarPinned: [], desktopPinned: [] });
    render(<Desktop />);
    expect(taskbar().querySelectorAll('[aria-label^="Open Nova"]')).toHaveLength(0);
    await user.click(screen.getByLabelText('Start'));
    const menu = screen.getByRole('menu', { name: 'Start menu' });
    await user.click(within(menu).getByRole('button', { name: 'Pin Nova Calculator to taskbar' }));
    expect(taskbarLaunchers()).toEqual(['Open Nova Calculator']);
    await user.click(within(menu).getByRole('menuitem', { name: 'Nova Calculator' }));
    expect(Object.values(useWindows.getState().windows).map((w) => w.appId)).toEqual([
      'calculator',
    ]);
  });

  it('hidden helper apps (Editor, Viewer) are never offered for pinning', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Start'));
    expect(
      screen.queryByRole('button', { name: /Nova Editor|Nova Viewer/ }),
    ).not.toBeInTheDocument();
  });
});

describe('right-click menus', () => {
  it('on a taskbar app: pin and unpin, and the menu closes', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    fireEvent.contextMenu(screen.getByLabelText('Open Nova Calculator'));
    const menu = screen.getByRole('menu', { name: 'Context menu' });
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((i) => i.textContent),
    ).toEqual(
      ['Open', 'Unpin from taskbar', 'Pin to desktop'].map((t) =>
        t === 'Pin to desktop' ? 'Unpin from desktop' : t,
      ),
    );
    await user.click(within(menu).getByRole('menuitem', { name: 'Unpin from taskbar' }));
    expect(screen.queryByRole('menu', { name: 'Context menu' })).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Open Nova Calculator')).not.toBeInTheDocument();
    expect(data().desktop.taskbarPinned).not.toContain('calculator');
  });

  it('on a desktop icon: Open, unpin from desktop, pin to taskbar', async () => {
    const user = userEvent.setup();
    set('desktop', { taskbarPinned: [] });
    render(<Desktop />);
    fireEvent.contextMenu(screen.getByLabelText('Nova Notes shortcut'));
    await user.click(screen.getByRole('menuitem', { name: 'Pin to taskbar' }));
    expect(taskbarLaunchers()).toEqual(['Open Nova Notes']);
    fireEvent.contextMenu(screen.getByLabelText('Nova Notes shortcut'));
    await user.click(screen.getByRole('menuitem', { name: 'Unpin from desktop' }));
    expect(screen.queryByLabelText('Nova Notes shortcut')).not.toBeInTheDocument();
    fireEvent.contextMenu(screen.getByLabelText('Open Nova Notes'));
    await user.click(screen.getByRole('menuitem', { name: 'Open' }));
    expect(Object.values(useWindows.getState().windows).map((w) => w.appId)).toEqual(['notes']);
  });

  it('on a running window button: lets you pin an unpinned app', async () => {
    const user = userEvent.setup();
    set('desktop', { taskbarPinned: [] });
    render(<Desktop />);
    act(() => void launchApp('calculator'));
    fireEvent.contextMenu(screen.getByTestId('task-calculator'));
    await user.click(screen.getByRole('menuitem', { name: 'Pin to taskbar' }));
    expect(data().desktop.taskbarPinned).toEqual(['calculator']);
  });

  it('closes on Escape and on an outside click', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    fireEvent.contextMenu(screen.getByLabelText('Open Nova Calculator'));
    expect(screen.getByRole('menu', { name: 'Context menu' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu', { name: 'Context menu' })).not.toBeInTheDocument();
    fireEvent.contextMenu(screen.getByLabelText('Open Nova Calculator'));
    fireEvent.pointerDown(screen.getByTestId('desktop-layer'));
    expect(screen.queryByRole('menu', { name: 'Context menu' })).not.toBeInTheDocument();
  });

  it("the browser's own menu is suppressed on the desktop but kept in text fields", () => {
    render(<Desktop />);
    const onDesktop = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    screen.getByTestId('desktop-layer').dispatchEvent(onDesktop);
    expect(onDesktop.defaultPrevented).toBe(true);
    const input = document.createElement('input');
    screen.getByTestId('desktop').appendChild(input);
    const inField = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    input.dispatchEvent(inField);
    expect(inField.defaultPrevented).toBe(false);
  });
});

describe('taskbar customization', () => {
  it('shows pinned apps in the order you chose', () => {
    set('desktop', { taskbarPinned: ['calculator', 'files', 'studio'] });
    render(<Desktop />);
    expect(taskbarLaunchers()).toEqual([
      'Open Nova Calculator',
      'Open Nova Files',
      'Open Nova Studio',
    ]);
  });

  it('skips pinned apps that no longer exist', () => {
    set('desktop', { taskbarPinned: ['ghost-app', 'files'] });
    render(<Desktop />);
    expect(taskbarLaunchers()).toEqual(['Open Nova Files']);
  });

  it('floating style leaves a gap and the desktop area makes room for it', () => {
    set('desktop', { taskbarStyle: 'floating' });
    render(<Desktop />);
    expect(taskbar()).toHaveAttribute('data-style', 'floating');
    expect(taskbar().style.margin).toBe('8px');
    expect(screen.getByTestId('desktop-layer').style.bottom).toBe('64px'); // 48 + 16
    set('desktop', { taskbarPosition: 'top' });
    return Promise.resolve().then(() => {
      expect(screen.getByTestId('desktop-layer').style.top).toBe('64px');
      expect(screen.getByTestId('desktop-layer').style.bottom).toBe('0px');
    });
  });

  it('docked taskbars reserve exactly their height', () => {
    render(<Desktop />);
    expect(screen.getByTestId('desktop-layer').style.bottom).toBe('48px');
    act(() => set('desktop', { taskbarSize: 'large' }));
    expect(screen.getByTestId('desktop-layer').style.bottom).toBe('58px');
  });

  it('transparency reaches the taskbar background', () => {
    set('desktop', { taskbarOpacity: 0.25 });
    render(<Desktop />);
    expect(screen.getByTestId('desktop').style.getPropertyValue('--taskbar-bg')).toBe(
      'rgb(0 0 0 / 0.25)',
    );
  });

  it('window titles can be hidden, leaving icon-only buttons that keep a tooltip', () => {
    render(<Desktop />);
    act(() => void launchApp('notes'));
    expect(within(screen.getByTestId('task-notes')).getByText('Nova Notes')).toBeInTheDocument();
    act(() => set('desktop', { taskbarLabels: false }));
    expect(
      within(screen.getByTestId('task-notes')).queryByText('Nova Notes'),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId('task-notes')).toHaveAttribute('title', 'Nova Notes');
  });

  it('the Start menu opens next to a taskbar on top and clears a floating bar', async () => {
    const user = userEvent.setup();
    set('desktop', { taskbarPosition: 'top', taskbarStyle: 'floating' });
    render(<Desktop />);
    await user.click(screen.getByLabelText('Start'));
    const menu = screen.getByRole('menu', { name: 'Start menu' });
    expect(menu.style.top).toBe('72px'); // 48 + 16 + 8
    expect(menu.style.bottom).toBe('');
  });
});

describe('desktop customization', () => {
  it('icon names and spacing', () => {
    render(<Desktop />);
    expect(
      within(screen.getByLabelText('Nova Notes shortcut')).getByText('Notes'),
    ).toBeInTheDocument();
    act(() => set('desktop', { iconLabels: false, iconSpacing: 'roomy' }));
    expect(
      within(screen.getByLabelText('Nova Notes shortcut')).queryByText('Notes'),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText('Nova Notes shortcut')).toHaveAttribute('title', 'Notes'); // still discoverable
    expect(screen.getByTestId('desktop').style.getPropertyValue('--icon-gap')).toBe('18px');
    expect(screen.getByTestId('desktop-icons').style.gap).toBe('var(--icon-gap)');
  });

  it('the Recycle Bin keeps its place whatever else is customized', () => {
    set('desktop', { desktopPinned: [], showDesktopIcons: false, iconLabels: false });
    render(<Desktop />);
    expect(screen.getByLabelText('Recycle Bin')).toBeInTheDocument();
  });

  it('desktop icons follow the order you chose', () => {
    set('desktop', { desktopPinned: ['calculator', 'studio', 'files'] });
    render(<Desktop />);
    const labels = within(screen.getByTestId('desktop-icons'))
      .getAllByRole('button')
      .map((b) => b.getAttribute('aria-label'));
    expect(labels).toEqual([
      'Recycle Bin',
      'Nova Calculator shortcut',
      'Nova Studio shortcut',
      'Nova Files shortcut',
    ]);
  });
});

describe('window customization', () => {
  it('windows use the shadow, title bar and border settings', () => {
    set('windows', { shadow: 'strong', titleBar: 'large', activeBorder: 'subtle' });
    render(<Desktop />);
    act(() => void launchApp('notes'));
    const win = screen.getByTestId('window-notes');
    expect(win.style.boxShadow).toBe('var(--shadow)');
    expect(win.style.borderColor).toBe('var(--active-border)');
    expect(win.querySelector('header')!.style.height).toBe('var(--titlebar)');
    const root = screen.getByTestId('desktop').style;
    expect(root.getPropertyValue('--titlebar')).toBe('44px');
    expect(root.getPropertyValue('--active-border')).toBe('rgb(255 255 255 / 0.28)');
    expect(root.getPropertyValue('--shadow')).toContain('rgb(0 0 0 / 0.65)');
  });

  it('a maximized window drops its shadow', () => {
    render(<Desktop />);
    act(() => void launchApp('notes'));
    const id = Object.keys(useWindows.getState().windows)[0];
    act(() => useWindows.getState().toggleMaximize(id));
    expect(screen.getByTestId('window-notes').style.boxShadow).toBe('none');
  });

  it('only the focused window gets the active border', () => {
    render(<Desktop />);
    act(() => {
      launchApp('notes');
      launchApp('calculator');
    });
    expect(screen.getByTestId('window-calculator').style.borderColor).toBe('var(--active-border)');
    expect(screen.getByTestId('window-notes').style.borderColor).toBe('var(--border)');
  });
});

describe('cursor and ripples', () => {
  it('a custom pointer reaches the whole desktop', () => {
    set('cursor', { style: 'classic', color: '#ff0000' });
    render(<Desktop />);
    const el = screen.getByTestId('desktop');
    expect(el).toHaveAttribute('data-cursor', 'custom');
    expect(el.style.cursor).toContain('data:image/svg+xml');
    expect(decodeURIComponent(el.style.cursor)).toContain('#ff0000');
  });

  it('the system pointer is left alone by default', () => {
    render(<Desktop />);
    expect(screen.getByTestId('desktop')).toHaveAttribute('data-cursor', 'system');
    expect(screen.getByTestId('desktop').style.cursor).toBe('');
  });

  it('ripples appear at clicks only when enabled, and never with reduced motion', () => {
    render(<Desktop />);
    fireEvent.pointerDown(document.body, { clientX: 100, clientY: 100 });
    expect(screen.queryByTestId('click-ripple')).not.toBeInTheDocument();

    act(() => set('cursor', { ripple: true, rippleColor: '#00ff00' }));
    fireEvent.pointerDown(document.body, { clientX: 100, clientY: 120 });
    const ripple = screen.getByTestId('click-ripple');
    expect(ripple.style.left).toBe('78px'); // centered on the click
    expect(ripple.style.top).toBe('98px');
    expect(screen.getByTestId('desktop').style.getPropertyValue('--ripple')).toBe('#00ff00');

    act(() => set('accessibility', { reduceMotion: true }));
    expect(screen.queryByTestId('click-ripple')).not.toBeInTheDocument();
    fireEvent.pointerDown(document.body, { clientX: 5, clientY: 5 });
    expect(screen.queryByTestId('click-ripple')).not.toBeInTheDocument();
  });
});

describe('sounds', () => {
  it('stay silent for windows unless enabled', () => {
    render(<Desktop />);
    act(() => void launchApp('notes'));
    expect(playMock).not.toHaveBeenCalledWith('open', expect.anything());
  });

  it('play for opening, minimizing, restoring and closing windows once enabled', () => {
    set('sounds', { enabled: true, pack: 'chime' });
    render(<Desktop />);
    act(() => void launchApp('notes'));
    expect(playMock).toHaveBeenCalledWith(
      'open',
      expect.objectContaining({ enabled: true, pack: 'chime' }),
    );
    const id = Object.keys(useWindows.getState().windows)[0];
    act(() => useWindows.getState().minimize(id));
    expect(playMock).toHaveBeenCalledWith('minimize', expect.anything());
    act(() => useWindows.getState().restore(id));
    expect(playMock).toHaveBeenCalledWith('restore', expect.anything());
    act(() => useWindows.getState().close(id));
    expect(playMock).toHaveBeenCalledWith('close', expect.anything());
  });

  it('button presses ask for a click sound (which only plays if click sounds are on)', () => {
    render(<Desktop />);
    fireEvent.pointerDown(screen.getByLabelText('Start'));
    expect(playMock).toHaveBeenCalledWith('click', expect.anything());
  });
});

describe('typography and scale', () => {
  it('the chosen font and interface size apply to the whole desktop', () => {
    set('interface', { font: 'serif' });
    set('accessibility', { uiScale: 1.2 });
    render(<Desktop />);
    expect(screen.getByTestId('desktop').style.fontFamily).toContain('Georgia');
    expect(document.documentElement.style.fontSize).toBe('120%');
  });
});
