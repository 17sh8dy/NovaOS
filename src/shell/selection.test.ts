import { describe, expect, it } from 'vitest';
import {
  DRAG_THRESHOLD,
  boxFrom,
  clampPoint,
  exceedsThreshold,
  hitTest,
  intersects,
  mergeSelection,
} from './selection';

const box = (left: number, top: number, right: number, bottom: number) => ({
  left,
  top,
  right,
  bottom,
});

describe('selection geometry', () => {
  it('builds a normalized box from any drag direction', () => {
    expect(boxFrom({ x: 10, y: 20 }, { x: 50, y: 60 })).toEqual(box(10, 20, 50, 60));
    expect(boxFrom({ x: 50, y: 60 }, { x: 10, y: 20 })).toEqual(box(10, 20, 50, 60));
    expect(boxFrom({ x: 50, y: 20 }, { x: 10, y: 60 })).toEqual(box(10, 20, 50, 60));
  });

  it('detects overlap, but not mere touching or separation', () => {
    const a = box(0, 0, 10, 10);
    expect(intersects(a, box(5, 5, 15, 15))).toBe(true);
    expect(intersects(a, box(2, 2, 4, 4))).toBe(true); // fully inside
    expect(intersects(a, box(-5, -5, 20, 20))).toBe(true); // fully containing
    expect(intersects(a, box(10, 0, 20, 10))).toBe(false); // edges touch
    expect(intersects(a, box(11, 0, 20, 10))).toBe(false);
    expect(intersects(a, box(0, 11, 10, 20))).toBe(false);
  });

  it('only starts after the drag threshold', () => {
    expect(exceedsThreshold({ x: 0, y: 0 }, { x: 1, y: 1 })).toBe(false);
    expect(exceedsThreshold({ x: 0, y: 0 }, { x: DRAG_THRESHOLD, y: 0 })).toBe(true);
  });

  it('clamps points into the desktop', () => {
    const area = box(0, 0, 100, 80);
    expect(clampPoint({ x: -20, y: 200 }, area)).toEqual({ x: 0, y: 80 });
    expect(clampPoint({ x: 40, y: 40 }, area)).toEqual({ x: 40, y: 40 });
  });

  it('hit-tests a set of items and merges additively with Ctrl', () => {
    const items = new Map([
      ['a', box(0, 0, 20, 20)],
      ['b', box(0, 30, 20, 50)],
      ['c', box(100, 100, 120, 120)],
    ]);
    expect(hitTest(box(-5, -5, 30, 40), items)).toEqual(['a', 'b']);
    expect(hitTest(box(200, 200, 300, 300), items)).toEqual([]);
    expect(mergeSelection(['c'], ['a'], true)).toEqual(['c', 'a']);
    expect(mergeSelection(['c', 'a'], ['a', 'b'], true)).toEqual(['c', 'a', 'b']);
    expect(mergeSelection(['c'], ['a'], false)).toEqual(['a']);
  });
});
