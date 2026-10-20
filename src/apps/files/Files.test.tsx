import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('./filesApi', () => ({
  errorMessage: (e: unknown) => (typeof e === 'string' ? e : 'error'),
  filesApi: {
    roots: vi.fn(),
    list: vi.fn(),
    mkdir: vi.fn(),
    createFile: vi.fn(),
    rename: vi.fn(),
    copy: vi.fn(),
    move: vi.fn(),
    trash: vi.fn(),
    openExternal: vi.fn(),
  },
}));
vi.mock('../../kernel/launcher', () => ({ launchApp: vi.fn() }));

import { Files, useFileClipboard } from './Files';
import { filesApi } from './filesApi';
import { launchApp } from '../../kernel/launcher';

const api = vi.mocked(filesApi);
const dir = (name: string) => ({ name, is_dir: true, is_symlink: false, size: 0, modified: 1 });
const file = (name: string, size = 10) => ({
  name,
  is_dir: false,
  is_symlink: false,
  size,
  modified: 1,
});

beforeEach(() => {
  vi.clearAllMocks();
  useFileClipboard.getState().clear();
  api.roots.mockResolvedValue([
    { id: 'nova', label: 'Nova Files', kind: 'nova', path: 'x' },
    { id: 'host-documents', label: 'Documents', kind: 'host', path: 'y' },
  ]);
  api.list.mockImplementation(async (_root, path) => ({
    entries:
      path === ''
        ? [dir('Docs'), file('a.txt'), file('pic.png', 2048), file('run.exe')]
        : [file('inner.md')],
    truncated: false,
  }));
  for (const fn of [api.mkdir, api.createFile, api.trash, api.openExternal])
    fn.mockResolvedValue(undefined);
  api.rename.mockResolvedValue('renamed');
  api.copy.mockResolvedValue('copied');
  api.move.mockResolvedValue('moved');
});

describe('Nova Files', () => {
  it('shows locations and the root listing, folders first as returned', async () => {
    render(<Files />);
    expect(await screen.findByText('a.txt')).toBeInTheDocument();
    expect(api.list).toHaveBeenCalledWith('nova', '');
    expect(screen.getByText('Documents', { selector: 'nav span' })).toBeInTheDocument();
    expect(screen.getByText('2.0 KB')).toBeInTheDocument();
  });

  it('opens folders, navigates back and up', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.dblClick(await screen.findByText('Docs'));
    expect(await screen.findByText('inner.md')).toBeInTheDocument();
    expect(api.list).toHaveBeenLastCalledWith('nova', 'Docs');
    await user.click(screen.getByLabelText('Up'));
    expect(await screen.findByText('a.txt')).toBeInTheDocument();
    await user.click(screen.getByLabelText('Back'));
    expect(await screen.findByText('inner.md')).toBeInTheDocument();
  });

  it('switches locations from the sidebar', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await screen.findByText('a.txt');
    await user.click(within(screen.getByLabelText('Locations')).getByText('Documents'));
    await waitFor(() => expect(api.list).toHaveBeenLastCalledWith('host-documents', ''));
  });

  it('opens text files in the editor and other files with the host', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.dblClick(await screen.findByText('a.txt'));
    expect(launchApp).toHaveBeenCalledWith('editor', { root: 'nova', path: 'a.txt' });
    await user.dblClick(screen.getByText('pic.png'));
    expect(api.openExternal).toHaveBeenCalledWith('nova', 'pic.png');
  });

  it('shows backend refusals instead of swallowing them', async () => {
    const user = userEvent.setup();
    api.openExternal.mockRejectedValue('.exe files can run code, so NovaOS will not launch them');
    render(<Files />);
    await user.dblClick(await screen.findByText('run.exe'));
    expect(await screen.findByRole('alert')).toHaveTextContent('can run code');
  });

  it('creates a folder with a joined path and reloads', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await screen.findByText('a.txt');
    await user.click(screen.getByLabelText('New folder'));
    await user.type(screen.getByLabelText('New folder', { selector: 'input' }), 'Stuff{Enter}');
    expect(api.mkdir).toHaveBeenCalledWith('nova', 'Stuff');
    await waitFor(() => expect(api.list.mock.calls.length).toBeGreaterThan(1));
  });

  it('renames the selected item', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.click(await screen.findByText('a.txt'));
    await user.click(screen.getByLabelText('Rename'));
    const input = screen.getByLabelText('Rename to');
    await user.clear(input);
    await user.type(input, 'b.txt{Enter}');
    expect(api.rename).toHaveBeenCalledWith('nova', 'a.txt', 'b.txt');
  });

  it('deletes the selected item to the recycle bin', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.click(await screen.findByText('a.txt'));
    await user.click(screen.getByLabelText('Delete'));
    expect(api.trash).toHaveBeenCalledWith('nova', 'a.txt');
  });

  it('copies in one place and pastes in another', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.click(await screen.findByText('a.txt'));
    await user.click(screen.getByLabelText('Copy'));
    await user.click(within(screen.getByLabelText('Locations')).getByText('Documents'));
    await waitFor(() => expect(api.list).toHaveBeenLastCalledWith('host-documents', ''));
    await user.click(screen.getByLabelText('Paste'));
    expect(api.copy).toHaveBeenCalledWith('nova', 'a.txt', 'host-documents', '');
  });

  it('cut then paste moves and clears the clipboard', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.click(await screen.findByText('a.txt'));
    await user.click(screen.getByLabelText('Cut'));
    await user.dblClick(screen.getByText('Docs'));
    await screen.findByText('inner.md');
    await user.click(screen.getByLabelText('Paste'));
    expect(api.move).toHaveBeenCalledWith('nova', 'a.txt', 'nova', 'Docs');
    await waitFor(() => expect(useFileClipboard.getState().op).toBeNull());
  });

  it('keyboard: Delete trashes, F2 opens rename', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.click(await screen.findByText('a.txt'));
    screen.getByLabelText('Files').focus();
    await user.keyboard('{F2}');
    expect(screen.getByLabelText('Rename to')).toHaveValue('a.txt');
    await user.keyboard('{Escape}');
    screen.getByLabelText('Files').focus();
    await user.keyboard('{Delete}');
    expect(api.trash).toHaveBeenCalledWith('nova', 'a.txt');
  });
});
