import { useCallback, useMemo, useState } from 'react';
import { patchSection, type SettingsData } from '../../settings/schema';
import { useSettings } from '../../settings/store';
import type { SetFn } from './controls';
import { record, stepRedo, stepUndo, useStudioHistory } from './history';

export interface StudioSession {
  /** What the UI and the preview show: the draft while changes are waiting, else the live settings. */
  data: SettingsData;
  advanced: boolean;
  instant: boolean;
  /** Draft mode with changes that are not applied yet. */
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  set: SetFn;
  /** Replace all customization at once (looks, resets). Goes through the same undo/draft rules. */
  applyData: (next: SettingsData, key: string) => void;
  undo: () => void;
  redo: () => void;
  apply: () => void;
  discard: () => void;
  setInstant: (on: boolean) => void;
  setAdvanced: (on: boolean) => void;
  favorites: string[];
  toggleFavorite: (id: string) => void;
}

const same = (a: SettingsData, b: SettingsData) => JSON.stringify(a) === JSON.stringify(b);

/**
 * Nova Studio's editing session. In Instant mode every change is applied (and undoable) at once;
 * in Draft mode changes wait in a preview until Apply. Studio's own layout settings (sidebar,
 * favorites, saved looks) are never held back: they apply immediately in both modes.
 */
export function useStudioSession(): StudioSession {
  const live = useSettings((s) => s.data);
  const setAll = useSettings((s) => s.setAll);
  const history = useStudioHistory();
  const [draft, setDraft] = useState<SettingsData | null>(null);

  const instant = live.studio.instantApply;
  const dirty = !instant && draft !== null && !same({ ...draft, studio: live.studio }, live);
  // The draft never owns the `studio` section: that always comes from the live settings.
  const data = useMemo<SettingsData>(
    () => (!instant && draft ? { ...draft, studio: live.studio } : live),
    [instant, draft, live],
  );

  const commit = useCallback(
    (next: SettingsData, key: string) => {
      history.set(record(history, live, key, Date.now()));
      setAll({ ...next, studio: live.studio });
    },
    [history, live, setAll],
  );

  const applyData = useCallback(
    (next: SettingsData, key: string) => {
      if (instant) commit(next, key);
      else setDraft(next);
    },
    [instant, commit],
  );

  const set: SetFn = (section, patch) => {
    if (section === 'studio') {
      // Studio chrome: straight to the live settings, no history.
      setAll(patchSection(live, 'studio', patch as Partial<SettingsData['studio']>));
      return;
    }
    const base = instant ? live : (draft ?? live);
    const next = patchSection(base, section, patch);
    if (instant) commit(next, `${section}.${Object.keys(patch).join(',')}`);
    else setDraft(next);
  };

  const undo = () => {
    const r = stepUndo(history, live);
    if (!r) return;
    history.set(r.state);
    setAll({ ...r.data, studio: live.studio });
  };

  const redo = () => {
    const r = stepRedo(history, live);
    if (!r) return;
    history.set(r.state);
    setAll({ ...r.data, studio: live.studio });
  };

  const apply = () => {
    if (!draft) return;
    commit(draft, 'apply');
    setDraft(null);
  };

  const discard = () => setDraft(null);

  const setInstant = (on: boolean) => {
    // Turning Instant on applies whatever was waiting, so nothing is silently lost.
    if (on && draft && dirty) commit(draft, 'apply');
    setDraft(null);
    setAll(patchSection(useSettings.getState().data, 'studio', { instantApply: on }));
  };

  const setAdvanced = (on: boolean) => setAll(patchSection(live, 'studio', { advanced: on }));

  const toggleFavorite = (id: string) => {
    const cur = live.studio.favorites;
    setAll(
      patchSection(live, 'studio', {
        favorites: cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id],
      }),
    );
  };

  return {
    data,
    advanced: live.studio.advanced,
    instant,
    dirty,
    canUndo: instant && history.undo.length > 0,
    canRedo: instant && history.redo.length > 0,
    set,
    applyData,
    undo,
    redo,
    apply,
    discard,
    setInstant,
    setAdvanced,
    favorites: live.studio.favorites,
    toggleFavorite,
  };
}
