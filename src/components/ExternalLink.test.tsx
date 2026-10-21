import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const state = { tauri: true };
vi.mock('../native/bridge', () => ({
  isTauri: () => state.tauri,
  native: vi.fn(),
}));
vi.mock('../apps/files/filesApi', () => ({
  errorMessage: (e: unknown) => String(e),
  filesApi: {
    roots: vi.fn().mockResolvedValue([]),
    list: vi.fn().mockResolvedValue({ entries: [], truncated: false }),
    createFile: vi.fn(),
    writeText: vi.fn(),
    readText: vi.fn(),
    trash: vi.fn(),
    readBinary: vi.fn(),
  },
}));
vi.mock('../apps/files/importApi', () => ({
  importApi: { list: vi.fn().mockResolvedValue([]), unlink: vi.fn() },
}));

import { ExternalLink } from './ExternalLink';
import { LINKS } from '../links';
import { native } from '../native/bridge';
import { About } from '../apps/settings/sections/About';
import { DataBackup } from '../apps/settings/sections/DataBackup';
import { FilesStorage } from '../apps/settings/sections/FilesStorage';
import { General } from '../apps/settings/sections/General';
import { Privacy } from '../apps/settings/sections/Privacy';
import { Shortcuts } from '../apps/settings/sections/Shortcuts';
import { Studio } from '../apps/studio/Studio';
import { registerBuiltinApps } from '../shell/builtinApps';
import { DEFAULTS } from '../settings/schema';
import { useSettings } from '../settings/store';

const nativeMock = vi.mocked(native);

beforeEach(() => {
  vi.clearAllMocks();
  state.tauri = true;
  nativeMock.mockResolvedValue(undefined);
  localStorage.clear();
  useSettings.setState({ data: DEFAULTS, loaded: true });
  registerBuiltinApps();
});

describe('the Discord link', () => {
  it('is exactly the invite you gave', () => {
    expect(LINKS.discord.url).toBe('https://discord.gg/XBhER9Z6EB');
    expect(LINKS.discord.short).toBe('discord.gg/XBhER9Z6EB');
    expect(LINKS.discord.url.startsWith('https://')).toBe(true);
  });

  it('shows its destination (href and tooltip) so nobody is surprised', () => {
    render(<ExternalLink to="discord">Join the Discord</ExternalLink>);
    const link = screen.getByRole('link', { name: /Join the Discord/ });
    expect(link).toHaveAttribute('href', 'https://discord.gg/XBhER9Z6EB');
    expect(link).toHaveAttribute('title', 'https://discord.gg/XBhER9Z6EB');
  });

  it('opens through the desktop shell using a key, never a URL from the UI', async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    render(<ExternalLink to="discord">Join the Discord</ExternalLink>);
    await user.click(screen.getByRole('link'));
    expect(nativeMock).toHaveBeenCalledWith('open_link', { key: 'discord' });
    expect(JSON.stringify(nativeMock.mock.calls)).not.toContain('http');
    expect(open).not.toHaveBeenCalled(); // and the webview itself never navigates away
  });

  it('in a plain browser it opens a new tab without leaking the opener', async () => {
    const user = userEvent.setup();
    state.tauri = false;
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    render(<ExternalLink to="discord">Join the Discord</ExternalLink>);
    await user.click(screen.getByRole('link'));
    expect(open).toHaveBeenCalledWith(
      'https://discord.gg/XBhER9Z6EB',
      '_blank',
      'noopener,noreferrer',
    );
    expect(nativeMock).not.toHaveBeenCalled();
  });

  it('if the browser cannot be opened it says so and copies the address', async () => {
    const user = userEvent.setup();
    nativeMock.mockRejectedValue('could not open your browser: nope');
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    render(<ExternalLink to="discord">Join the Discord</ExternalLink>);
    await user.click(screen.getByRole('link'));
    expect(await screen.findByRole('status')).toHaveTextContent(/copied/i);
    expect(writeText).toHaveBeenCalledWith('https://discord.gg/XBhER9Z6EB');
    await waitFor(() => expect(screen.queryByRole('status')).toBeInTheDocument());
  });
});

describe('links appear only where they belong (no flooding)', () => {
  const discordLinks = () => screen.queryAllByRole('link', { name: /discord/i });

  it('Settings: only About has one; every other Settings page has none', () => {
    for (const Page of [General, FilesStorage, Shortcuts, Privacy, DataBackup]) {
      const { unmount } = render(<Page />);
      expect(discordLinks(), Page.name).toHaveLength(0);
      unmount();
    }
    render(<About />);
    expect(discordLinks()).toHaveLength(1);
    expect(
      screen.getByText('Suggest ideas, report bugs and chat with other NovaOS users.'),
    ).toBeInTheDocument();
  });

  it('Nova Studio: one on Home (ideas), one in Library (sharing), none on any studio page', async () => {
    const user = userEvent.setup();
    render(<Studio />);
    expect(discordLinks()).toHaveLength(1);
    expect(screen.getByText(/suggest an idea for Nova Studio/i)).toBeInTheDocument();
    const nav = screen.getByRole('navigation', { name: 'Studios' });
    for (const title of [
      'Themes',
      'Windows',
      'Taskbar',
      'Desktop',
      'Motion',
      'Cursor',
      'Sounds',
      'Interface',
    ]) {
      await user.click(nav.querySelector(`[aria-label="${title}"]`) as HTMLElement);
      expect(discordLinks(), title).toHaveLength(0);
    }
    await user.click(nav.querySelector('[aria-label="Library"]') as HTMLElement);
    expect(discordLinks()).toHaveLength(1);
    expect(screen.getByText(/Share it in the Discord/i)).toBeInTheDocument();
  });
});
