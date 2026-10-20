export interface Point {
  x: number;
  y: number;
}

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** A drag must travel this far before it becomes a selection box, so plain clicks never flash one. */
export const DRAG_THRESHOLD = 4;

export const boxFrom = (a: Point, b: Point): Box => ({
  left: Math.min(a.x, b.x),
  top: Math.min(a.y, b.y),
  right: Math.max(a.x, b.x),
  bottom: Math.max(a.y, b.y),
});

/** Touching edges do not count as overlapping. */
export const intersects = (a: Box, b: Box): boolean =>
  a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;

export const exceedsThreshold = (a: Point, b: Point): boolean =>
  Math.hypot(a.x - b.x, a.y - b.y) >= DRAG_THRESHOLD;

/** Keep a point inside a container so the box never extends past the desktop. */
export const clampPoint = (p: Point, within: Box): Point => ({
  x: Math.min(within.right, Math.max(within.left, p.x)),
  y: Math.min(within.bottom, Math.max(within.top, p.y)),
});

/** Ids of every item whose box overlaps the selection box. */
export function hitTest(selection: Box, items: Map<string, Box>): string[] {
  const hits: string[] = [];
  for (const [id, box] of items) if (intersects(selection, box)) hits.push(id);
  return hits;
}

/** With Ctrl held a drag adds to the existing selection; otherwise it replaces it. */
export const mergeSelection = (base: string[], hits: string[], additive: boolean): string[] =>
  additive ? [...new Set([...base, ...hits])] : hits;
