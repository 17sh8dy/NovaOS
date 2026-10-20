import { useEffect, useRef, useState } from 'react';
import { storageGet, storageSet } from '../../native/storage';

const KEY = 'notes.main';

export function Notes() {
  const [text, setText] = useState('');
  const [ready, setReady] = useState(false);
  const [saved, setSaved] = useState(true);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    void storageGet(KEY).then((v) => {
      if (!alive) return;
      setText(v ?? '');
      setReady(true);
    });
    return () => {
      alive = false;
    };
  }, []);

  const onChange = (v: string) => {
    setText(v);
    setSaved(false);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      void storageSet(KEY, v).then(() => setSaved(true));
    }, 400);
  };

  // Flush a pending save when the window closes so the last keystrokes are not lost.
  const latest = useRef(text);
  latest.current = text;
  useEffect(
    () => () => {
      if (timer.current !== undefined) {
        window.clearTimeout(timer.current);
        void storageSet(KEY, latest.current);
      }
    },
    [],
  );

  return (
    <div className="flex h-full flex-col" data-testid="notes">
      <textarea
        aria-label="note"
        disabled={!ready}
        value={text}
        onChange={(e) => onChange(e.target.value)}
        placeholder="Start typing…"
        className="flex-1 resize-none bg-transparent p-3 text-sm outline-none"
      />
      <div className="border-t border-white/10 px-3 py-1 text-xs text-white/50">
        {saved ? 'Saved' : 'Saving…'} · {text.length} characters
      </div>
    </div>
  );
}
