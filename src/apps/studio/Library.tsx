import { useState } from 'react';
import { Check, Trash2 } from 'lucide-react';
import { BUILTIN_LOOKS } from '../../settings/builtinPresets';
import {
  MAX_PRESETS,
  applyLook,
  cleanName,
  exportLook,
  importLook,
  pickPreset,
  type PresetData,
} from '../../settings/schema';
import { errorMessage, filesApi } from '../files/filesApi';
import { isTauri } from '../../native/bridge';
import { Button, Group, Page, Row } from '../settings/ui';
import type { StudioSession } from './useStudioSession';

const ROOT = 'nova';

const newId = () => `look-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 5)}`;

/** A safe file name for a look: letters, digits, dashes. */
export const lookFileName = (name: string): string => {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40);
  return `${slug || 'my-look'}.novalook.json`;
};

/** Built-in looks, your saved looks, and sharing: export a look, import someone else's. */
export function Library({ session }: { session: StudioSession }) {
  const { data, set, applyData } = session;
  const presets = data.studio.presets;
  const [name, setName] = useState('');
  const [text, setText] = useState('');
  const [fileName, setFileName] = useState('my-look.novalook.json');
  const [note, setNote] = useState<{ ok: boolean; msg: string } | null>(null);
  const say = (ok: boolean, msg: string) => setNote({ ok, msg });
  const desktop = isTauri();

  const applyBuiltin = (look: PresetData, label: string) => {
    applyData(applyLook(data, look), `look.${label}`);
    say(true, `Applied “${label}”.`);
  };

  const save = () => {
    const clean = cleanName(name);
    if (!clean) return say(false, 'Give your look a name first.');
    if (presets.length >= MAX_PRESETS)
      return say(false, `You can keep up to ${MAX_PRESETS} looks.`);
    set('studio', { presets: [...presets, { id: newId(), name: clean, data: pickPreset(data) }] });
    setName('');
    say(true, `Saved “${clean}”.`);
  };

  const remove = (id: string) => set('studio', { presets: presets.filter((p) => p.id !== id) });

  const addImported = (raw: string) => {
    try {
      const look = importLook(raw);
      if (presets.length >= MAX_PRESETS)
        return say(false, `You can keep up to ${MAX_PRESETS} looks.`);
      set('studio', { presets: [...presets, { id: newId(), name: look.name, data: look.data }] });
      say(true, `Added “${look.name}” to your looks. Press Use to apply it.`);
    } catch (e) {
      say(false, errorMessage(e));
    }
  };

  const currentName = cleanName(name) ?? 'My look';

  const saveFile = async () => {
    const file = lookFileName(fileName.replace(/\.novalook\.json$/i, ''));
    const path = `Documents/${file}`;
    try {
      await filesApi.createFile(ROOT, path).catch(() => undefined); // fine if it exists
      await filesApi.writeText(ROOT, path, exportLook(currentName, pickPreset(data)));
      say(true, `Saved to Nova Files → ${path}. Custom wallpapers are not included.`);
    } catch (e) {
      say(false, errorMessage(e));
    }
  };

  const loadFile = async () => {
    const file = lookFileName(fileName.replace(/\.novalook\.json$/i, ''));
    try {
      addImported(await filesApi.readText(ROOT, `Documents/${file}`));
    } catch (e) {
      say(false, `Could not read Documents/${file}: ${errorMessage(e)}`);
    }
  };

  return (
    <Page
      title="Library"
      description="Save how NovaOS looks, switch between looks instantly, and share them."
    >
      <Group title="Built-in looks">
        {BUILTIN_LOOKS.map((l) => (
          <Row key={l.id} label={l.name} hint={l.description}>
            <Button label={`Use ${l.name}`} onClick={() => applyBuiltin(l.data, l.name)}>
              Use
            </Button>
          </Row>
        ))}
      </Group>

      <Group title={`Your looks (${presets.length}/${MAX_PRESETS})`}>
        {presets.length === 0 && (
          <p className="px-4 py-3 text-xs text-white/50">
            Nothing saved yet. Set up NovaOS the way you like, then save it below.
          </p>
        )}
        {presets.map((p) => (
          <Row key={p.id} label={p.name}>
            <span className="flex gap-2">
              <Button label={`Use ${p.name}`} onClick={() => applyBuiltin(p.data, p.name)}>
                <span className="flex items-center gap-1">
                  <Check size={12} /> Use
                </span>
              </Button>
              <Button label={`Delete ${p.name}`} danger onClick={() => remove(p.id)}>
                <Trash2 size={12} />
              </Button>
            </span>
          </Row>
        ))}
        <div className="flex items-center gap-2 px-4 py-3">
          <input
            aria-label="Look name"
            value={name}
            maxLength={40}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && save()}
            placeholder="Name this look"
            className="flex-1 rounded-lg bg-black/30 px-3 py-1.5 text-xs outline-none"
          />
          <Button onClick={save}>Save current look</Button>
        </div>
      </Group>

      <Group title="Share">
        <Row
          label="Copy this look as text"
          hint="Send it to a friend. Custom wallpapers are not included."
        >
          <Button
            onClick={() => {
              const out = exportLook(currentName, pickPreset(data));
              setText(out);
              void navigator.clipboard?.writeText(out).catch(() => undefined);
              say(true, 'The look is shown below and copied to the clipboard.');
            }}
          >
            Copy
          </Button>
        </Row>
        <div className="space-y-2 px-4 py-3">
          <textarea
            aria-label="Look JSON"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Paste a look here to add it to yours"
            spellCheck={false}
            className="h-24 w-full resize-none rounded-lg bg-black/30 p-2 font-mono text-[11px] outline-none"
          />
          <Button disabled={!text.trim()} onClick={() => addImported(text)}>
            Add pasted look
          </Button>
        </div>
        <div className="flex flex-wrap items-center gap-2 px-4 py-3">
          <input
            aria-label="Look file name"
            value={fileName}
            onChange={(e) => setFileName(e.target.value)}
            className="w-56 rounded-lg bg-black/30 px-3 py-1.5 text-xs outline-none"
          />
          <Button disabled={!desktop} onClick={() => void saveFile()}>
            Save to Nova Files
          </Button>
          <Button disabled={!desktop} onClick={() => void loadFile()}>
            Load from Nova Files
          </Button>
          <span className="text-[11px] text-white/40">Files live in Nova Files → Documents.</span>
        </div>
      </Group>

      {note && (
        <p role="status" className={`text-xs ${note.ok ? 'text-emerald-300' : 'text-rose-300'}`}>
          {note.msg}
        </p>
      )}
    </Page>
  );
}
