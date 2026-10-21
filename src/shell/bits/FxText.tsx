import type { TextTarget } from '../../settings/bits';
import { useActiveBits } from './useBits';

/**
 * Text that wears the Shiny / Echo / Fold effects when they are on for `where`. With every effect
 * off it renders just the text, so nothing changes in the page.
 */
export function FxText({ where, children }: { where: TextTarget; children: string }) {
  const { shinyText, echoText, foldText } = useActiveBits();
  const shiny = shinyText.enabled && shinyText.targets.includes(where);
  // Shiny Text paints the letters itself, so it replaces Echo where both are on.
  const echo = echoText.enabled && echoText.targets.includes(where) && !shiny;
  const fold = foldText.enabled && foldText.targets.includes(where);
  if (!shiny && !echo && !fold) return <>{children}</>;

  const cls = ['bit-fx', shiny && 'bit-shiny', echo && 'bit-echo', fold && 'bit-fold']
    .filter(Boolean)
    .join(' ');
  if (!fold) return <span className={cls}>{children}</span>;
  return (
    <span className={cls}>
      <span className="sr-only">{children}</span>
      <span aria-hidden="true">
        {[...children].map((ch, i) => (
          <span key={i} className="bit-fold-l" style={{ '--i': i } as React.CSSProperties}>
            {ch === ' ' ? ' ' : ch}
          </span>
        ))}
      </span>
    </span>
  );
}
