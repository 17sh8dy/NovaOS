import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, within, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const state = { tauri: true };
vi.mock('../../native/bridge', () => ({
  isTauri: () => state.tauri,
  native: vi.fn(),
}));
vi.mock('../files/filesApi', () => ({
  errorMessage: (e: unknown) => (typeof e === 'string' ? e : 'error'),
  filesApi: { list: vi.fn(), trash: vi.fn(), readBinary: vi.fn() },
}));

import { Appearance } from './sections/Appearance';
import { native } from '../../native/bridge';
import { filesApi } from '../files/filesApi';
import { DEFAULTS } from '../../settings/schema';
import { useSettings } from '../../settings/store';

const nativeMock = vi.mocked(native);
const api = vi.mocked(filesApi);
const data = () => useSettings.getState().data;
const entry = (name: string, is_dir = false) => ({
  name,
  is_dir,
  is_symlink: false,
  size: 5,
  modified: 1,
});

beforeEach(() => {
  vi.clearAllMocks();
  state.tauri = true;
  localStorage.clear();
  useSettings.setState({ data: DEFAULTS, loaded: false });
  api.list.mockResolvedValue({ entries: [], truncated: false });
  api.trash.mockResolvedValue(undefined);
  api.readBinary.mockResolvedValue(new ArrayBuffer(8));
  URL.createObjectURL = vi.fn(() => 'blob:thumb');
  URL.revokeObjectURL = vi.fn();
});

describe('custom wallpapers in Appearance', () => {
  it('lists only valid image files from the Wallpapers folder', async () => {
    api.list.mockResolvedValue({
      entries: [
        entry('Sunset.png'),
        entry('notes.txt'),
        entry('sub', true),
        entry('evil.exe'),
        entry('b.JPG'),
      ],
      truncated: false,
    });
    render(<Appearance />);
    expect(await screen.findByLabelText('wallpaper Sunset.png')).toBeInTheDocument();
    expect(screen.getByLabelText('wallpaper b.JPG')).toBeInTheDocument();
    expect(screen.queryByLabelText('wallpaper notes.txt')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('wallpaper evil.exe')).not.toBeInTheDocument();
    expect(api.list).toHaveBeenCalledWith('nova', 'Pictures/Wallpapers');
  });

  it('Add wallpaper opens the native picker, then selects the new file', async () => {
    const user = userEvent.setup();
    nativeMock.mockResolvedValue('Mountain.jpg');
    api.list.mockResolvedValueOnce({ entries: [], truncated: false }).mockResolvedValue({
      entries: [entry('Mountain.jpg')],
      truncated: false,
    });
    render(<Appearance />);
    await user.click(screen.getByText('Add wallpaper…'));
    expect(nativeMock).toHaveBeenCalledWith('wallpaper_add');
    await waitFor(() => expect(data().appearance.wallpaper).toBe('file:Mountain.jpg'));
    expect(await screen.findByLabelText('wallpaper Mountain.jpg')).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('cancelling the picker changes nothing', async () => {
    const user = userEvent.setup();
    nativeMock.mockResolvedValue(null);
    render(<Appearance />);
    await user.click(screen.getByText('Add wallpaper…'));
    expect(data().appearance.wallpaper).toBe('aurora');
  });

  it('shows why an image was refused', async () => {
    const user = userEvent.setup();
    nativeMock.mockRejectedValue('that file is not a real .png image');
    render(<Appearance />);
    await user.click(screen.getByText('Add wallpaper…'));
    expect(await screen.findByRole('alert')).toHaveTextContent('not a real .png image');
    expect(data().appearance.wallpaper).toBe('aurora');
  });

  it('removing sends the file to the Recycle Bin and resets the wallpaper if it was in use', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('appearance', { wallpaper: 'file:Sunset.png' });
    api.list
      .mockResolvedValueOnce({ entries: [entry('Sunset.png')], truncated: false })
      .mockResolvedValue({
        entries: [],
        truncated: false,
      });
    render(<Appearance />);
    await screen.findByLabelText('wallpaper Sunset.png');
    await user.click(screen.getByLabelText('Remove wallpaper Sunset.png'));
    expect(api.trash).toHaveBeenCalledWith('nova', 'Pictures/Wallpapers/Sunset.png');
    await waitFor(() => expect(data().appearance.wallpaper).toBe('aurora'));
  });

  it('removing a wallpaper that is not in use leaves the current one alone', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('appearance', { wallpaper: 'ocean' });
    api.list.mockResolvedValue({ entries: [entry('A.png')], truncated: false });
    render(<Appearance />);
    await screen.findByLabelText('wallpaper A.png');
    await user.click(screen.getByLabelText('Remove wallpaper A.png'));
    expect(data().appearance.wallpaper).toBe('ocean');
  });

  it('fit and dim save', async () => {
    const user = userEvent.setup();
    render(<Appearance />);
    await user.click(screen.getByRole('radio', { name: 'Tile' }));
    expect(data().appearance.wallpaperFit).toBe('tile');
    fireEvent.change(screen.getByLabelText('Dim wallpaper'), { target: { value: '0.3' } });
    expect(data().appearance.wallpaperDim).toBe(0.3);
  });

  it('is honest in the browser preview, where there is no native picker', () => {
    state.tauri = false;
    render(<Appearance />);
    expect(screen.queryByText('Add wallpaper…')).not.toBeInTheDocument();
    expect(screen.getByText(/available in the NovaOS desktop app/)).toBeInTheDocument();
    expect(api.list).not.toHaveBeenCalled();
  });

  it('presets still work', async () => {
    const user = userEvent.setup();
    render(<Appearance />);
    await user.click(screen.getByLabelText('wallpaper forest'));
    expect(data().appearance.wallpaper).toBe('forest');
  });
});

describe('selection rectangle settings', () => {
  it('toggle, swatches, custom color and strength all save', async () => {
    const user = userEvent.setup();
    render(<Appearance />);
    const group = screen.getByRole('switch', { name: 'Selection rectangle' });
    expect(group).toHaveAttribute('aria-checked', 'true');
    await user.click(group);
    expect(data().appearance.selectionEnabled).toBe(false);
    await user.click(screen.getByLabelText('selection color #f43f5e'));
    expect(data().appearance.selectionColor).toBe('#f43f5e');
    fireEvent.change(screen.getByLabelText('custom selection color'), {
      target: { value: '#00ffcc' },
    });
    expect(data().appearance.selectionColor).toBe('#00ffcc');
    fireEvent.change(screen.getByLabelText('Selection fill'), { target: { value: '0.4' } });
    expect(data().appearance.selectionOpacity).toBe(0.4);
    await user.click(screen.getByLabelText('selection color match accent'));
    expect(data().appearance.selectionColor).toBe('accent');
    expect(
      within(screen.getByLabelText('Selection preview')).queryByRole('button'),
    ).not.toBeInTheDocument();
  });
});
