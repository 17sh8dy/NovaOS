import {
  TEXT_TARGETS,
  TEXT_TARGET_LABELS,
  type BitDef,
  type BitParam,
  type TextTarget,
} from '../../settings/bits';
import { accentColor, isHexColor, type SettingsData } from '../../settings/schema';
import { Segmented, Toggle } from '../settings/ui';
import { AdvSlider, ColorPick } from './studioUi';

/** The glass pill that switches an effect on and off. */
export function GlassPill({
  label,
  on,
  onChange,
}: {
  label: string;
  on: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={`inline-flex items-center gap-2 rounded-full border px-4 py-1.5 text-xs font-medium backdrop-blur-md transition-colors ${
        on
          ? 'border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_28%,transparent)] text-white shadow-[0_0_14px_-2px_var(--accent),inset_0_1px_0_rgb(255_255_255/0.3)]'
          : 'border-white/20 bg-white/10 text-white/70 shadow-[inset_0_1px_0_rgb(255_255_255/0.2)] hover:bg-white/15'
      }`}
    >
      <span
        aria-hidden="true"
        className={`h-2 w-2 rounded-full ${on ? 'bg-[var(--accent)] shadow-[0_0_8px_var(--accent)]' : 'bg-white/40'}`}
      />
      {on ? 'On' : 'Off'}
    </button>
  );
}

type Fx = Record<string, unknown> & { enabled: boolean };

const formatSlider = (p: Extract<BitParam, { kind: 'slider' }>) => (v: number) => {
  if (p.percent) return `${Math.round(v * 100)}%`;
  const decimals = p.decimals ?? (p.range.step < 1 ? 1 : 0);
  return `${Number(v.toFixed(decimals))}${p.unit}`;
};

function Param({
  def,
  param,
  cur,
  patch,
  accent,
  advanced,
}: {
  def: BitDef;
  param: BitParam;
  cur: Fx;
  patch: (p: Record<string, unknown>) => void;
  accent: string;
  advanced: boolean;
}) {
  const name = `${def.title} ${param.label.toLowerCase()}`;
  const value = cur[param.key];
  const row = (control: React.ReactNode) => (
    <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5">
      <span className="text-xs text-white/70">{param.label}</span>
      {control}
    </div>
  );

  switch (param.kind) {
    case 'slider':
      return row(
        <AdvSlider
          label={name}
          {...param.range}
          value={value as number}
          onChange={(v) => patch({ [param.key]: v })}
          format={formatSlider(param)}
          advanced={advanced}
          unit={param.percent ? '%' : param.unit}
          toField={param.percent ? (v) => Math.round(v * 100) : undefined}
          fromField={param.percent ? (n) => n / 100 : undefined}
        />,
      );
    case 'color': {
      const c = value as string;
      return row(
        <span className="flex items-center gap-3">
          <button
            aria-label={`${name} match accent`}
            aria-pressed={c === 'accent'}
            onClick={() => patch({ [param.key]: 'accent' })}
            className={`rounded-full px-3 py-1 text-xs ${c === 'accent' ? 'bg-[var(--accent)] text-white' : 'bg-white/10 hover:bg-white/20'}`}
          >
            Match accent
          </button>
          <ColorPick
            label={name}
            value={isHexColor(c) ? c : accent}
            onChange={(hex) => patch({ [param.key]: hex })}
            advanced={advanced}
          />
        </span>,
      );
    }
    case 'bool':
      return row(
        <Toggle
          label={`${def.title}: ${param.label}`}
          checked={value as boolean}
          onChange={(v) => patch({ [param.key]: v })}
        />,
      );
    case 'choice':
      return row(
        <Segmented<string>
          label={name}
          value={value as string}
          onChange={(v) => patch({ [param.key]: v })}
          options={param.options}
        />,
      );
    case 'targets': {
      const chosen = value as TextTarget[];
      return row(
        <span role="group" aria-label={name} className="flex flex-wrap gap-1.5">
          {TEXT_TARGETS.map((t) => {
            const on = chosen.includes(t);
            return (
              <button
                key={t}
                aria-pressed={on}
                aria-label={`${def.title} on ${TEXT_TARGET_LABELS[t]}`}
                onClick={() =>
                  patch({ [param.key]: on ? chosen.filter((x) => x !== t) : [...chosen, t] })
                }
                className={`rounded-full border px-3 py-1 text-xs backdrop-blur-md ${
                  on
                    ? 'border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_25%,transparent)] text-white'
                    : 'border-white/15 bg-white/5 text-white/60 hover:bg-white/10'
                }`}
              >
                {TEXT_TARGET_LABELS[t]}
              </button>
            );
          })}
        </span>,
      );
    }
  }
}

/** One React Bits effect: its glass pill, and (once it is on) every way to tweak it. */
export function BitCard({
  def,
  data,
  set,
  advanced,
}: {
  def: BitDef;
  data: SettingsData;
  set: <S extends keyof SettingsData>(section: S, patch: Partial<SettingsData[S]>) => void;
  advanced: boolean;
}) {
  const cur = data.bits[def.id] as unknown as Fx;
  const patch = (p: Record<string, unknown>) =>
    set('bits', { [def.id]: { ...cur, ...p } } as Partial<SettingsData['bits']>);
  const accent = accentColor(data.appearance.accent);
  return (
    <div>
      <GlassPill label={def.title} on={cur.enabled} onChange={(enabled) => patch({ enabled })} />
      {cur.enabled && (
        <div className="mt-3 space-y-3 rounded-xl border border-white/10 bg-black/20 p-3">
          {def.params.map((param) => (
            <Param
              key={param.key}
              def={def}
              param={param}
              cur={cur}
              patch={patch}
              accent={accent}
              advanced={advanced}
            />
          ))}
        </div>
      )}
    </div>
  );
}
