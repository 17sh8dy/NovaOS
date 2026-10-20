import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Settings, SECTIONS } from './Settings';
import { Desktop } from '../../shell/Desktop';
import { registerBuiltinApps } from '../../shell/builtinApps';
import { useWindows } from '../../kernel/windowStore';
import { useDisplay } from '../../shell/displayMode';
import { useShell } from '../../shell/shellStore';
import { DEFAULTS } from '../../settings/schema';
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
  useSettings.setState({ data: DEFAULTS, loaded: false });
  useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
  useShell.setState({ startOpen: false });
  useDisplay.setState({ mode: 'windowed', error: null });
});

const data = () => useSettings.getState().data;

describe('Settings app', () => {
  it('has at least five sections and every one renders its own page', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    expect(SECTIONS.length).toBeGreaterThanOrEqual(5);
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    for (const s of SECTIONS) {
      await user.click(within(nav).getByText(s.title));
      expect(
        screen.getByRole('region', { name: new RegExp(s.title.replace(/&/g, '&'), 'i') }),
      ).toBeInTheDocument();
    }
  });

  it('search finds sections by keyword and shows a message when nothing matches', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    await user.type(screen.getByLabelText('Search settings'), 'blur');
    expect(within(nav).getByText('Appearance')).toBeInTheDocument();
    expect(within(nav).queryByText('General')).not.toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Appearance' })).toBeInTheDocument();
    await user.clear(screen.getByLabelText('Search settings'));
    await user.type(screen.getByLabelText('Search settings'), 'zzzz');
    expect(screen.getByText('No settings match.')).toBeInTheDocument();
  });

  it('General: clock format, seconds and date change the saved settings and the preview', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    await user.click(screen.getByRole('radio', { name: '24-hour' }));
    await user.click(screen.getByRole('switch', { name: 'Show seconds' }));
    await user.click(screen.getByRole('switch', { name: 'Show date' }));
    expect(data().general).toMatchObject({
      clockFormat: '24h',
      showSeconds: true,
      showDate: false,
    });
    expect(screen.getByTestId('clock-preview').textContent).toMatch(/^\d{2}:\d{2}:\d{2}$/);
  });

  it('General: the single-click toggle saves, and Settings itself never needs a double click', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    const toggle = screen.getByRole('switch', { name: 'Open with a single click' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    await user.click(toggle);
    expect(data().general.singleClickOpen).toBe(true);
    // Settings controls and navigation respond to ONE click in both modes.
    await user.click(
      within(screen.getByRole('navigation', { name: 'Settings sections' })).getByText('Appearance'),
    );
    expect(screen.getByRole('region', { name: 'Appearance' })).toBeInTheDocument();
    await user.click(screen.getByLabelText('accent teal'));
    expect(data().appearance.accent).toBe('teal');
    await user.click(
      within(screen.getByRole('navigation', { name: 'Settings sections' })).getByText('General'),
    );
    await user.click(screen.getByRole('switch', { name: 'Open with a single click' }));
    expect(data().general.singleClickOpen).toBe(false);
  });

  it('General: Shell Mode is shown but cannot be chosen', () => {
    render(<Settings />);
    expect(screen.getByRole('radio', { name: 'Shell (soon)' })).toBeDisabled();
  });

  it('Appearance: accent, wallpaper, opacity, blur and corners all save', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    await user.click(screen.getByText('Appearance'));
    await user.click(screen.getByLabelText('accent teal'));
    await user.click(screen.getByLabelText('wallpaper ocean'));
    await user.click(screen.getByRole('radio', { name: 'Sharp' }));
    expect(data().appearance).toMatchObject({
      accent: 'teal',
      wallpaper: 'ocean',
      corners: 'sharp',
    });
    const opacity = screen.getByLabelText('Window opacity') as HTMLInputElement;
    const { fireEvent } = await import('@testing-library/react');
    fireEvent.change(opacity, { target: { value: '0.6' } });
    fireEvent.change(screen.getByLabelText('Background blur'), { target: { value: '0' } });
    expect(data().appearance.windowOpacity).toBe(0.6);
    expect(data().appearance.blur).toBe(0);
    fireEvent.change(screen.getByLabelText('custom accent color'), {
      target: { value: '#12abef' },
    });
    expect(data().appearance.accent).toBe('#12abef');
  });

  it('Data & Backup: pasted settings are applied, junk is rejected with a reason, reset needs confirming', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    await user.click(screen.getByText('Data & Backup'));
    const box = screen.getByLabelText('Settings JSON');
    await user.click(box);
    await user.paste('{"hello":1}');
    await user.click(screen.getByText('Apply pasted settings'));
    expect(screen.getByRole('status')).toHaveTextContent(/not a NovaOS settings export/);
    expect(data()).toEqual(DEFAULTS);

    await user.clear(box);
    await user.click(box);
    await user.paste('{"app":"novaos","version":1,"settings":{"general":{"clockFormat":"24h"}}}');
    await user.click(screen.getByText('Apply pasted settings'));
    expect(screen.getByRole('status')).toHaveTextContent('imported and applied');
    expect(data().general.clockFormat).toBe('24h');

    await user.click(screen.getByText('Reset…'));
    expect(data().general.clockFormat).toBe('24h'); // not yet
    await user.click(screen.getByText('Cancel'));
    expect(data().general.clockFormat).toBe('24h');
    await user.click(screen.getByText('Reset…'));
    await user.click(screen.getByText('Yes, reset'));
    expect(data()).toEqual(DEFAULTS);
  });

  it('Privacy & Security tells the truth about programs and shell mode', async () => {
    const user = userEvent.setup();
    render(<Settings />);
    await user.click(screen.getByText('Privacy & Security'));
    expect(screen.getByText(/can't run Windows or Linux programs/)).toBeInTheDocument();
    expect(screen.getByText(/Off\. NovaOS is an application/)).toBeInTheDocument();
  });
});

describe('settings are applied to the desktop', () => {
  it('taskbar position and size, desktop icons and icon size', () => {
    render(<Desktop />);
    expect(screen.getByTestId('taskbar')).toHaveAttribute('data-position', 'bottom');
    expect(screen.getByTestId('taskbar')).toHaveStyle({ height: '48px' });
    expect(screen.getByTestId('desktop-icons')).toBeInTheDocument();

    useSettings.getState().set('desktop', { taskbarPosition: 'top', taskbarSize: 'large' });
    return Promise.resolve()
      .then(() => {
        // re-render happens synchronously with zustand; assert after the tick
        expect(screen.getByTestId('taskbar')).toHaveAttribute('data-position', 'top');
        expect(screen.getByTestId('taskbar')).toHaveStyle({ height: '58px' });
        useSettings.getState().set('desktop', { showDesktopIcons: false });
      })
      .then(() => {
        expect(screen.queryByLabelText('Nova Notes shortcut')).not.toBeInTheDocument();
        expect(screen.getByLabelText('Recycle Bin')).toBeInTheDocument(); // never hidden
      });
  });

  it('unpinning an app removes it from the taskbar and the desktop but not the Start menu', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    expect(screen.getByLabelText('Open Nova Notes')).toBeInTheDocument();
    useSettings.getState().set('desktop', { unpinned: ['notes'] });
    await Promise.resolve();
    expect(screen.queryByLabelText('Open Nova Notes')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Nova Notes shortcut')).not.toBeInTheDocument();
    await user.click(screen.getByLabelText('Start'));
    expect(within(screen.getByRole('menu')).getByText('Nova Notes')).toBeInTheDocument();
  });

  it('accent, opacity, blur, corners, contrast and motion reach the page as CSS', async () => {
    render(<Desktop />);
    useSettings
      .getState()
      .set('appearance', { accent: '#112233', windowOpacity: 0.6, blur: 8, corners: 'round' });
    useSettings
      .getState()
      .set('accessibility', { highContrast: true, reduceMotion: true, uiScale: 1.2 });
    await Promise.resolve();
    const el = screen.getByTestId('desktop');
    expect(el.style.getPropertyValue('--accent')).toBe('#112233');
    expect(el.style.getPropertyValue('--blur')).toBe('8px');
    expect(el.style.getPropertyValue('--radius')).toBe('20px');
    expect(el.style.getPropertyValue('--surface')).toBe('rgb(20 22 34 / 1)'); // high contrast = opaque
    expect(el).toHaveAttribute('data-contrast', 'high');
    expect(el).toHaveAttribute('data-motion', 'reduced');
    expect(document.documentElement.style.fontSize).toBe('120%');
    useSettings.getState().set('accessibility', { highContrast: false });
    await Promise.resolve();
    expect(el.style.getPropertyValue('--surface')).toBe('rgb(20 22 34 / 0.6)');
  });

  it('24-hour clock with no date reaches the taskbar', async () => {
    render(<Desktop />);
    useSettings.getState().set('general', { clockFormat: '24h', showDate: false });
    await Promise.resolve();
    const clock = screen.getByLabelText('clock');
    expect(clock.textContent).toMatch(/^\d{2}:\d{2}$/);
  });

  it('Start in Immersive Mode applies once, after settings load, and can be left', async () => {
    const setMode = vi.fn(async (m: 'windowed' | 'immersive' | 'shell') => {
      useDisplay.setState({ mode: m });
    });
    useDisplay.setState({ setMode });
    localStorage.setItem(
      'novaos:settings',
      JSON.stringify({ general: { startInImmersive: true } }),
    );
    render(<Desktop />);
    await vi.waitFor(() => expect(setMode).toHaveBeenCalledWith('immersive'));
    expect(setMode).toHaveBeenCalledTimes(1);
    // leaving it must not bounce back in
    await useDisplay.getState().exitImmersive();
    await Promise.resolve();
    expect(setMode).toHaveBeenCalledTimes(1);
  });
});
