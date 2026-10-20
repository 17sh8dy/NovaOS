import { beforeEach, describe, expect, it } from 'vitest';
import { MIN_WINDOW, TITLEBAR_GRAB, useWindows } from './windowStore';

const desktop = { width: 1280, height: 720 };
const opts = { appId: 'calc', title: 'Calc', size: { width: 400, height: 300 } };
const s = () => useWindows.getState();

beforeEach(() => {
  useWindows.setState({ windows: {}, activeId: null, nextZ: 1, counter: 0 });
});

describe('window manager', () => {
  it('opens a window, focuses it and cascades the next one', () => {
    const a = s().open(opts, desktop);
    const b = s().open(opts, desktop);
    expect(a).not.toBe(b);
    expect(s().activeId).toBe(b);
    expect(s().windows[b].rect.x).toBeGreaterThan(s().windows[a].rect.x);
    expect(s().windows[b].z).toBeGreaterThan(s().windows[a].z);
  });

  it('reuses a singleton instead of opening a second window', () => {
    const a = s().open({ ...opts, singleton: true }, desktop);
    s().minimize(a);
    const b = s().open({ ...opts, singleton: true }, desktop);
    expect(b).toBe(a);
    expect(Object.keys(s().windows)).toHaveLength(1);
    expect(s().windows[a].minimized).toBe(false);
    expect(s().activeId).toBe(a);
  });

  it('focus raises z-order', () => {
    const a = s().open(opts, desktop);
    const b = s().open(opts, desktop);
    s().focus(a);
    expect(s().windows[a].z).toBeGreaterThan(s().windows[b].z);
    expect(s().activeId).toBe(a);
  });

  it('minimizing the active window hands focus to the next visible one', () => {
    const a = s().open(opts, desktop);
    const b = s().open(opts, desktop);
    s().minimize(b);
    expect(s().activeId).toBe(a);
    s().minimize(a);
    expect(s().activeId).toBeNull();
  });

  it('closing the active window hands focus to the topmost remaining', () => {
    const a = s().open(opts, desktop);
    const b = s().open(opts, desktop);
    const c = s().open(opts, desktop);
    s().minimize(b);
    s().close(c);
    expect(s().activeId).toBe(a);
    expect(s().windows[c]).toBeUndefined();
  });

  it('taskbar toggles active -> minimized -> restored', () => {
    const a = s().open(opts, desktop);
    s().taskbarActivate(a);
    expect(s().windows[a].minimized).toBe(true);
    s().taskbarActivate(a);
    expect(s().windows[a].minimized).toBe(false);
    expect(s().activeId).toBe(a);
  });

  it('taskbar on a background window focuses instead of minimizing', () => {
    const a = s().open(opts, desktop);
    s().open(opts, desktop);
    s().taskbarActivate(a);
    expect(s().windows[a].minimized).toBe(false);
    expect(s().activeId).toBe(a);
  });

  it('keeps a window grabbable when dragged far off-screen', () => {
    const a = s().open(opts, desktop);
    s().move(a, 99999, 99999, desktop);
    let r = s().windows[a].rect;
    expect(r.x).toBe(desktop.width - TITLEBAR_GRAB);
    expect(r.y).toBe(desktop.height - TITLEBAR_GRAB);
    s().move(a, -99999, -99999, desktop);
    r = s().windows[a].rect;
    expect(r.x + r.width).toBe(TITLEBAR_GRAB);
    expect(r.y).toBe(0);
  });

  it('enforces a minimum size and ignores move/resize while maximized', () => {
    const a = s().open(opts, desktop);
    s().setRect(a, { x: 0, y: 0, width: 10, height: 10 });
    expect(s().windows[a].rect.width).toBe(MIN_WINDOW.width);
    expect(s().windows[a].rect.height).toBe(MIN_WINDOW.height);
    s().toggleMaximize(a);
    const before = s().windows[a].rect;
    s().move(a, 500, 500, desktop);
    s().setRect(a, { x: 1, y: 1, width: 999, height: 999 });
    expect(s().windows[a].rect).toEqual(before);
    s().toggleMaximize(a);
    expect(s().windows[a].maximized).toBe(false);
  });

  it('clamps a new window larger than the desktop', () => {
    const a = s().open({ ...opts, size: { width: 5000, height: 5000 } }, desktop);
    const r = s().windows[a].rect;
    expect(r.width).toBeLessThanOrEqual(desktop.width);
    expect(r.height).toBeLessThanOrEqual(desktop.height);
  });
});
