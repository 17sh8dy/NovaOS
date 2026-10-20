import { useEffect, useState } from 'react';
import { errorMessage, filesApi } from '../files/filesApi';

const MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
  svg: 'image/svg+xml',
};

/** Read-only image viewer for files opened from Nova Files. Click to toggle fit / actual size. */
export function Viewer({ args }: { args?: Record<string, string> }) {
  const root = args?.root;
  const path = args?.path;
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fit, setFit] = useState(true);

  useEffect(() => {
    if (root === undefined || path === undefined) {
      setError('No image was specified.');
      return;
    }
    let alive = true;
    let objectUrl: string | null = null;
    const ext = path.split('.').pop()?.toLowerCase() ?? '';
    filesApi
      .readBinary(root, path)
      .then((buf) => {
        if (!alive) return;
        // An <img> never runs scripts, even for SVG, so this is safe for untrusted images.
        objectUrl = URL.createObjectURL(
          new Blob([buf], { type: MIME[ext] ?? 'application/octet-stream' }),
        );
        setUrl(objectUrl);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [root, path]);

  const name = path?.split('/').pop() || root || '';
  return (
    <div className="flex h-full flex-col" data-testid="viewer">
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto bg-black/30 p-2">
        {error && (
          <p role="alert" className="text-xs text-rose-300">
            {error}
          </p>
        )}
        {!error && !url && <p className="text-xs text-white/40">Loading…</p>}
        {url && (
          <img
            src={url}
            alt={name}
            onClick={() => setFit((f) => !f)}
            className={
              fit ? 'max-h-full max-w-full cursor-zoom-in object-contain' : 'cursor-zoom-out'
            }
          />
        )}
      </div>
      <div className="flex justify-between border-t border-white/10 px-3 py-1 text-xs text-white/50">
        <span className="truncate">{name}</span>
        <span>{fit ? 'Fit to window' : 'Actual size'}</span>
      </div>
    </div>
  );
}
