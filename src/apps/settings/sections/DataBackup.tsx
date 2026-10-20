import { useState } from 'react';
import { isTauri } from '../../../native/bridge';
import { errorMessage, filesApi } from '../../files/filesApi';
import { useSettings } from '../../../settings/store';
import { Button, Group, Page, Row } from '../ui';

/** Where the backup lives inside the Nova Files sandbox. */
export const BACKUP_ROOT = 'nova';
export const BACKUP_PATH = 'Documents/novaos-settings.json';

export function DataBackup() {
  const { exportJson, importJson, reset } = useSettings();
  const [text, setText] = useState('');
  const [note, setNote] = useState<{ ok: boolean; msg: string } | null>(null);
  const [confirming, setConfirming] = useState(false);
  const desktop = isTauri();

  const say = (ok: boolean, msg: string) => setNote({ ok, msg });

  const apply = (json: string) => {
    try {
      importJson(json);
      say(true, 'Settings imported and applied.');
    } catch (e) {
      say(false, errorMessage(e));
    }
  };

  const save = async () => {
    try {
      await filesApi.createFile(BACKUP_ROOT, BACKUP_PATH).catch(() => undefined); // fine if it exists
      await filesApi.writeText(BACKUP_ROOT, BACKUP_PATH, exportJson());
      say(true, `Saved to Nova Files → ${BACKUP_PATH}`);
    } catch (e) {
      say(false, errorMessage(e));
    }
  };

  const load = async () => {
    try {
      apply(await filesApi.readText(BACKUP_ROOT, BACKUP_PATH));
    } catch (e) {
      say(false, `Could not read ${BACKUP_PATH}: ${errorMessage(e)}`);
    }
  };

  return (
    <Page
      title="Data & Backup"
      description="Export your settings, move them to another computer, or start fresh."
    >
      <Group title="Export">
        <Row label="Save a backup file" hint={`Writes ${BACKUP_PATH} in Nova Files.`}>
          <Button disabled={!desktop} onClick={() => void save()}>
            Save to Nova Files
          </Button>
        </Row>
        <Row
          label="Copy settings as text"
          hint="Paste them anywhere, or into Import on another machine."
        >
          <Button
            onClick={() => {
              setText(exportJson());
              void navigator.clipboard?.writeText(exportJson()).catch(() => undefined);
              say(true, 'Settings shown below and copied to the clipboard.');
            }}
          >
            Copy
          </Button>
        </Row>
      </Group>

      <Group title="Import">
        <div className="space-y-2 px-4 py-3">
          <textarea
            aria-label="Settings JSON"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Paste exported settings here"
            spellCheck={false}
            className="h-28 w-full resize-none rounded-lg bg-black/30 p-2 font-mono text-[11px] outline-none"
          />
          <div className="flex gap-2">
            <Button disabled={!text.trim()} onClick={() => apply(text)}>
              Apply pasted settings
            </Button>
            <Button disabled={!desktop} onClick={() => void load()}>
              Load from Nova Files
            </Button>
          </div>
        </div>
      </Group>

      {note && (
        <p role="status" className={`text-xs ${note.ok ? 'text-emerald-300' : 'text-rose-300'}`}>
          {note.msg}
        </p>
      )}

      <Group title="Reset">
        <Row
          label="Reset all settings"
          hint="Returns every setting to its default. Your files are not touched."
        >
          {confirming ? (
            <span className="flex gap-2">
              <Button
                danger
                onClick={() => {
                  reset();
                  setConfirming(false);
                  say(true, 'All settings were reset.');
                }}
              >
                Yes, reset
              </Button>
              <Button onClick={() => setConfirming(false)}>Cancel</Button>
            </span>
          ) : (
            <Button onClick={() => setConfirming(true)}>Reset…</Button>
          )}
        </Row>
      </Group>
    </Page>
  );
}
