import { useEffect, useState } from 'react';
import { Trash2 } from 'lucide-react';
import { native } from '../native/bridge';
import { DesktopIcon, type DesktopIconProps } from './DesktopIcon';

type Shared = Pick<DesktopIconProps, 'iconPx' | 'selected' | 'onSelect' | 'register'>;

/**
 * The Recycle Bin: always on the desktop. Everything deleted in Nova Files goes to the host
 * system's Recycle Bin (never deleted outright), so this opens that bin for review and restore.
 */
export function RecycleBin(shared: Shared) {
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    if (!notice) return;
    const t = window.setTimeout(() => setNotice(null), 5000);
    return () => window.clearTimeout(t);
  }, [notice]);

  const open = () => {
    native<void>('open_recycle_bin').catch((e) =>
      setNotice(
        typeof e === 'string' ? e : 'The Recycle Bin can be opened from the NovaOS desktop app.',
      ),
    );
  };

  return (
    <>
      <DesktopIcon
        {...shared}
        id="recycle-bin"
        label="Recycle Bin"
        ariaLabel="Recycle Bin"
        title="Recycle Bin: items deleted in Nova Files go here"
        icon={Trash2}
        iconClass="text-white/80"
        open={open}
      />
      {notice && (
        <div
          role="status"
          className="fixed bottom-16 left-1/2 z-[200000] -translate-x-1/2 rounded-lg border border-white/15 bg-black/80 px-4 py-2 text-xs shadow-xl"
        >
          {notice}
        </div>
      )}
    </>
  );
}
