import { useEffect, useMemo, useRef, useState } from 'react';
import type { CursorEngine, SettingsData } from '../settings/schema';
import { drawShape, shapeFor, toDataUri, type Shape } from './cursorShapes';

/** Should NovaOS draw the pointer itself right now? */
export function useNovaPointer(engine: CursorEngine, mode: string): boolean {
  return engine === 'nova' || (engine === 'auto' && mode === 'immersive');
}

/**
 * NovaOS's own mouse pointer. Fullscreen webviews can freeze or hide the system pointer
 * (especially custom image cursors), so in Immersive Mode NovaOS hides the native pointer and
 * draws this one, following every pointer move.
 *
 * Safety: the native pointer is only hidden (`onReady(true)`) after this one has actually
 * received a pointer position, so there is never a moment with no visible pointer at all.
 */
export function NovaCursor({
  active,
  cursor,
  onReady,
}: {
  active: boolean;
  cursor: SettingsData['cursor'];
  onReady: (ready: boolean) => void;
}) {
  const el = useRef<HTMLImageElement>(null);
  const pos = useRef({ x: 0, y: 0 });
  const [shape, setShape] = useState<Shape>('arrow');
  const [seen, setSeen] = useState(false);
  const seenRef = useRef(false);
  const [hidden, setHidden] = useState(false);

  const drawing = useMemo(
    () =>
      drawShape(shape, {
        color: cursor.color,
        outline: cursor.outline,
        size: cursor.size,
        style: cursor.style === 'system' ? 'classic' : cursor.style,
      }),
    [shape, cursor.color, cursor.outline, cursor.size, cursor.style],
  );
  const hotspot = useRef(drawing.hotspot);
  hotspot.current = drawing.hotspot;

  const place = () => {
    const node = el.current;
    if (!node) return;
    const [hx, hy] = hotspot.current;
    node.style.transform = `translate3d(${pos.current.x - hx}px, ${pos.current.y - hy}px, 0)`;
  };

  useEffect(() => {
    if (!active) {
      seenRef.current = false;
      setSeen(false);
      onReady(false);
      return;
    }
    const onMove = (e: PointerEvent) => {
      if (e.pointerType === 'touch') return; // touch has no pointer to draw
      pos.current = { x: e.clientX, y: e.clientY };
      setShape((cur) => {
        const next = shapeFor(e.target);
        return next === cur ? cur : next;
      });
      setHidden(false);
      if (!seenRef.current) {
        seenRef.current = true;
        setSeen(true);
        onReady(true); // only now is it safe to hide the native pointer
      }
      place();
    };
    // Only leaving the window hides it. Losing focus must not: the native pointer is hidden over an
    // unfocused window too, so hiding this one would leave no pointer at all.
    const onLeave = () => setHidden(true);
    window.addEventListener('pointermove', onMove, { capture: true, passive: true });
    window.addEventListener('pointerdown', onMove, { capture: true, passive: true });
    document.documentElement.addEventListener('mouseleave', onLeave);
    return () => {
      window.removeEventListener('pointermove', onMove, true);
      window.removeEventListener('pointerdown', onMove, true);
      document.documentElement.removeEventListener('mouseleave', onLeave);
      seenRef.current = false;
      setSeen(false);
      onReady(false);
    };
    // `onReady` is a stable setter from the parent.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  // Keep the image on the pointer when its shape or size changes under a still mouse.
  useEffect(place);

  if (!active || !seen) return null;
  return (
    <img
      ref={el}
      data-testid="nova-cursor"
      data-shape={shape}
      alt=""
      aria-hidden="true"
      draggable={false}
      src={toDataUri(drawing.svg)}
      className="pointer-events-none fixed top-0 left-0 select-none"
      style={{
        width: cursor.size,
        height: cursor.size,
        zIndex: 2147483000,
        display: hidden ? 'none' : 'block',
        willChange: 'transform',
      }}
    />
  );
}
