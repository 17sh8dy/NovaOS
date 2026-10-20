import { isTauri, native } from './bridge';

/**
 * Key/value persistence. SQLite (via Rust) inside the desktop shell, localStorage in a plain
 * browser so `vite dev` and the Playwright tests keep working.
 */
const PREFIX = 'novaos:';

export async function storageGet(key: string): Promise<string | null> {
  if (isTauri()) return native<string | null>('kv_get', { key });
  try {
    return localStorage.getItem(PREFIX + key);
  } catch {
    return null;
  }
}

export async function storageSet(key: string, value: string): Promise<void> {
  if (isTauri()) {
    await native<void>('kv_set', { key, value });
    return;
  }
  try {
    localStorage.setItem(PREFIX + key, value);
  } catch {
    /* storage unavailable; the in-memory state still works for this session */
  }
}
