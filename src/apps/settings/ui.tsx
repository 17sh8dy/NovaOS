import type { ReactNode } from 'react';

/** Shared building blocks so every settings section looks and behaves the same. */

export function Page({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="space-y-5">
      <header>
        <h2 className="text-base font-semibold">{title}</h2>
        {description && <p className="mt-0.5 text-xs text-white/50">{description}</p>}
      </header>
      {children}
    </section>
  );
}

export function Group({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <div>
      {title && (
        <h3 className="mb-1.5 text-[11px] font-semibold tracking-wider text-white/50 uppercase">
          {title}
        </h3>
      )}
      <div
        data-bit="bento"
        className="relative divide-y divide-white/10 rounded-xl border border-white/10 bg-white/[0.03]"
      >
        {children}
      </div>
    </div>
  );
}

export function Row({
  label,
  hint,
  children,
  stacked,
}: {
  label: string;
  hint?: string;
  children?: ReactNode;
  /** Put the control under the label (for wide controls like sliders and swatch grids). */
  stacked?: boolean;
}) {
  return (
    <div
      className={`flex gap-4 px-4 py-3 ${stacked ? 'flex-col' : 'items-center justify-between'}`}
    >
      <div className="min-w-0">
        <div className="text-sm">{label}</div>
        {hint && <div className="mt-0.5 text-xs text-white/50">{hint}</div>}
      </div>
      {children && <div className={stacked ? '' : 'shrink-0'}>{children}</div>}
    </div>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-5 w-9 rounded-full transition-colors ${
        checked ? 'bg-[var(--accent)]' : 'bg-white/20'
      }`}
    >
      <span
        className={`absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white transition-transform ${
          checked ? 'translate-x-4' : ''
        }`}
      />
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string; disabled?: boolean; title?: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex rounded-lg bg-black/30 p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={value === o.value}
          disabled={o.disabled}
          title={o.title}
          onClick={() => onChange(o.value)}
          className={`rounded-md px-3 py-1 text-xs disabled:cursor-not-allowed disabled:opacity-40 ${
            value === o.value ? 'bg-[var(--accent)] text-white' : 'text-white/70 hover:bg-white/10'
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Slider({
  value,
  min,
  max,
  step,
  onChange,
  label,
  format,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  label: string;
  format: (v: number) => string;
}) {
  return (
    <div className="flex items-center gap-3">
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-44 accent-[var(--accent)]"
      />
      <span className="w-12 text-right text-xs text-white/60 tabular-nums">{format(value)}</span>
    </div>
  );
}

export function Button({
  children,
  onClick,
  danger,
  disabled,
  label,
}: {
  children: ReactNode;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      className={`rounded-md px-3 py-1.5 text-xs disabled:opacity-40 ${
        danger ? 'bg-rose-500/80 hover:bg-rose-500' : 'bg-white/10 hover:bg-white/20'
      }`}
    >
      {children}
    </button>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-[11px]">{children}</kbd>;
}
