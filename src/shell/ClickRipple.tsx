import { useEffect, useState } from 'react';

interface Ripple {
  id: number;
  x: number;
  y: number;
}

/** A soft expanding ring at every click, in a color and speed chosen in Nova Studio. */
export function ClickRipple({ enabled, speed }: { enabled: boolean; speed: number }) {
  const [ripples, setRipples] = useState<Ripple[]>([]);

  useEffect(() => {
    if (!enabled) {
      setRipples([]);
      return;
    }
    let n = 0;
    const timers = new Set<number>();
    const onDown = (e: PointerEvent) => {
      const id = ++n;
      setRipples((r) => [...r.slice(-5), { id, x: e.clientX, y: e.clientY }]);
      const t = window.setTimeout(
        () => {
          setRipples((r) => r.filter((x) => x.id !== id));
          timers.delete(t);
        },
        900 / Math.max(0.5, speed),
      );
      timers.add(t);
    };
    window.addEventListener('pointerdown', onDown, true);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      timers.forEach((t) => window.clearTimeout(t));
    };
  }, [enabled, speed]);

  if (!enabled) return null;
  return (
    <>
      {ripples.map((r) => (
        <span
          key={r.id}
          data-testid="click-ripple"
          aria-hidden="true"
          className="pointer-events-none fixed z-[300000] rounded-full"
          style={{
            left: r.x - 22,
            top: r.y - 22,
            width: 44,
            height: 44,
            border: '2px solid var(--ripple)',
            background: 'color-mix(in srgb, var(--ripple) 18%, transparent)',
            animation: `nova-ripple ${0.7 / Math.max(0.5, speed)}s ease-out forwards`,
          }}
        />
      ))}
    </>
  );
}
