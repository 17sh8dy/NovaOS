import { useEffect, useState, type ReactNode } from 'react';
import { ExternalLink as LinkIcon } from 'lucide-react';
import { LINKS, type LinkKey } from '../links';
import { isTauri, native } from '../native/bridge';

/**
 * A link to somewhere outside NovaOS. In the desktop app it opens your default browser through
 * Rust (which only knows the allow-listed addresses); the address is always visible on hover and
 * as the link's own href. If the browser can't be opened, the address is copied so nothing is lost.
 */
export function ExternalLink({
  to,
  children,
  className = '',
}: {
  to: LinkKey;
  children: ReactNode;
  className?: string;
}) {
  const link = LINKS[to];
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    if (!note) return;
    const t = window.setTimeout(() => setNote(null), 5000);
    return () => window.clearTimeout(t);
  }, [note]);

  const open = async (e: React.MouseEvent) => {
    e.preventDefault();
    if (!isTauri()) {
      window.open(link.url, '_blank', 'noopener,noreferrer');
      return;
    }
    try {
      await native<void>('open_link', { key: to });
    } catch {
      void navigator.clipboard?.writeText(link.url).catch(() => undefined);
      setNote(`Couldn’t open your browser. The link (${link.short}) was copied.`);
    }
  };

  return (
    <>
      <a
        href={link.url}
        title={link.url}
        onClick={(e) => void open(e)}
        className={`inline-flex items-center gap-1 text-[var(--accent)] hover:underline ${className}`}
      >
        {children}
        <LinkIcon size={11} aria-hidden="true" />
      </a>
      {note && (
        <span role="status" className="ml-2 text-xs text-amber-300">
          {note}
        </span>
      )}
    </>
  );
}
