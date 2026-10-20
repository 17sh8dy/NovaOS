import { Calculator as CalcIcon, NotebookPen, Settings as SettingsIcon } from 'lucide-react';
import { getApp, registerApp } from '../kernel/appRegistry';
import { Calculator } from '../apps/calculator/Calculator';
import { Notes } from '../apps/notes/Notes';
import { Settings } from '../apps/settings/Settings';

/** Idempotent so hot reload and tests can call it repeatedly. */
export function registerBuiltinApps(): void {
  if (getApp('settings')) return;
  registerApp({
    id: 'settings',
    title: 'Nova Settings',
    icon: SettingsIcon,
    component: Settings,
    defaultSize: { width: 520, height: 520 },
    singleton: true,
    pinned: true,
  });
  registerApp({
    id: 'notes',
    title: 'Nova Notes',
    icon: NotebookPen,
    component: Notes,
    defaultSize: { width: 480, height: 400 },
    singleton: true,
    pinned: true,
  });
  registerApp({
    id: 'calculator',
    title: 'Nova Calculator',
    icon: CalcIcon,
    component: Calculator,
    defaultSize: { width: 300, height: 420 },
    pinned: true,
  });
}
