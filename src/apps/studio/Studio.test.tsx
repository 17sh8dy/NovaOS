import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const state = { tauri: false };
vi.mock('../../native/bridge', () => ({
  isTauri: () => state.tauri,
  native: vi.fn(),
}));
vi.mock('../files/filesApi', () => ({
  errorMessage: (e: unknown) =>
    typeof e === 'string' ? e : e instanceof Error ? e.message : 'error',
  filesApi: {
    list: vi.fn(),
    trash: vi.fn(),
    createFile: vi.fn(),
    writeText: vi.fn(),
    readText: vi.fn(),
    readBinary: vi.fn(),
  },
}));
vi.mock('../../settings/sounds', async (orig) => ({
  ...(await orig<typeof import('../../settings/sounds')>()),
  playSound: vi.fn(),
}));

import { Studio } from './Studio';
import { CONTROLS } from './controls';
import { STUDIOS } from './studios';
import { useStudioHistory } from './history';
import { registerBuiltinApps } from '../../shell/builtinApps';
import { native } from '../../native/bridge';
import { filesApi } from '../files/filesApi';
import { playSound } from '../../settings/sounds';
import { DEFAULTS } from '../../settings/schema';
import { useSettings } from '../../settings/store';

const api = vi.mocked(filesApi);
const data = () => useSettings.getState().data;

beforeAll(() => registerBuiltinApps());

beforeEach(() => {
  vi.clearAllMocks();
  state.tauri = false;
  localStorage.clear();
  useSettings.setState({ data: DEFAULTS, loaded: true });
  useStudioHistory.getState().clear();
  api.list.mockResolvedValue({ entries: [], truncated: false });
  api.trash.mockResolvedValue(undefined);
  api.createFile.mockResolvedValue(undefined);
  api.writeText.mockResolvedValue(undefined);
  api.readBinary.mockResolvedValue(new ArrayBuffer(8));
  URL.createObjectURL = vi.fn(() => 'blob:x');
  URL.revokeObjectURL = vi.fn();
});

const nav = () => screen.getByRole('navigation', { name: 'Studios' });
const go = async (user: ReturnType<typeof userEvent.setup>, title: string) =>
  user.click(within(nav()).getByRole('button', { name: title }));
const mini = () => screen.getByTestId('mini-desktop');

describe('Nova Studio shell', () => {
  it('opens on Home with a tile for every studio, and a nav entry for each plus Home and Library', () => {
    render(<Studio />);
    expect(screen.getByText('Welcome to Nova Studio')).toBeInTheDocument();
    for (const s of STUDIOS) {
      expect(screen.getByRole('button', { name: `Open ${s.title} Studio` })).toBeInTheDocument();
      expect(within(nav()).getByRole('button', { name: s.title })).toBeInTheDocument();
    }
    expect(within(nav()).getByRole('button', { name: 'Home' })).toBeInTheDocument();
    expect(within(nav()).getByRole('button', { name: 'Library' })).toBeInTheDocument();
    expect(STUDIOS.length).toBeGreaterThanOrEqual(8);
  });

  it('every studio page shows every one of its controls', async () => {
    const user = userEvent.setup();
    const { container } = render(<Studio />);
    for (const s of STUDIOS) {
      await go(user, s.title);
      expect(screen.getByRole('region', { name: `${s.title} Studio` })).toBeInTheDocument();
      for (const c of CONTROLS.filter((x) => x.studio === s.id)) {
        expect(
          container.querySelector(`[data-control="${c.id}"]`),
          `${s.id}: ${c.id}`,
        ).not.toBeNull();
      }
    }
  });

  it('Home tiles open their studio', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await user.click(screen.getByRole('button', { name: 'Open Cursor Studio' }));
    expect(screen.getByRole('region', { name: 'Cursor Studio' })).toBeInTheDocument();
  });
});

describe('instant apply, undo and redo', () => {
  it('changes apply at once, and Undo / Redo walk through them', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Themes');
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
    await user.click(screen.getByLabelText('accent teal'));
    expect(data().appearance.accent).toBe('teal');
    expect(screen.getByRole('button', { name: 'Undo' })).toBeEnabled();
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().appearance.accent).toBe('violet');
    expect(screen.getByRole('button', { name: 'Redo' })).toBeEnabled();
    await user.click(screen.getByRole('button', { name: 'Redo' }));
    expect(data().appearance.accent).toBe('teal');
  });

  it('Ctrl+Z and Ctrl+Shift+Z work from the keyboard', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Themes');
    await user.click(screen.getByLabelText('accent rose'));
    await user.keyboard('{Control>}z{/Control}');
    expect(data().appearance.accent).toBe('violet');
    await user.keyboard('{Control>}{Shift>}z{/Shift}{/Control}');
    expect(data().appearance.accent).toBe('rose');
  });

  it('dragging a slider is one undo step', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Windows');
    const slider = screen.getByLabelText('Window opacity');
    fireEvent.change(slider, { target: { value: '0.7' } });
    fireEvent.change(slider, { target: { value: '0.6' } });
    fireEvent.change(slider, { target: { value: '0.55' } });
    expect(data().appearance.windowOpacity).toBe(0.55);
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().appearance.windowOpacity).toBe(0.8);
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
  });

  it('reset this studio restores its defaults and can be undone', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Cursor');
    await user.click(screen.getByRole('radio', { name: 'Dot' }));
    await user.click(screen.getByRole('switch', { name: 'Click ripples' }));
    expect(data().cursor).toMatchObject({ style: 'dot', ripple: true });
    await user.click(screen.getByRole('button', { name: 'Reset Cursor Studio' }));
    expect(data().cursor).toEqual(DEFAULTS.cursor);
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().cursor).toMatchObject({ style: 'dot', ripple: true });
  });

  it('reset all customization (from Home) keeps general settings and can be undone', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('general', { clockFormat: '24h' });
    useSettings.getState().set('appearance', { accent: 'rose' });
    render(<Studio />);
    await user.click(screen.getByRole('button', { name: 'Reset all customization' }));
    expect(data().appearance.accent).toBe('violet');
    expect(data().general.clockFormat).toBe('24h');
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().appearance.accent).toBe('rose');
  });
});

describe('draft mode (apply instantly off)', () => {
  it('changes wait in the preview until Apply, and Discard throws them away', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await user.click(screen.getByRole('switch', { name: 'Apply instantly' }));
    expect(data().studio.instantApply).toBe(false);
    await go(user, 'Themes');
    await user.click(screen.getByLabelText('accent rose'));
    expect(data().appearance.accent).toBe('violet'); // the real desktop is untouched
    expect(mini().style.getPropertyValue('--accent')).toBe('#f43f5e'); // but the preview shows it
    expect(screen.getByRole('status')).toHaveTextContent(/changes waiting/i);
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'Discard' }));
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mini().style.getPropertyValue('--accent')).toBe('#8b5cf6');

    await user.click(screen.getByLabelText('accent amber'));
    await user.click(screen.getByRole('button', { name: 'Apply' }));
    expect(data().appearance.accent).toBe('amber');
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    // applying is itself undoable once instant mode is back
    await user.click(screen.getByRole('switch', { name: 'Apply instantly' }));
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().appearance.accent).toBe('violet');
  });

  it('turning Instant back on applies waiting changes instead of losing them', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('studio', { instantApply: false });
    render(<Studio />);
    await go(user, 'Windows');
    await user.click(screen.getByRole('radio', { name: 'Strong' }));
    expect(data().windows.shadow).toBe('soft');
    await user.click(screen.getByRole('switch', { name: 'Apply instantly' }));
    expect(data().windows.shadow).toBe('strong');
    expect(data().studio.instantApply).toBe(true);
  });

  it("Studio's own layout settings still apply immediately in draft mode", async () => {
    const user = userEvent.setup();
    useSettings.getState().set('studio', { instantApply: false });
    render(<Studio />);
    await go(user, 'Interface');
    await user.click(screen.getByRole('radio', { name: 'Compact' }));
    expect(data().studio.sidebar).toBe('compact');
    expect(screen.queryByRole('status')).not.toBeInTheDocument(); // not a pending change
  });
});

describe('advanced mode', () => {
  it('adds exact number fields and hex fields', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Windows');
    expect(screen.queryByLabelText('Window opacity exact value')).not.toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Advanced mode' }));
    expect(data().studio.advanced).toBe(true);
    const exact = screen.getByLabelText('Window opacity exact value');
    await user.clear(exact);
    await user.type(exact, '65');
    expect(data().appearance.windowOpacity).toBe(0.8); // not applied while typing
    await user.keyboard('{Enter}');
    expect(data().appearance.windowOpacity).toBe(0.65);
    // out-of-range exact values are clamped, and the box shows what was accepted
    await user.clear(exact);
    await user.type(exact, '500');
    await user.tab();
    expect(data().appearance.windowOpacity).toBe(1);
    expect(exact).toHaveValue('100');
    // junk is refused and the box snaps back
    await user.clear(exact);
    await user.type(exact, 'abc');
    await user.tab();
    expect(data().appearance.windowOpacity).toBe(1);
    expect(exact).toHaveValue('100');

    await go(user, 'Themes');
    const hex = screen.getByLabelText('window color hex');
    await user.clear(hex);
    await user.type(hex, '#102030');
    expect(data().appearance.surfaceColor).toBe('#102030');
    await user.clear(hex);
    await user.type(hex, '#12');
    expect(data().appearance.surfaceColor).toBe('#102030'); // incomplete hex is ignored
  });
});

describe('favorites and search', () => {
  it('starring a setting puts it on Home, where it still works', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    expect(screen.getByText(/Tap the star next to any setting/)).toBeInTheDocument();
    await go(user, 'Cursor');
    await user.click(screen.getByRole('button', { name: 'Add Click ripples to favorites' }));
    expect(data().studio.favorites).toEqual(['cursor.ripple']);
    await go(user, 'Home');
    const ripple = screen.getByRole('switch', { name: 'Click ripples' });
    await user.click(ripple);
    expect(data().cursor.ripple).toBe(true);
    await user.click(screen.getByRole('button', { name: 'Remove Click ripples from favorites' }));
    expect(data().studio.favorites).toEqual([]);
    expect(screen.getByText(/Tap the star next to any setting/)).toBeInTheDocument();
  });

  it('global search finds settings in any studio and they work in place', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await user.type(screen.getByLabelText('Search all settings'), 'blur');
    expect(screen.getByText(/Results for “blur”/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Background blur'), { target: { value: '10' } });
    expect(data().appearance.blur).toBe(10);
    expect(screen.queryByLabelText('Taskbar opacity')).not.toBeInTheDocument(); // not a blur setting
  });

  it('search results can be edited, and link to their studio', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await user.type(screen.getByLabelText('Search all settings'), 'taskbar transparency');
    fireEvent.change(screen.getByLabelText('Taskbar opacity'), { target: { value: '0.9' } });
    expect(data().desktop.taskbarOpacity).toBe(0.9);
    await user.click(screen.getByText('Open Taskbar Studio →'));
    expect(screen.getByRole('region', { name: 'Taskbar Studio' })).toBeInTheDocument();
    expect(screen.getByLabelText('Search all settings')).toHaveValue('');
  });

  it('says so when nothing matches', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await user.type(screen.getByLabelText('Search all settings'), 'zzzzqq');
    expect(screen.getByText(/Nothing matches/)).toBeInTheDocument();
  });
});

describe('live preview', () => {
  it('shows exactly what the settings say', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    const bar = () => screen.getByTestId('mini-taskbar');
    expect(within(bar()).getByText('Nova Notes')).toBeInTheDocument();
    await go(user, 'Taskbar');
    await user.click(screen.getByRole('switch', { name: 'Show window titles on the taskbar' }));
    expect(within(bar()).queryByText('Nova Notes')).not.toBeInTheDocument();
    await user.click(screen.getByRole('radio', { name: 'Top' }));
    expect(bar().style.top).not.toBe('');
    await user.click(screen.getByRole('radio', { name: 'Floating' }));
    expect(bar().style.left).not.toBe('0px');
    expect(bar().style.left).not.toBe('');
    await go(user, 'Desktop');
    await user.click(screen.getByRole('switch', { name: 'Show icon names' }));
    expect(within(mini()).queryByText('Recycle Bin')).not.toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Selection rectangle' }));
  });

  it('a custom pointer shows up as the preview cursor', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Cursor');
    expect(mini().style.cursor).toBe('');
    await user.click(screen.getByRole('radio', { name: 'Crosshair' }));
    expect(mini().style.cursor).toContain('data:image/svg+xml');
    expect(mini().getAttribute('data-cursor')).toBe('custom');
  });

  it('can move to the side or be hidden, and the sidebar can go compact', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Interface');
    await user.click(screen.getByRole('radio', { name: 'Side' }));
    expect(data().studio.previewPosition).toBe('side');
    await user.click(screen.getByRole('switch', { name: 'Show live preview' }));
    expect(screen.queryByTestId('mini-desktop')).not.toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Show live preview' }));
    expect(screen.getByTestId('mini-desktop')).toBeInTheDocument();
    await user.click(screen.getByRole('radio', { name: 'Compact' }));
    expect(within(nav()).queryByText('Themes')).not.toBeInTheDocument(); // icons only
    expect(within(nav()).getByRole('button', { name: 'Themes' })).toBeInTheDocument(); // still labelled for screen readers
  });

  it('clicking the preview shows a ripple only when ripples are on', () => {
    useSettings.getState().set('cursor', { ripple: true });
    const { container } = render(<Studio />);
    fireEvent.pointerDown(mini(), { clientX: 10, clientY: 10 });
    expect(
      container.querySelectorAll('[data-testid="mini-desktop"] span[aria-hidden="true"]').length,
    ).toBeGreaterThan(0);
  });
});

describe('pinned apps', () => {
  it('pin, unpin and reorder apps on the taskbar and the desktop', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Taskbar');
    await user.click(screen.getByRole('button', { name: 'Unpin Nova Notes from the taskbar' }));
    expect(data().desktop.taskbarPinned).not.toContain('notes');
    expect(data().desktop.desktopPinned).toContain('notes'); // desktop is separate
    await user.click(screen.getByRole('button', { name: 'Pin Nova Notes to the taskbar' }));
    expect(data().desktop.taskbarPinned.at(-1)).toBe('notes');
    const before = [...data().desktop.taskbarPinned];
    await user.click(
      screen.getByRole('button', { name: 'Move Nova Notes earlier on the taskbar' }),
    );
    const after = data().desktop.taskbarPinned;
    expect(after.indexOf('notes')).toBe(before.indexOf('notes') - 1);

    await go(user, 'Desktop');
    await user.click(screen.getByRole('button', { name: 'Unpin Nova Files from the desktop' }));
    expect(data().desktop.desktopPinned).not.toContain('files');
    expect(data().desktop.taskbarPinned).toContain('files');
  });

  it('the first pinned app cannot move earlier and the last cannot move later', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Taskbar');
    const first = data().desktop.taskbarPinned[0];
    const last = data().desktop.taskbarPinned.at(-1);
    const title = (id?: string) =>
      id === 'files'
        ? 'Nova Files'
        : id === 'notes'
          ? 'Nova Notes'
          : id === 'calculator'
            ? 'Nova Calculator'
            : id === 'studio'
              ? 'Nova Studio'
              : 'Nova Settings';
    expect(
      screen.getByRole('button', { name: `Move ${title(first)} earlier on the taskbar` }),
    ).toBeDisabled();
    expect(
      screen.getByRole('button', { name: `Move ${title(last)} later on the taskbar` }),
    ).toBeDisabled();
  });
});

describe('sounds page', () => {
  it('preview buttons play sounds even while sounds are off', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Sounds');
    await user.click(screen.getByRole('button', { name: 'Play Window opens' }));
    const [event, settings] = vi.mocked(playSound).mock.calls[0];
    expect(event).toBe('open');
    expect(settings).toMatchObject({ enabled: true, clicks: true });
    expect(data().sounds.enabled).toBe(false); // previewing does not turn them on
  });

  it('pack, volume and switches save', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Sounds');
    await user.click(screen.getByRole('switch', { name: 'Play sounds' }));
    await user.click(screen.getByRole('radio', { name: 'Chime' }));
    fireEvent.change(screen.getByLabelText('Volume'), { target: { value: '0.25' } });
    expect(data().sounds).toMatchObject({ enabled: true, pack: 'chime', volume: 0.25 });
  });
});

describe('Library', () => {
  it('built-in looks apply to the settings (and can be undone)', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('general', { clockFormat: '24h' });
    render(<Studio />);
    await go(user, 'Library');
    await user.click(screen.getByRole('button', { name: 'Use Neon Dusk' }));
    expect(data().appearance.accent).toBe('rose');
    expect(data().cursor).toMatchObject({ style: 'dot', ripple: true });
    expect(data().general.clockFormat).toBe('24h'); // looks never touch general settings
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().appearance.accent).toBe('violet');
  });

  it('saves your current look, applies it later, and deletes it', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Library');
    await user.click(screen.getByText('Save current look'));
    expect(screen.getByRole('status')).toHaveTextContent(/name first/i);
    expect(data().studio.presets).toHaveLength(0);

    await go(user, 'Themes');
    await user.click(screen.getByLabelText('accent amber'));
    await go(user, 'Library');
    await user.type(screen.getByLabelText('Look name'), 'My Amber{Enter}');
    expect(data().studio.presets).toHaveLength(1);
    expect(data().studio.presets[0]).toMatchObject({ name: 'My Amber' });
    expect(data().studio.presets[0].data.appearance.accent).toBe('amber');

    await go(user, 'Themes');
    await user.click(screen.getByLabelText('accent teal'));
    await go(user, 'Library');
    await user.click(screen.getByRole('button', { name: 'Use My Amber' }));
    expect(data().appearance.accent).toBe('amber');

    await user.click(screen.getByRole('button', { name: 'Delete My Amber' }));
    expect(data().studio.presets).toHaveLength(0);
  });

  it('a look round-trips through copy and paste, and bad pastes are explained', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('cursor', { style: 'crosshair' });
    render(<Studio />);
    await go(user, 'Library');
    await user.click(screen.getByText('Copy'));
    const box = screen.getByLabelText('Look JSON') as HTMLTextAreaElement;
    const exported = box.value;
    expect(exported).toContain('"kind": "look"');
    expect(exported).not.toContain('"favorites"'); // never leaks personal settings
    expect(exported).not.toContain('"general"');

    useSettings.getState().set('cursor', { style: 'system' });
    await user.click(screen.getByText('Add pasted look'));
    expect(data().studio.presets).toHaveLength(1);
    await user.click(screen.getByRole('button', { name: /Use My look/ }));
    expect(data().cursor.style).toBe('crosshair');

    await user.clear(box);
    await user.click(box);
    await user.paste('{"app":"novaos","settings":{}}');
    await user.click(screen.getByText('Add pasted look'));
    expect(screen.getByRole('status')).toHaveTextContent(/not a NovaOS look/);
    expect(data().studio.presets).toHaveLength(1);
  });

  it('saves and loads look files through Nova Files (Documents only)', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    api.readText.mockResolvedValue(
      '{"app":"novaos","kind":"look","version":2,"name":"From File","look":{"appearance":{"accent":"green"}}}',
    );
    render(<Studio />);
    await go(user, 'Library');
    const file = screen.getByLabelText('Look file name');
    await user.clear(file);
    await user.type(file, '../../evil name');
    await user.click(screen.getByText('Save to Nova Files'));
    const [root, path, text] = api.writeText.mock.calls[0];
    expect(root).toBe('nova');
    expect(path).toBe('Documents/evil-name.novalook.json'); // sanitized: no folder tricks possible
    expect(JSON.parse(text).kind).toBe('look');
    await user.click(screen.getByText('Load from Nova Files'));
    await waitFor(() => expect(data().studio.presets.map((p) => p.name)).toContain('From File'));
    expect(api.readText).toHaveBeenCalledWith('nova', 'Documents/evil-name.novalook.json');
  });

  it('keeps at most 30 looks', async () => {
    const user = userEvent.setup();
    const many = Array.from({ length: 30 }, (_, i) => ({
      id: `p${i}`,
      name: `P${i}`,
      data: DEFAULTS as never,
    }));
    useSettings.getState().set('studio', { presets: many });
    render(<Studio />);
    await go(user, 'Library');
    await user.type(screen.getByLabelText('Look name'), 'One too many{Enter}');
    expect(data().studio.presets).toHaveLength(30);
    expect(screen.getByRole('status')).toHaveTextContent(/up to 30/);
  });
});

describe('wallpaper inside Studio', () => {
  it('adds a custom wallpaper through the native picker and selects it', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    vi.mocked(native).mockResolvedValue('Beach.png');
    api.list.mockResolvedValueOnce({ entries: [], truncated: false }).mockResolvedValue({
      entries: [{ name: 'Beach.png', is_dir: false, is_symlink: false, size: 5, modified: 1 }],
      truncated: false,
    });
    render(<Studio />);
    await go(user, 'Themes');
    await user.click(screen.getByText('Add wallpaper…'));
    expect(native).toHaveBeenCalledWith('wallpaper_add');
    await waitFor(() => expect(data().appearance.wallpaper).toBe('file:Beach.png'));
    expect(await screen.findByLabelText('wallpaper Beach.png')).toBeInTheDocument();
    // and it can be undone like any other change
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().appearance.wallpaper).toBe('aurora');
  });
});
