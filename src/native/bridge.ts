import { invoke } from '@tauri-apps/api/core';

/** True when running inside the Tauri shell (as opposed to a plain browser during `vite dev`). */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * The only door from the UI to the Rust backend. Every privileged capability is a named
 * command registered in src-tauri; the frontend never gets raw filesystem or shell access.
 */
export async function native<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error(`native command "${command}" needs the NovaOS desktop shell`);
  return invoke<T>(command, args);
}

export interface PlatformInfo {
  os: string;
  arch: string;
  version: string;
  /** Always false for now: NovaOS is an application, not a kernel or system shell. */
  shell_mode_available: boolean;
}
