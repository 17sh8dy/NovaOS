import { useEffect, useState } from 'react';
import { isTauri } from '../../../native/bridge';
import { errorMessage } from '../../files/filesApi';
import { importApi, type ImportRow } from '../../files/importApi';
import { Button, Group, Page, Row } from '../ui';

const FACTS: [string, string][] = [
  [
    'Your files',
    'NovaOS only reaches the locations listed in Files & Storage. Copying files in never changes the originals.',
  ],
  [
    'Programs',
    "NovaOS can't run Windows or Linux programs. Imported .exe, .bat, .sh and similar files are stored as data and never launched.",
  ],
  [
    'Network',
    'NovaOS makes no internet connections. Its pages are not allowed to contact any server.',
  ],
  ['Shell Mode', 'Off. NovaOS is an application running on your system, not a replacement for it.'],
  [
    'Linked items',
    'Linked files are your originals: changes made through NovaOS change them. Unlinking never touches them.',
  ],
];

export function Privacy() {
  const [links, setLinks] = useState<ImportRow[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = () =>
    importApi
      .list()
      .then((r) => setLinks(r.filter((x) => x.mode === 'link')))
      .catch(() => undefined);

  useEffect(() => {
    if (isTauri()) void load();
  }, []);

  const unlink = async (id: number) => {
    setError(null);
    try {
      await importApi.unlink(id);
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Page title="Privacy & Security" description="What NovaOS can and can't do on your computer.">
      <Group title="Protections">
        {FACTS.map(([label, text]) => (
          <Row key={label} label={label} hint={text} />
        ))}
      </Group>
      {isTauri() && (
        <Group title="Linked items">
          {links.length === 0 && (
            <p className="px-4 py-3 text-xs text-white/50">
              Nothing is linked. Everything you imported is a copy.
            </p>
          )}
          {links.map((l) => (
            <Row
              key={l.id}
              label={l.name}
              hint={`${l.source_path}${l.available ? '' : ' · unavailable'}`}
            >
              <Button label={`Unlink ${l.name}`} onClick={() => void unlink(l.id)}>
                Unlink
              </Button>
            </Row>
          ))}
          {error && <p className="px-4 py-2 text-xs text-rose-400">{error}</p>}
        </Group>
      )}
    </Page>
  );
}
