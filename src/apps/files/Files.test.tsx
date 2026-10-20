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
vi.mock('./importApi', () => ({
  importApi: { list: vi.fn().mockResolvedValue([]) },
  onImportProgress: vi.fn(),
}));

import { Files, useFileClipboard } from './Files';
import { useSettings } from '../../settings/store';
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
        ? [dir('Docs'), file('a.txt'), file('pic.png', 2048), file('run.exe'), file('report.pdf')]
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

  it('opens text in the editor, images in the viewer and the rest with the host', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.dblClick(await screen.findByText('a.txt'));
    expect(launchApp).toHaveBeenCalledWith('editor', { root: 'nova', path: 'a.txt' });
    await user.dblClick(screen.getByText('pic.png'));
    expect(launchApp).toHaveBeenCalledWith('viewer', { root: 'nova', path: 'pic.png' });
    await user.dblClick(screen.getByText('report.pdf'));
    expect(api.openExternal).toHaveBeenCalledWith('nova', 'report.pdf');
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

  it('shows linked items under a warning heading and flags linked folders', async () => {
    const user = userEvent.setup();
    api.roots.mockResolvedValue([
      { id: 'nova', label: 'Nova Files', kind: 'nova', path: 'x' },
      { id: 'link-1', label: 'Thesis', kind: 'link', path: 'C:\\Docs\\Thesis' },
      { id: 'link-2', label: 'notes.md', kind: 'linkfile', path: 'C:\\Docs\\notes.md' },
    ]);
    render(<Files />);
    await screen.findByText('a.txt');
    expect(screen.getByText('Linked')).toBeInTheDocument();
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
    await user.click(within(screen.getByLabelText('Locations')).getByText('Thesis'));
    await waitFor(() => expect(api.list).toHaveBeenLastCalledWith('link-1', ''));
    expect(await screen.findByRole('note')).toHaveTextContent(/original files.*change them/);
  });

  it('a linked single file opens directly instead of being browsed', async () => {
    const user = userEvent.setup();
    api.roots.mockResolvedValue([
      { id: 'nova', label: 'Nova Files', kind: 'nova', path: 'x' },
      { id: 'link-2', label: 'notes.md', kind: 'linkfile', path: 'C:\\Docs\\notes.md' },
    ]);
    render(<Files />);
    await screen.findByText('a.txt');
    await user.click(within(screen.getByLabelText('Locations')).getByText('notes.md'));
    expect(launchApp).toHaveBeenCalledWith('editor', { root: 'link-2', path: '' });
    expect(api.list).not.toHaveBeenCalledWith('link-2', '');
  });

  it('the Import button opens the import dialog', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await screen.findByText('a.txt');
    await user.click(screen.getByLabelText('Import files or folders'));
    expect(screen.getByRole('dialog', { name: 'Import files' })).toBeInTheDocument();
  });

  it('asks before deleting when "Confirm before deleting" is on, and Cancel keeps the file', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('files', { confirmDelete: true });
    render(<Files />);
    await user.click(await screen.findByText('a.txt'));
    await user.click(screen.getByLabelText('Delete'));
    expect(api.trash).not.toHaveBeenCalled();
    const dialog = screen.getByRole('alertdialog', { name: 'Confirm delete' });
    expect(dialog).toHaveTextContent('a.txt');
    await user.click(within(dialog).getByText('Cancel'));
    expect(api.trash).not.toHaveBeenCalled();
    await user.click(screen.getByLabelText('Delete'));
    await user.click(within(screen.getByRole('alertdialog')).getByText('Delete'));
    expect(api.trash).toHaveBeenCalledWith('nova', 'a.txt');
    useSettings.getState().set('files', { confirmDelete: false });
  });

  it('hides file sizes when "Show file sizes" is off', async () => {
    useSettings.getState().set('files', { showSizes: false });
    render(<Files />);
    await screen.findByText('pic.png');
    expect(screen.queryByText('2.0 KB')).not.toBeInTheDocument();
    useSettings.getState().set('files', { showSizes: true });
  });

  it('single-click mode: one click opens a folder and a file, in Nova Files', async () => {
    const user = userEvent.setup();
    useSettings.getState().set('general', { singleClickOpen: true });
    render(<Files />);
    await user.click(await screen.findByText('Docs'));
    expect(await screen.findByText('inner.md')).toBeInTheDocument();
    expect(api.list).toHaveBeenLastCalledWith('nova', 'Docs');
    await user.click(screen.getByText('inner.md'));
    expect(launchApp).toHaveBeenCalledWith('editor', { root: 'nova', path: 'Docs/inner.md' });
    useSettings.getState().set('general', { singleClickOpen: false });
  });

  it('default mode: one click only selects; it takes a double click to open', async () => {
    const user = userEvent.setup();
    render(<Files />);
    await user.click(await screen.findByText('Docs'));
    expect(api.list).toHaveBeenCalledTimes(1); // still just the first listing
    expect(screen.getByText('Docs').closest('button')).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByText('a.txt'));
    expect(launchApp).not.toHaveBeenCalled();
  });
});
