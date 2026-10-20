import { describe, expect, it } from 'vitest';
import {
  back,
  breadcrumbs,
  canBack,
  canForward,
  currentLocation,
  forward,
  formatSize,
  initialHistory,
  isTextFile,
  joinPath,
  navigate,
  parentPath,
} from './filesModel';

describe('path helpers', () => {
  it('joins, finds parents and builds breadcrumbs', () => {
    expect(joinPath('', 'a')).toBe('a');
    expect(joinPath('a/b', 'c')).toBe('a/b/c');
    expect(parentPath('a/b/c')).toBe('a/b');
    expect(parentPath('a')).toBe('');
    expect(parentPath('')).toBe('');
    expect(breadcrumbs('a/b')).toEqual([
      { name: 'a', path: 'a' },
      { name: 'b', path: 'a/b' },
    ]);
    expect(breadcrumbs('')).toEqual([]);
  });

  it('formats sizes', () => {
    expect(formatSize(0)).toBe('0 B');
    expect(formatSize(1023)).toBe('1023 B');
    expect(formatSize(1536)).toBe('1.5 KB');
    expect(formatSize(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(formatSize(300 * 1024 * 1024)).toBe('300 MB');
  });

  it('recognises text files', () => {
    expect(isTextFile('notes.TXT')).toBe(true);
    expect(isTextFile('main.rs')).toBe(true);
    expect(isTextFile('photo.png')).toBe(false);
    expect(isTextFile('.gitignore')).toBe(false);
    expect(isTextFile('noext')).toBe(false);
  });
});

describe('history', () => {
  const home = { root: 'nova', path: '' };
  it('goes back and forward, and navigating drops the forward branch', () => {
    let h = initialHistory(home);
    h = navigate(h, { root: 'nova', path: 'Documents' });
    h = navigate(h, { root: 'nova', path: 'Documents/x' });
    expect(canBack(h)).toBe(true);
    expect(canForward(h)).toBe(false);
    h = back(back(h));
    expect(currentLocation(h)).toEqual(home);
    expect(canForward(h)).toBe(true);
    h = forward(h);
    expect(currentLocation(h).path).toBe('Documents');
    h = navigate(h, { root: 'nova', path: 'Pictures' });
    expect(canForward(h)).toBe(false);
    expect(h.stack).toHaveLength(3);
  });

  it('ignores navigating to the current place and over-stepping', () => {
    const h = initialHistory(home);
    expect(navigate(h, home)).toBe(h);
    expect(back(h)).toBe(h);
    expect(forward(h)).toBe(h);
  });
});
