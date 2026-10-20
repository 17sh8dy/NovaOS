import { useState } from 'react';
import { evaluate, formatResult } from './evaluate';

const KEYS = [
  'C',
  '(',
  ')',
  '÷',
  '7',
  '8',
  '9',
  '×',
  '4',
  '5',
  '6',
  '−',
  '1',
  '2',
  '3',
  '+',
  '0',
  '.',
  '⌫',
  '=',
];

export function Calculator() {
  const [expr, setExpr] = useState('');
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState(false);

  const press = (k: string) => {
    setError(false);
    if (k === 'C') {
      setExpr('');
      setResult(null);
    } else if (k === '⌫') {
      setExpr((e) => e.slice(0, -1));
    } else if (k === '=') {
      try {
        const r = formatResult(evaluate(expr));
        setResult(r);
        setExpr(r);
      } catch {
        setError(true);
      }
    } else {
      setExpr((e) => (result !== null && /[\d.]/.test(k) && e === result ? k : e + k));
      setResult(null);
    }
  };

  return (
    <div className="flex h-full flex-col gap-2 p-3" data-testid="calculator">
      <div
        aria-label="display"
        className="min-h-14 rounded-lg bg-black/30 px-3 py-2 text-right font-mono text-2xl break-all"
      >
        {error ? <span className="text-rose-400">Error</span> : expr || '0'}
      </div>
      <div className="grid flex-1 grid-cols-4 gap-2">
        {KEYS.map((k) => (
          <button
            key={k}
            onClick={() => press(k)}
            className={`rounded-lg text-lg transition-colors ${
              k === '='
                ? 'bg-[var(--accent)] text-white hover:brightness-110'
                : /\d|\./.test(k)
                  ? 'bg-white/10 hover:bg-white/20'
                  : 'bg-white/5 hover:bg-white/15'
            }`}
          >
            {k}
          </button>
        ))}
      </div>
    </div>
  );
}
