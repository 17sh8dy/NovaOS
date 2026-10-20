import { useEffect, useState } from 'react';
import { isTauri, native, type PlatformInfo } from '../../../native/bridge';
import { errorMessage, filesApi } from '../../files/filesApi';
import type { FsRoot } from '../../files/filesModel';
import { useSettings } from '../../../settings/store';
import { Button, Group, Page, Row, Toggle } from '../ui';

const KIND_LABEL: Record<FsRoot['kind'], string> = {
  nova: 'NovaOS folder',
  host: 'Your computer',
  custom: 'Added by you',
  link: 'Linked (original files)',
  linkfile: 'Linked file (original)',
};

export function FilesStorage() {
  const { data, set } = useSettings();
  const [roots, setRoots] = useState<FsRoot[]>([]);
  const [dataDir, setDataDir] = useState('');
  const [error, setError] = useState<string | null>(null);

  const load = () =>
    filesApi
      .roots()
      .then(setRoots)
      .catch(() => undefined);

  useEffect(() => {
    if (!isTauri()) return;
    void load();
    void native<PlatformInfo>('platform_info')
      .then((i) => setDataDir(i.data_dir))
      .catch(() => undefined);
  }, []);

  const add = async () => {
    setError(null);
    try {
      await filesApi.addRoot();
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const remove = async (id: string) => {
    setError(null);
    try {
      await filesApi.removeRoot(id);
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Page title="Files & Storage" description="What Nova Files can reach, and how it behaves.">
      <Group title="Nova Files behavior">
        <Row label="Confirm before deleting" hint="Deleted items always go to the Recycle Bin.">
          <Toggle
            label="Confirm before deleting"
            checked={data.files.confirmDelete}
            onChange={(v) => set('files', { confirmDelete: v })}
          />
        </Row>
        <Row label="Show file sizes">
          <Toggle
            label="Show file sizes"
            checked={data.files.showSizes}
            onChange={(v) => set('files', { showSizes: v })}
          />
        </Row>
      </Group>

      {isTauri() ? (
        <Group title="Folders NovaOS can access">
          {roots.map((r) => (
            <Row key={r.id} label={r.label} hint={`${KIND_LABEL[r.kind]} · ${r.path}`}>
              {r.kind === 'custom' && (
                <Button label={`Remove ${r.label}`} onClick={() => void remove(r.id)}>
                  Remove
                </Button>
              )}
            </Row>
          ))}
          <div className="px-4 py-3">
            <Button onClick={() => void add()}>Add folder…</Button>
            <p className="mt-2 text-xs text-white/50">
              Whole drives and system folders are never allowed. To bring files in, use Import in
              Nova Files.
            </p>
            {error && <p className="mt-1 text-xs text-rose-400">{error}</p>}
          </div>
        </Group>
      ) : (
        <p className="text-xs text-white/50">
          Folder access is available in the NovaOS desktop app.
        </p>
      )}

      {dataDir && (
        <Group title="Storage">
          <Row
            label="NovaOS data folder"
            hint="Holds the settings database and your Nova Files folder."
          >
            <code className="max-w-64 truncate text-[11px] text-white/60" title={dataDir}>
              {dataDir}
            </code>
          </Row>
        </Group>
      )}
    </Page>
  );
}
