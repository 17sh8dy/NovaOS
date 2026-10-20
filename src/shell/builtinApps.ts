import {
  Calculator as CalcIcon,
  FileText,
  FolderOpen,
  NotebookPen,
  Settings as SettingsIcon,
} from 'lucide-react';
import { getApp, registerApp } from '../kernel/appRegistry';
import { Calculator } from '../apps/calculator/Calculator';
import { Notes } from '../apps/notes/Notes';
import { Settings } from '../apps/settings/Settings';
import { Files } from '../apps/files/Files';
import { Editor } from '../apps/editor/Editor';

/** Idempotent so hot reload and tests can call it repeatedly. */
export function registerBuiltinApps(): void {
  if (getApp('settings')) return;
  registerApp({
    id: 'files',
    title: 'Nova Files',
    icon: FolderOpen,
    component: Files,
    defaultSize: { width: 720, height: 480 },
    pinned: true,
  });
  registerApp({
    id: 'editor',
    title: 'Nova Editor',
    icon: FileText,
    component: Editor,
    defaultSize: { width: 560, height: 420 },
    hidden: true,
  });
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
