import { useEffect, useRef, useState } from 'react';
import { errorMessage, filesApi } from '../files/filesApi';

type Status = 'loading' | 'saved' | 'dirty' | 'error';

/** Plain-text editor for files opened from Nova Files. Autosaves; Ctrl+S saves immediately. */
export function Editor({ args }: { args?: Record<string, string> }) {
  const root = args?.root;
  const path = args?.path;
  const [text, setText] = useState('');
  const [status, setStatus] = useState<Status>('loading');
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const latest = useRef({ text: '', dirty: false });

  useEffect(() => {
    if (!root || !path) {
      setError('No file was specified.');
      setStatus('error');
      return;
    }
    let alive = true;
    filesApi
      .readText(root, path)
      .then((t) => {
        if (!alive) return;
        latest.current = { text: t, dirty: false };
        setText(t);
        setStatus('saved');
      })
      .catch((e) => {
        if (!alive) return;
        setError(errorMessage(e));
        setStatus('error');
      });
    return () => {
      alive = false;
    };
  }, [root, path]);

  const save = async () => {
    if (!root || !path || !latest.current.dirty) return;
    window.clearTimeout(timer.current);
    const snapshot = latest.current.text;
    try {
      await filesApi.writeText(root, path, snapshot);
      if (latest.current.text === snapshot) {
        latest.current.dirty = false;
        setStatus('saved');
      }
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
      setStatus('error');
    }
  };

  const onChange = (v: string) => {
    setText(v);
    latest.current = { text: v, dirty: true };
    setStatus('dirty');
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void save(), 800);
  };

  // Flush unsaved edits if the window closes before the debounce fires.
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      if (root && path && latest.current.dirty) {
        void filesApi.writeText(root, path, latest.current.text).catch(() => undefined);
      }
    },
    [root, path],
  );

  const name = path?.split('/').pop() ?? '';
  return (
    <div className="flex h-full flex-col" data-testid="editor">
      {error && (
        <div role="alert" className="bg-rose-500/20 px-3 py-1.5 text-xs text-rose-200">
          {error}
        </div>
      )}
      <textarea
        aria-label={`Editing ${name}`}
        disabled={
          status === 'loading' || (status === 'error' && latest.current.text === '' && !text)
        }
        value={text}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
            e.preventDefault();
            void save();
          }
        }}
        className="flex-1 resize-none bg-transparent p-3 font-mono text-xs outline-none"
      />
      <div className="flex justify-between border-t border-white/10 px-3 py-1 text-xs text-white/50">
        <span className="truncate">{name}</span>
        <span>
          {status === 'dirty'
            ? 'Unsaved…'
            : status === 'saved'
              ? 'Saved'
              : status === 'loading'
                ? 'Loading…'
                : 'Error'}
        </span>
      </div>
    </div>
  );
}
