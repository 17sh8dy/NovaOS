import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('./importApi', () => ({
  importApi: {
    pick: vi.fn(),
    scan: vi.fn(),
    start: vi.fn(),
    cancel: vi.fn(),
    discard: vi.fn(),
    status: vi.fn(),
    list: vi.fn(),
    unlink: vi.fn(),
  },
  onImportProgress: vi.fn(),
}));

import { ImportDialog } from './ImportDialog';
import { importApi, onImportProgress, type Progress, type ScanReport } from './importApi';

const api = vi.mocked(importApi);
const MB = 1024 * 1024;

let emit: (p: Progress) => void = () => undefined;

const scanReport = (over: Partial<ScanReport> = {}): ScanReport => ({
  files: 10,
  dirs: 2,
  bytes: 5 * MB,
  symlinks: 0,
  programs: 0,
  unreadable: [],
  unreadable_count: 0,
  free_bytes: 500 * MB,
  required_bytes: 69 * MB,
  enough_space: true,
  ...over,
});

const progress = (over: Partial<Progress> = {}): Progress => ({
  job: 'sel1',
  state: 'running',
  items_done: 0,
  items_total: 1,
  files_done: 0,
  files_total: 10,
  bytes_done: 0,
  bytes_total: 5 * MB,
  speed_bps: 0,
  current: '',
  error_count: 0,
  errors: [],
  message: null,
  ...over,
});

function setup() {
  const props = { onClose: vi.fn(), onChanged: vi.fn(), onShowImports: vi.fn() };
  render(<ImportDialog {...props} />);
  return { ...props, user: userEvent.setup() };
}

async function reachReview(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByText('Import folder…'));
  await screen.findByTestId('scan-summary');
}

beforeEach(() => {
  vi.clearAllMocks();
  api.list.mockResolvedValue([]);
  api.pick.mockResolvedValue({
    id: 'sel1',
    items: [{ name: 'proj', path: 'C:\\proj', is_dir: true }],
  });
  api.scan.mockResolvedValue(scanReport());
  api.start.mockResolvedValue(progress());
  api.cancel.mockResolvedValue(undefined);
  api.discard.mockResolvedValue(undefined);
  api.status.mockResolvedValue(null);
  api.unlink.mockResolvedValue(undefined);
  vi.mocked(onImportProgress).mockImplementation(async (cb) => {
    emit = cb;
    return () => undefined;
  });
});

describe('ImportDialog', () => {
  it('offers files and folder imports and stays put if the native dialog is cancelled', async () => {
    api.pick.mockResolvedValue(null);
    const { user } = setup();
    await user.click(screen.getByText('Import files…'));
    expect(api.pick).toHaveBeenCalledWith('files');
    expect(screen.getByText('Import folder…')).toBeInTheDocument();
    expect(api.scan).not.toHaveBeenCalled();
  });

  it('scans first and shows counts, size and free space before anything is copied', async () => {
    const { user } = setup();
    await reachReview(user);
    expect(api.pick).toHaveBeenCalledWith('folder');
    expect(api.scan).toHaveBeenCalledWith('sel1');
    expect(screen.getByTestId('scan-summary')).toHaveTextContent(
      '10 files in 2 folders, 5.0 MB total',
    );
    expect(screen.getByText(/500 MB free/)).toBeInTheDocument();
    expect(api.start).not.toHaveBeenCalled();
  });

  it('copies with live progress, speed, counts, then a summary', async () => {
    const { user, onChanged, onShowImports, onClose } = setup();
    await reachReview(user);
    await user.click(screen.getByText('Copy into NovaOS'));
    expect(api.start).toHaveBeenCalledWith('sel1', 'copy', false);
    await screen.findByTestId('import-progress');
    await waitFor(() => expect(onImportProgress).toHaveBeenCalled());

    act(() =>
      emit(
        progress({
          files_done: 5,
          bytes_done: 2 * MB,
          speed_bps: 1024 * 1024,
          current: 'sub/a.bin',
        }),
      ),
    );
    expect(screen.getByText(/Files: 5 \/ 10/)).toBeInTheDocument();
    expect(screen.getByText(/1\.0 MB\/s/)).toBeInTheDocument();
    expect(screen.getByText('sub/a.bin')).toBeInTheDocument();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '40');

    act(() => emit(progress({ state: 'done', items_done: 1, files_done: 10, bytes_done: 5 * MB })));
    expect(await screen.findByTestId('import-result')).toHaveTextContent(
      'Imported 10 files (5.0 MB). Your originals were not changed.',
    );
    expect(onChanged).toHaveBeenCalledTimes(1);
    await user.click(screen.getByText('Show in Nova Files'));
    expect(onShowImports).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it('events for some other job are ignored', async () => {
    const { user } = setup();
    await reachReview(user);
    await user.click(screen.getByText('Copy into NovaOS'));
    await screen.findByTestId('import-progress');
    await waitFor(() => expect(onImportProgress).toHaveBeenCalled());
    act(() => emit(progress({ job: 'other', state: 'done', files_done: 99 })));
    expect(screen.queryByTestId('import-result')).not.toBeInTheDocument();
  });

  it('cancel tells Rust, cannot be closed mid-run, and explains partial results', async () => {
    const { user } = setup();
    await reachReview(user);
    await user.click(screen.getByText('Copy into NovaOS'));
    await screen.findByTestId('import-progress');
    await waitFor(() => expect(onImportProgress).toHaveBeenCalled());
    expect(screen.getByLabelText('Close import')).toBeDisabled();
    await user.click(screen.getByText('Cancel import'));
    expect(api.cancel).toHaveBeenCalledWith('sel1');
    act(() => emit(progress({ state: 'cancelled', items_done: 0 })));
    expect(await screen.findByTestId('import-result')).toHaveTextContent(
      /cancelled.*already finished were kept.*originals were not changed/i,
    );
    expect(screen.queryByText('Show in Nova Files')).not.toBeInTheDocument();
  });

  it('lists skipped items and failures with the reason', async () => {
    const { user } = setup();
    await reachReview(user);
    await user.click(screen.getByText('Copy into NovaOS'));
    await screen.findByTestId('import-progress');
    await waitFor(() => expect(onImportProgress).toHaveBeenCalled());
    act(() =>
      emit(
        progress({
          state: 'failed',
          message: 'write failed for big.iso: the disk is full',
          error_count: 2,
          errors: ['locked.docx: sharing violation', 'x/link: link skipped'],
        }),
      ),
    );
    const result = await screen.findByTestId('import-result');
    expect(result).toHaveTextContent('the disk is full');
    expect(result).toHaveTextContent('2 items skipped');
    expect(result).toHaveTextContent('locked.docx: sharing violation');
  });

  it('blocks Copy when there is not enough disk space', async () => {
    api.scan.mockResolvedValue(
      scanReport({ enough_space: false, free_bytes: 10 * MB, required_bytes: 69 * MB }),
    );
    const { user } = setup();
    await reachReview(user);
    expect(screen.getByText(/Not enough disk space/)).toBeInTheDocument();
    expect(screen.getByText('Copy into NovaOS')).toBeDisabled();
  });

  it('warns about links, programs and unreadable items found by the scan', async () => {
    api.scan.mockResolvedValue(
      scanReport({
        symlinks: 2,
        programs: 3,
        unreadable_count: 1,
        unreadable: ['C:\\proj\\x: denied'],
      }),
    );
    const { user } = setup();
    await reachReview(user);
    expect(screen.getByText(/2 links \/ shortcuts will be skipped/)).toBeInTheDocument();
    expect(screen.getByText(/3 program files/)).toHaveTextContent(
      /can’t run Windows or Linux programs/,
    );
    expect(screen.getByText(/1 item can’t be read/)).toBeInTheDocument();
  });

  it('Link mode shows the edits-affect-originals warning and needs acknowledgement', async () => {
    api.start.mockResolvedValue(progress({ state: 'done', items_done: 1 }));
    const { user, onChanged } = setup();
    await reachReview(user);
    await user.click(screen.getByLabelText(/Link/));
    expect(screen.getByRole('alert')).toHaveTextContent(/original files.*changes or removes them/s);
    const go = screen.getByText('Link originals');
    expect(go).toBeDisabled();
    await user.click(
      screen.getByLabelText(/I understand that changes made in NovaOS affect the original files/),
    );
    expect(go).toBeEnabled();
    await user.click(go);
    expect(api.start).toHaveBeenCalledWith('sel1', 'link', true);
    expect(await screen.findByTestId('import-result')).toHaveTextContent(
      /Linked 1 item.*affect the originals/,
    );
    expect(onChanged).toHaveBeenCalled();
    expect(screen.queryByText('Show in Nova Files')).not.toBeInTheDocument();
  });

  it('link mode ignores free space (nothing is copied)', async () => {
    api.scan.mockResolvedValue(scanReport({ enough_space: false }));
    const { user } = setup();
    await reachReview(user);
    await user.click(screen.getByLabelText(/Link/));
    await user.click(screen.getByLabelText(/I understand/));
    expect(screen.getByText('Link originals')).toBeEnabled();
  });

  it('shows start errors from Rust and lets the user go back', async () => {
    api.start.mockRejectedValue('another import is already running');
    const { user } = setup();
    await reachReview(user);
    await user.click(screen.getByText('Copy into NovaOS'));
    expect(await screen.findByRole('alert')).toHaveTextContent('another import is already running');
    await user.click(screen.getByText('Back'));
    expect(api.discard).toHaveBeenCalledWith('sel1');
    expect(screen.getByText('Import files…')).toBeInTheDocument();
  });

  it('shows recent imports and unlinks a linked one', async () => {
    api.list.mockResolvedValue([
      {
        id: 4,
        mode: 'link',
        name: 'Thesis',
        source_path: 'C:\\Docs\\Thesis',
        dest_rel: null,
        is_dir: true,
        files: 0,
        bytes: 0,
        errors: 0,
        status: 'linked',
        created_at: 1,
        available: true,
      },
      {
        id: 3,
        mode: 'copy',
        name: 'Photos',
        source_path: 'D:\\Photos',
        dest_rel: 'Photos',
        is_dir: true,
        files: 12,
        bytes: 3 * MB,
        errors: 2,
        status: 'partial',
        created_at: 1,
        available: true,
      },
    ]);
    const { user, onChanged } = setup();
    await user.click(await screen.findByText(/Recent imports \(2\)/));
    expect(screen.getByText(/12 files, 3\.0 MB · 2 skipped/)).toBeInTheDocument();
    await user.click(screen.getByLabelText('Unlink Thesis'));
    expect(api.unlink).toHaveBeenCalledWith(4);
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(screen.queryByLabelText('Unlink Photos')).not.toBeInTheDocument();
  });

  it('closing during review discards the pending selection', async () => {
    const { user, onClose } = setup();
    await reachReview(user);
    await user.click(screen.getByLabelText('Close import'));
    expect(api.discard).toHaveBeenCalledWith('sel1');
    expect(onClose).toHaveBeenCalled();
  });
});
