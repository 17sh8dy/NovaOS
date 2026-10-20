import { beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Desktop } from './Desktop';
import { registerBuiltinApps } from './builtinApps';
import { useWindows } from '../kernel/windowStore';
import { useDisplay } from './displayMode';
import { useShell } from './shellStore';

beforeAll(() => {
  registerBuiltinApps();
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

beforeEach(() => {
  useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
  useShell.setState({ startOpen: false });
  useDisplay.setState({ mode: 'windowed', error: null });
  localStorage.clear();
});

describe('Desktop shell', () => {
  it('launches an app from the Start menu and shows it on the taskbar', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Start'));
    await user.click(within(screen.getByRole('menu')).getByText('Nova Calculator'));
    expect(screen.getByTestId('window-calculator')).toBeInTheDocument();
    expect(screen.getByTestId('task-calculator')).toBeInTheDocument();
  });

  it('closes a window with its close button', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Open Nova Calculator'));
    await user.click(within(screen.getByTestId('window-calculator')).getByLabelText('Close'));
    expect(screen.queryByTestId('window-calculator')).not.toBeInTheDocument();
  });

  it('Settings is a singleton', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Open Nova Settings'));
    await user.click(screen.getByLabelText('Open Nova Settings'));
    expect(screen.getAllByTestId('window-settings')).toHaveLength(1);
  });

  it('calculator computes through the UI', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    await user.click(screen.getByLabelText('Open Nova Calculator'));
    const calc = within(screen.getByTestId('calculator'));
    for (const k of ['2', '+', '3', '×', '4', '='])
      await user.click(calc.getByText(k, { selector: 'button' }));
    expect(calc.getByLabelText('display')).toHaveTextContent('14');
  });

  it('refuses Shell Mode', async () => {
    await useDisplay.getState().setMode('shell');
    expect(useDisplay.getState().mode).toBe('windowed');
    expect(useDisplay.getState().error).toMatch(/not available/i);
  });

  it('Ctrl+Alt+Q always leaves Immersive Mode', async () => {
    const user = userEvent.setup();
    render(<Desktop />);
    useDisplay.setState({ mode: 'immersive' });
    await user.keyboard('{Control>}{Alt>}q{/Alt}{/Control}');
    expect(useDisplay.getState().mode).toBe('windowed');
  });
});
