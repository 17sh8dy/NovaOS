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

import { Studio } from './Studio';
import { CONTROLS, controlById } from './controls';
import { STUDIOS, resetStudio } from './studios';
import { useStudioHistory } from './history';
import { registerBuiltinApps } from '../../shell/builtinApps';
import { native } from '../../native/bridge';
import { filesApi } from '../files/filesApi';
import { BITS_CATALOG } from '../../settings/bits';
import { DEFAULTS, patchSection } from '../../settings/schema';
import { useSettings } from '../../settings/store';

const api = vi.mocked(filesApi);
const data = () => useSettings.getState().data;

beforeAll(() => registerBuiltinApps());

/** A minimal valid .cur so the cursor loader has something real to decode. */
const CUR = Uint8Array.from([
  0,
  0,
  2,
  0,
  1,
  0,
  32,
  32,
  0,
  0,
  4,
  0,
  5,
  0,
  40,
  0,
  0,
  0,
  22,
  0,
  0,
  0,
  ...new Array(40).fill(1),
]).buffer;

class FakeImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  set src(_v: string) {
    queueMicrotask(() => this.onload?.());
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  state.tauri = false;
  localStorage.clear();
  useSettings.setState({ data: DEFAULTS, loaded: true });
  useStudioHistory.getState().clear();
  api.list.mockResolvedValue({ entries: [], truncated: false });
  api.trash.mockResolvedValue(undefined);
  api.readBinary.mockResolvedValue(CUR);
  URL.createObjectURL = vi.fn(() => 'blob:cursor');
  URL.revokeObjectURL = vi.fn();
  vi.stubGlobal('Image', FakeImage);
});

const nav = () => screen.getByRole('navigation', { name: 'Studios' });
const go = async (user: ReturnType<typeof userEvent.setup>, title: string) =>
  user.click(within(nav()).getByRole('button', { name: title }));

describe('the React Bits studio', () => {
  it('is in the Studio sidebar and on Home', () => {
    render(<Studio />);
    expect(within(nav()).getByRole('button', { name: 'React Bits' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open React Bits Studio' })).toBeInTheDocument();
  });

  it('has a glass pill for every effect, and they all start off', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    expect(BITS_CATALOG.length).toBeGreaterThanOrEqual(12);
    for (const def of BITS_CATALOG) {
      const pill = screen.getByRole('switch', { name: def.title });
      expect(pill, def.title).toHaveAttribute('aria-checked', 'false');
      expect(pill).toHaveTextContent('Off');
    }
  });

  it('includes everything that fits: all the ones asked for except Splash Cursor', () => {
    const titles = BITS_CATALOG.map((b) => b.title);
    for (const t of [
      'Fold Text',
      'Echo Text',
      'Shiny Text',
      'Glow Cursor',
      'Elastic Mesh',
      'Swarm Cursor',
      'Click Spark',
      'Magnet',
      'Animated List',
      'Magic Bento',
      'Chroma Grid',
      'Dock',
      'Border Glow',
    ])
      expect(titles, t).toContain(t);
    expect(titles).not.toContain('Splash Cursor');
  });

  it('a pill turns its effect on and reveals its tweaks; turning it off hides them again', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    expect(screen.queryByLabelText('Click Spark sparks')).not.toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Click Spark' }));
    expect(data().bits.clickSpark.enabled).toBe(true);
    expect(screen.getByRole('switch', { name: 'Click Spark' })).toHaveTextContent('On');
    expect(screen.getByLabelText('Click Spark sparks')).toBeInTheDocument();
    expect(screen.getByLabelText('Click Spark spark length')).toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Click Spark' }));
    expect(data().bits.clickSpark.enabled).toBe(false);
    expect(screen.queryByLabelText('Click Spark sparks')).not.toBeInTheDocument();
  });

  it('sliders change that effect only, and keep the rest of its settings', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    await user.click(screen.getByRole('switch', { name: 'Glow Cursor' }));
    fireEvent.change(screen.getByLabelText('Glow Cursor size'), { target: { value: '400' } });
    expect(data().bits.glowCursor.size).toBe(400);
    expect(data().bits.glowCursor.enabled).toBe(true);
    expect(data().bits.glowCursor.intensity).toBe(0.35);
    expect(data().bits.clickSpark).toEqual(DEFAULTS.bits.clickSpark);
  });

  it('colors: Match accent, or any hex', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    await user.click(screen.getByRole('switch', { name: 'Swarm Cursor' }));
    fireEvent.change(screen.getByLabelText('Swarm Cursor dot color'), {
      target: { value: '#00ff88' },
    });
    expect(data().bits.swarmCursor.color).toBe('#00ff88');
    await user.click(screen.getByRole('button', { name: 'Swarm Cursor dot color match accent' }));
    expect(data().bits.swarmCursor.color).toBe('accent');
  });

  it('Magic Bento has a tilt switch; Animated List has styles', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    await user.click(screen.getByRole('switch', { name: 'Magic Bento' }));
    await user.click(screen.getByRole('switch', { name: 'Magic Bento: Tilt toward the pointer' }));
    expect(data().bits.magicBento.tilt).toBe(false);
    await user.click(screen.getByRole('switch', { name: 'Animated List' }));
    await user.click(screen.getByRole('radio', { name: 'Pop' }));
    expect(data().bits.animatedList.style).toBe('pop');
  });

  it('text effects choose where they apply', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    await user.click(screen.getByRole('switch', { name: 'Shiny Text' }));
    expect(data().bits.shinyText.targets).toEqual(['icons', 'titles']);
    await user.click(screen.getByRole('button', { name: 'Shiny Text on Taskbar clock' }));
    expect(data().bits.shinyText.targets).toEqual(['icons', 'titles', 'clock']);
    await user.click(screen.getByRole('button', { name: 'Shiny Text on Desktop icon names' }));
    expect(data().bits.shinyText.targets).toEqual(['titles', 'clock']);
  });

  it('Advanced mode adds exact number fields, and percentages are typed as percentages', async () => {
    const user = userEvent.setup();
    useSettings.setState({ data: patchSection(DEFAULTS, 'studio', { advanced: true }) });
    render(<Studio />);
    await go(user, 'React Bits');
    await user.click(screen.getByRole('switch', { name: 'Glow Cursor' }));
    const field = screen.getByLabelText('Glow Cursor brightness exact value');
    await user.clear(field);
    await user.type(field, '60{Enter}');
    expect(data().bits.glowCursor.intensity).toBe(0.6);
  });

  it('Turn all off clears every effect but keeps the tweaks', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    expect(screen.getByRole('button', { name: 'Turn all effects off' })).toBeDisabled();
    await user.click(screen.getByRole('switch', { name: 'Magnet' }));
    await user.click(screen.getByRole('switch', { name: 'Dock' }));
    expect(screen.getByRole('status')).toHaveTextContent('2 on');
    fireEvent.change(screen.getByLabelText('Dock growth'), { target: { value: '1.8' } });
    await user.click(screen.getByRole('button', { name: 'Turn all effects off' }));
    expect(data().bits.magnet.enabled).toBe(false);
    expect(data().bits.dock.enabled).toBe(false);
    expect(data().bits.dock.magnify).toBe(1.8);
    expect(screen.getByRole('status')).toHaveTextContent('None on');
  });

  it('says so when Reduce motion is pausing the effects', async () => {
    const user = userEvent.setup();
    let d = patchSection(DEFAULTS, 'accessibility', { reduceMotion: true });
    d = patchSection(d, 'bits', { magnet: { ...d.bits.magnet, enabled: true } });
    useSettings.setState({ data: d });
    render(<Studio />);
    await go(user, 'React Bits');
    expect(
      screen.getByText(/Reduce motion is on, so moving effects are paused/),
    ).toBeInTheDocument();
  });

  it('every change can be undone', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'React Bits');
    await user.click(screen.getByRole('switch', { name: 'Elastic Mesh' }));
    expect(data().bits.elasticMesh.enabled).toBe(true);
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().bits.elasticMesh.enabled).toBe(false);
  });

  it('resetting the studio puts every effect and tweak back to the defaults', () => {
    const changed = patchSection(DEFAULTS, 'bits', {
      ...DEFAULTS.bits,
      dock: { enabled: true, magnify: 1.9, radius: 200 },
    });
    expect(resetStudio(changed, 'bits').bits).toEqual(DEFAULTS.bits);
  });

  it('is found by search and can be favorited', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await user.type(screen.getByLabelText('Search all settings'), 'shimmer');
    expect(await screen.findByRole('switch', { name: 'Shiny Text' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Add Shiny Text to favorites' }));
    expect(data().studio.favorites).toContain('bits.shiny-text');
  });

  it('every control id is valid for favorites and unique', () => {
    const ids = CONTROLS.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z0-9.-]{1,60}$/);
    expect(controlById('bits.click-spark')).toBeDefined();
  });

  it('React Bits is saved into a look, and the studio belongs to the Studio list', () => {
    expect(STUDIOS.some((s) => s.id === 'bits')).toBe(true);
  });
});

describe('custom cursor file picker', () => {
  const entry = (name: string) => ({
    name,
    is_dir: false,
    is_symlink: false,
    size: 60,
    modified: 1,
  });

  it('outside the desktop app it explains instead of offering a picker', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Cursor');
    expect(screen.getByText(/available in the NovaOS desktop app/)).toBeInTheDocument();
    expect(screen.queryByText('Add cursor…')).not.toBeInTheDocument();
  });

  it('adds a cursor through the native picker, selects it, and the choice can be undone', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    vi.mocked(native).mockResolvedValue('Arrow.cur');
    api.list.mockResolvedValueOnce({ entries: [], truncated: false }).mockResolvedValue({
      entries: [entry('Arrow.cur')],
      truncated: false,
    });
    render(<Studio />);
    await go(user, 'Cursor');
    await user.click(screen.getByText('Add cursor…'));
    expect(native).toHaveBeenCalledWith('cursor_add');
    await waitFor(() => expect(data().cursor.customFile).toBe('Arrow.cur'));
    expect(await screen.findByLabelText('cursor Arrow.cur')).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(data().cursor.customFile).toBe('');
  });

  it('a cancelled dialog changes nothing', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    vi.mocked(native).mockResolvedValue(null);
    render(<Studio />);
    await go(user, 'Cursor');
    await user.click(screen.getByText('Add cursor…'));
    expect(data().cursor.customFile).toBe('');
  });

  it('shows the reason when a file is refused', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    vi.mocked(native).mockRejectedValue('that file is not a real .cur cursor');
    render(<Studio />);
    await go(user, 'Cursor');
    await user.click(screen.getByText('Add cursor…'));
    expect(await screen.findByRole('alert')).toHaveTextContent('not a real .cur cursor');
    expect(data().cursor.customFile).toBe('');
  });

  it('lists the cursors you added, picks one, and None goes back to the built-in pointer', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    api.list.mockResolvedValue({
      entries: [entry('A.cur'), entry('B.png'), entry('notes.txt'), entry('evil.exe')],
      truncated: false,
    });
    render(<Studio />);
    await go(user, 'Cursor');
    expect(await screen.findByLabelText('cursor A.cur')).toBeInTheDocument();
    expect(screen.getByLabelText('cursor B.png')).toBeInTheDocument();
    expect(screen.queryByLabelText('cursor notes.txt')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('cursor evil.exe')).not.toBeInTheDocument();
    await user.click(screen.getByLabelText('cursor B.png'));
    expect(data().cursor.customFile).toBe('B.png');
    await user.click(screen.getByLabelText('no custom cursor'));
    expect(data().cursor.customFile).toBe('');
  });

  it('removing a cursor sends it to the Recycle Bin and stops using it', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    useSettings.setState({ data: patchSection(DEFAULTS, 'cursor', { customFile: 'A.cur' }) });
    api.list.mockResolvedValue({ entries: [entry('A.cur')], truncated: false });
    render(<Studio />);
    await go(user, 'Cursor');
    await user.click(await screen.findByLabelText('Remove cursor A.cur'));
    expect(api.trash).toHaveBeenCalledWith('nova', 'Pictures/Cursors/A.cur');
    await waitFor(() => expect(data().cursor.customFile).toBe(''));
  });

  it('warns, and offers a fix, when the pointer engine is set to Windows', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    useSettings.setState({
      data: patchSection(DEFAULTS, 'cursor', { customFile: 'A.cur', engine: 'system' }),
    });
    api.list.mockResolvedValue({ entries: [entry('A.cur')], truncated: false });
    render(<Studio />);
    await go(user, 'Cursor');
    expect(await screen.findByText(/can’t show custom cursor files/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Let NovaOS draw it' }));
    expect(data().cursor.engine).toBe('auto');
  });

  it('falls back with a message when the file cannot be read', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    useSettings.setState({ data: patchSection(DEFAULTS, 'cursor', { customFile: 'Bad.cur' }) });
    api.list.mockResolvedValue({ entries: [entry('Bad.cur')], truncated: false });
    api.readBinary.mockResolvedValue(Uint8Array.from([1, 2, 3, 4]).buffer);
    render(<Studio />);
    await go(user, 'Cursor');
    expect(await screen.findByText(/couldn’t read that cursor/)).toBeInTheDocument();
  });

  it('points to the three cursor sites as links, and warns about licenses', async () => {
    const user = userEvent.setup();
    state.tauri = true;
    render(<Studio />);
    await go(user, 'Cursor');
    expect(screen.getByRole('link', { name: /RW-Designer/ })).toHaveAttribute(
      'href',
      'https://www.rw-designer.com/cursor-library',
    );
    expect(screen.getByRole('link', { name: /Custom Cursor/ })).toHaveAttribute(
      'href',
      'https://custom-cursor.com/',
    );
    expect(screen.getByRole('link', { name: /Cursors-4U/ })).toHaveAttribute(
      'href',
      'https://www.cursors-4u.com/',
    );
    expect(screen.getByText(/check each one’s license/)).toBeInTheDocument();
  });

  it('hotspot and everywhere settings are real controls', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    await go(user, 'Cursor');
    await user.click(screen.getByRole('radio', { name: 'Center' }));
    expect(data().cursor.customHotspot).toBe('center');
    await user.click(screen.getByRole('switch', { name: 'Use custom cursor everywhere' }));
    expect(data().cursor.customEverywhere).toBe(true);
  });
});
