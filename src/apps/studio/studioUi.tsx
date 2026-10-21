import { useEffect, useState, type ReactNode } from 'react';
import { Star } from 'lucide-react';
import { isHexColor } from '../../settings/schema';
import { Slider } from '../settings/ui';

/** A control's row in Studio: label, hint, a favorite star, and the control itself. */
export function ControlRow({
  id,
  title,
  hint,
  advanced,
  favorite,
  onToggleFavorite,
  stacked,
  children,
}: {
  id: string;
  title: string;
  hint?: string;
  advanced?: boolean;
  favorite: boolean;
  onToggleFavorite: (id: string) => void;
  stacked?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      data-control={id}
      className={`flex gap-4 px-4 py-3 ${stacked ? 'flex-col' : 'items-center justify-between'}`}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm">
          {title}
          {advanced && (
            <span className="rounded bg-white/10 px-1.5 py-px text-[9px] tracking-wider text-white/60 uppercase">
              Advanced
            </span>
          )}
          <button
            aria-label={`${favorite ? 'Remove' : 'Add'} ${title} ${favorite ? 'from' : 'to'} favorites`}
            aria-pressed={favorite}
            title={favorite ? 'Remove from favorites' : 'Add to favorites (shown on Home)'}
            onClick={() => onToggleFavorite(id)}
            className={`grid h-5 w-5 place-items-center rounded ${
              favorite ? 'text-amber-300' : 'text-white/25 hover:text-white/70'
            }`}
          >
            <Star size={12} fill={favorite ? 'currentColor' : 'none'} />
          </button>
        </div>
        {hint && <div className="mt-0.5 text-xs text-white/50">{hint}</div>}
      </div>
      <div className={stacked ? '' : 'shrink-0'}>{children}</div>
    </div>
  );
}

/** A slider that also shows an exact number field in Advanced mode. */
export function AdvSlider(p: {
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  label: string;
  format: (v: number) => string;
  advanced: boolean;
  /** Number shown in the exact field (defaults to the raw value). */
  toField?: (v: number) => number;
  fromField?: (n: number) => number;
  unit?: string;
}) {
  const toField = p.toField ?? ((v: number) => v);
  const fromField = p.fromField ?? ((n: number) => n);
  const [text, setText] = useState(String(toField(p.value)));
  useEffect(() => setText(String(toField(p.value))), [p.value]); // eslint-disable-line react-hooks/exhaustive-deps

  // Typing never fights the slider: the number is applied when you press Enter or leave the box,
  // then the box shows the value that was actually accepted (clamped to the allowed range).
  const commit = () => {
    const n = Number(text);
    if (text.trim() !== '' && Number.isFinite(n)) p.onChange(fromField(n));
    else setText(String(toField(p.value)));
  };

  return (
    <div className="flex items-center gap-3">
      <Slider
        label={p.label}
        min={p.min}
        max={p.max}
        step={p.step}
        value={p.value}
        onChange={p.onChange}
        format={p.format}
      />
      {p.advanced && (
        <label className="flex items-center gap-1 text-[11px] text-white/50">
          <input
            aria-label={`${p.label} exact value`}
            inputMode="decimal"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
            }}
            onBlur={commit}
            className="w-14 rounded bg-black/30 px-1.5 py-1 text-right text-xs text-white outline-none"
          />
          {p.unit}
        </label>
      )}
    </div>
  );
}

/** A hex color text field: only a complete #rrggbb is accepted. */
export function HexField({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (hex: string) => void;
  label: string;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const ok = isHexColor(text);
  return (
    <input
      aria-label={label}
      value={text}
      spellCheck={false}
      maxLength={7}
      onChange={(e) => {
        // However it is typed or pasted, there is exactly one leading #.
        const v = `#${e.target.value.replace(/^#+/, '')}`;
        setText(v);
        if (isHexColor(v)) onChange(v.toLowerCase());
      }}
      onBlur={() => setText(value)}
      className={`w-24 rounded bg-black/30 px-2 py-1 font-mono text-xs outline-none ${
        ok ? '' : 'ring-1 ring-rose-500'
      }`}
    />
  );
}

/** A native color picker with an optional exact hex field. */
export function ColorPick({
  value,
  onChange,
  label,
  advanced,
}: {
  value: string;
  onChange: (hex: string) => void;
  label: string;
  advanced: boolean;
}) {
  return (
    <span className="flex items-center gap-2">
      <input
        type="color"
        aria-label={label}
        value={isHexColor(value) ? value : '#000000'}
        onChange={(e) => onChange(e.target.value)}
        className="h-7 w-10 cursor-pointer rounded border-0 bg-transparent p-0"
      />
      {advanced && <HexField label={`${label} hex`} value={value} onChange={onChange} />}
    </span>
  );
}
