import { MAX_PINNED, patchSection, type SettingsData } from './schema';

export type PinPlace = 'taskbar' | 'desktop';

const key = (place: PinPlace) => (place === 'taskbar' ? 'taskbarPinned' : 'desktopPinned');

export const pinsFor = (data: SettingsData, place: PinPlace): string[] => data.desktop[key(place)];

export const isPinned = (data: SettingsData, place: PinPlace, id: string): boolean =>
  pinsFor(data, place).includes(id);

/** Pin (appended at the end) or unpin an app. Pinning something already pinned changes nothing. */
export function setPinned(
  data: SettingsData,
  place: PinPlace,
  id: string,
  pinned: boolean,
): SettingsData {
  const cur = pinsFor(data, place);
  if (pinned === cur.includes(id)) return data;
  if (pinned && cur.length >= MAX_PINNED) return data;
  return patchSection(data, 'desktop', {
    [key(place)]: pinned ? [...cur, id] : cur.filter((x) => x !== id),
  });
}

/** Drop a pinned app into the slot another pinned app holds (drag and drop). */
export function reorderPinned(
  data: SettingsData,
  place: PinPlace,
  id: string,
  targetId: string,
): SettingsData {
  const cur = [...pinsFor(data, place)];
  const from = cur.indexOf(id);
  const to = cur.indexOf(targetId);
  if (from < 0 || to < 0 || from === to) return data;
  cur.splice(to, 0, ...cur.splice(from, 1));
  return patchSection(data, 'desktop', { [key(place)]: cur });
}

/** Move a pinned app earlier (-1) or later (+1). Stops at the ends. */
export function movePinned(
  data: SettingsData,
  place: PinPlace,
  id: string,
  delta: -1 | 1,
): SettingsData {
  const cur = [...pinsFor(data, place)];
  const from = cur.indexOf(id);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= cur.length) return data;
  [cur[from], cur[to]] = [cur[to], cur[from]];
  return patchSection(data, 'desktop', { [key(place)]: cur });
}
