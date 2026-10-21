import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { BitsDom } from './BitsDom';
import { BitsOverlay } from './BitsOverlay';
import { ElasticMesh } from './ElasticMesh';
import { FxText } from './FxText';
import { bitsAttr, bitsVars, useActiveBits, useListFx } from './useBits';
import { BITS_DEFAULTS, sanitizeBits, type BitsData } from '../../settings/bits';
import { DEFAULTS, patchSection } from '../../settings/schema';
import { useSettings } from '../../settings/store';

const withBits = (patch: Record<string, Record<string, unknown>>): BitsData => {
  const out = sanitizeBits(BITS_DEFAULTS);
  for (const [k, v] of Object.entries(patch)) {
    (out as unknown as Record<string, unknown>)[k] = {
      ...(out as unknown as Record<string, object>)[k],
      ...v,
    };
  }
  return out;
};

const setBits = (patch: Record<string, Record<string, unknown>>, a11y = {}) => {
  let d = patchSection(DEFAULTS, 'bits', withBits(patch));
  d = patchSection(d, 'accessibility', a11y);
  useSettings.setState({ data: d, loaded: true });
};

beforeEach(() => {
  useSettings.setState({ data: DEFAULTS, loaded: true });
});

const move = (x: number, y: number) => {
  act(() => {
    fireEvent(window, new MouseEvent('pointermove', { clientX: x, clientY: y, bubbles: true }));
  });
};
const frames = () => act(() => new Promise<void>((r) => setTimeout(r, 40)));

const rect = (left: number, top: number, w: number, h: number) =>
  ({
    left,
    top,
    right: left + w,
    bottom: top + h,
    width: w,
    height: h,
    x: left,
    y: top,
    toJSON: () => ({}),
  }) as DOMRect;

function box(tokens: string, r: DOMRect) {
  const el = document.createElement('div');
  el.dataset.bit = tokens;
  el.getBoundingClientRect = () => r;
  document.body.appendChild(el);
  return el;
}

describe('FxText', () => {
  it('with every effect off it renders only the text', () => {
    const { container } = render(<FxText where="icons">Notes</FxText>);
    expect(container.innerHTML).toBe('Notes');
  });

  it('Shiny Text wraps it only where it was asked to', () => {
    setBits({ shinyText: { enabled: true, targets: ['titles'] } });
    const { container, rerender } = render(<FxText where="titles">Notes</FxText>);
    expect(container.querySelector('.bit-shiny')).toHaveTextContent('Notes');
    rerender(<FxText where="icons">Notes</FxText>);
    expect(container.innerHTML).toBe('Notes');
  });

  it('Echo Text adds the echo class, but yields to Shiny Text on the same text', () => {
    setBits({ echoText: { enabled: true, targets: ['clock'] } });
    const { container, rerender } = render(<FxText where="clock">12:00</FxText>);
    expect(container.querySelector('.bit-echo')).not.toBeNull();
    setBits({
      echoText: { enabled: true, targets: ['clock'] },
      shinyText: { enabled: true, targets: ['clock'] },
    });
    rerender(<FxText where="clock">12:00</FxText>);
    expect(container.querySelector('.bit-shiny')).not.toBeNull();
    expect(container.querySelector('.bit-echo')).toBeNull();
  });

  it('Fold Text splits into letters but still reads as the whole word to a screen reader', () => {
    setBits({ foldText: { enabled: true, targets: ['icons'] } });
    const { container } = render(<FxText where="icons">Hi you</FxText>);
    expect(container.querySelectorAll('.bit-fold-l')).toHaveLength(6);
    expect(container.querySelector('.sr-only')).toHaveTextContent('Hi you');
    expect(container.querySelector('[aria-hidden="true"]')).not.toBeNull();
    const letters = [...container.querySelectorAll('.bit-fold-l')].map((l) => l.textContent);
    expect(letters.join('')).toBe('Hi you');
  });

  it('Reduce motion and High contrast turn the effects off', () => {
    setBits({ foldText: { enabled: true, targets: ['icons'] } }, { reduceMotion: true });
    expect(render(<FxText where="icons">Notes</FxText>).container.innerHTML).toBe('Notes');
    setBits({ shinyText: { enabled: true, targets: ['icons'] } }, { highContrast: true });
    expect(render(<FxText where="icons">Notes</FxText>).container.innerHTML).toBe('Notes');
  });
});

describe('animated lists', () => {
  it('give nothing when off, and a staggered class when on', () => {
    const off = renderHook(() => useListFx()).result.current;
    expect(off(3)).toEqual({});
    setBits({ animatedList: { enabled: true, style: 'pop' } });
    const on = renderHook(() => useListFx()).result.current;
    const fx = on(3);
    expect(fx.className).toBe('bit-li bit-li-pop');
    expect((fx.style as Record<string, number>)['--i']).toBe(3);
  });

  it('caps the delay so a long list never waits forever', () => {
    setBits({ animatedList: { enabled: true } });
    const fx = renderHook(() => useListFx()).result.current(5000);
    expect((fx.style as Record<string, number>)['--i']).toBeLessThanOrEqual(14);
  });
});

describe('desktop switches', () => {
  it('only the effects that are on show up as words on the desktop', () => {
    expect(bitsAttr(BITS_DEFAULTS)).toBe('');
    expect(
      bitsAttr(withBits({ chromaGrid: { enabled: true }, borderGlow: { enabled: true } })),
    ).toBe('chroma glow');
  });

  it('turns tweaks into CSS variables, with Accent resolving to the theme color', () => {
    const v = bitsVars(
      withBits({
        magicBento: { color: '#ff0000', radius: 300 },
        echoText: { color: 'accent', spread: 5 },
        animatedList: { stagger: 90 },
      }),
    ) as Record<string, string | number>;
    expect(v['--bit-bento']).toBe('#ff0000');
    expect(v['--bit-bento-r']).toBe('300px');
    expect(v['--bit-echo']).toBe('var(--accent)');
    expect(v['--bit-echo-d']).toBe('5px');
    expect(v['--bit-list-stagger']).toBe('90ms');
  });

  it('useActiveBits reflects accessibility settings live', () => {
    setBits({ clickSpark: { enabled: true } });
    const { result } = renderHook(() => useActiveBits());
    expect(result.current.clickSpark.enabled).toBe(true);
    act(() => setBits({ clickSpark: { enabled: true } }, { reduceMotion: true }));
    expect(result.current.clickSpark.enabled).toBe(false);
    expect(useSettings.getState().data.bits.clickSpark.enabled).toBe(true); // saved setting untouched
  });
});

describe('BitsDom (Magnet, Dock, Chroma, Bento, Border Glow)', () => {
  it('does nothing to elements while every effect is off', async () => {
    const el = box('magnet dock chroma bento glow', rect(100, 100, 40, 40));
    render(<BitsDom bits={BITS_DEFAULTS} />);
    move(120, 120);
    await frames();
    expect(el.getAttribute('style')).toBeNull();
    el.remove();
  });

  it('Magnet leans an element toward the pointer, and puts it back when it is out of reach', async () => {
    const el = box('magnet', rect(100, 100, 40, 40)); // center (120,120)
    render(<BitsDom bits={withBits({ magnet: { enabled: true, strength: 0.5, radius: 150 } })} />);
    move(170, 120);
    await waitFor(() => expect(el.style.translate).not.toBe(''));
    expect(parseFloat(el.style.translate)).toBeGreaterThan(0);
    move(900, 900);
    await waitFor(() => expect(el.style.translate).toBe(''));
    el.remove();
  });

  it('Dock grows the element nearest the pointer more than a far one', async () => {
    const near = box('dock', rect(100, 100, 40, 40));
    const far = box('dock', rect(400, 100, 40, 40));
    render(<BitsDom bits={withBits({ dock: { enabled: true, magnify: 1.6, radius: 200 } })} />);
    move(120, 120);
    await waitFor(() => expect(near.style.scale).not.toBe(''));
    expect(parseFloat(near.style.scale)).toBeGreaterThan(1.5);
    expect(parseFloat(far.style.scale)).toBe(1);
    expect(near.style.zIndex).toBe('5');
    near.remove();
    far.remove();
  });

  it('Chroma sets how much color an element shows from its distance', async () => {
    const el = box('chroma', rect(100, 100, 40, 40));
    render(<BitsDom bits={withBits({ chromaGrid: { enabled: true, radius: 200 } })} />);
    move(120, 120);
    await waitFor(() => expect(el.style.getPropertyValue('--chroma')).toBe('1.000'));
    move(320, 120);
    await waitFor(() => expect(el.style.getPropertyValue('--chroma')).toBe('0.000'));
    el.remove();
  });

  it('Magic Bento lights a card and tilts it only while the pointer is over it', async () => {
    const el = box('bento', rect(100, 100, 200, 100));
    render(<BitsDom bits={withBits({ magicBento: { enabled: true, tilt: true, radius: 200 } })} />);
    move(250, 120);
    await waitFor(() => expect(el.style.getPropertyValue('--mx')).toBe('150px'));
    expect(el.style.getPropertyValue('--my')).toBe('20px');
    expect(parseFloat(el.style.getPropertyValue('--glow'))).toBe(1);
    expect(el.style.transform).toContain('rotateY');
    move(380, 120); // outside the card, still within reach
    await waitFor(() => expect(el.style.transform).toBe(''));
    expect(parseFloat(el.style.getPropertyValue('--glow'))).toBeGreaterThan(0);
    el.remove();
  });

  it('Magic Bento does not tilt when tilt is off', async () => {
    const el = box('bento', rect(100, 100, 200, 100));
    render(<BitsDom bits={withBits({ magicBento: { enabled: true, tilt: false } })} />);
    move(250, 120);
    await waitFor(() => expect(el.style.getPropertyValue('--glow')).not.toBe(''));
    expect(el.style.transform).toBe('');
    el.remove();
  });

  it('Border Glow follows the pointer around a window edge', async () => {
    const el = box('glow', rect(100, 100, 400, 300));
    render(<BitsDom bits={withBits({ borderGlow: { enabled: true, radius: 100 } })} />);
    move(90, 250);
    await waitFor(() => expect(el.style.getPropertyValue('--gx')).toBe('-10px'));
    expect(parseFloat(el.style.getPropertyValue('--gglow'))).toBeGreaterThan(0.5);
    move(800, 800);
    await waitFor(() => expect(el.style.getPropertyValue('--gglow')).toBe('0.000'));
    el.remove();
  });

  it('ignores elements that did not opt in to that effect', async () => {
    const el = box('chroma', rect(100, 100, 40, 40));
    render(<BitsDom bits={withBits({ magnet: { enabled: true } })} />);
    move(120, 120);
    await frames();
    expect(el.style.translate).toBe('');
    el.remove();
  });

  it('removes everything it did when the effect is turned off', async () => {
    const el = box('magnet dock chroma', rect(100, 100, 40, 40));
    const { rerender } = render(
      <BitsDom
        bits={withBits({
          magnet: { enabled: true },
          dock: { enabled: true },
          chromaGrid: { enabled: true },
        })}
      />,
    );
    move(130, 120);
    await waitFor(() => expect(el.style.scale).not.toBe(''));
    rerender(<BitsDom bits={BITS_DEFAULTS} />);
    expect(el.getAttribute('style') ?? '').toBe('');
    el.remove();
  });

  it('ignores touch', async () => {
    const el = box('dock', rect(100, 100, 40, 40));
    render(<BitsDom bits={withBits({ dock: { enabled: true } })} />);
    act(() => {
      const e = new MouseEvent('pointermove', { clientX: 120, clientY: 120, bubbles: true });
      Object.defineProperty(e, 'pointerType', { value: 'touch' });
      fireEvent(window, e);
    });
    await frames();
    expect(el.style.scale).toBe('');
    el.remove();
  });
});

describe('canvas effects', () => {
  it('draw nothing and add nothing while everything is off', () => {
    const { container } = render(
      <>
        <BitsOverlay bits={BITS_DEFAULTS} />
        <ElasticMesh config={BITS_DEFAULTS.elasticMesh} />
      </>,
    );
    expect(container.querySelector('canvas')).toBeNull();
  });

  it('are click-through, so they can never block the desktop', () => {
    render(
      <>
        <BitsOverlay bits={withBits({ glowCursor: { enabled: true } })} />
        <ElasticMesh config={{ ...BITS_DEFAULTS.elasticMesh, enabled: true }} />
      </>,
    );
    for (const id of ['bits-overlay', 'elastic-mesh']) {
      const c = screen.getByTestId(id);
      expect(c).toHaveClass('pointer-events-none');
      expect(c).toHaveAttribute('aria-hidden', 'true');
    }
  });

  it('sit below NovaOS own pointer', () => {
    render(<BitsOverlay bits={withBits({ clickSpark: { enabled: true } })} />);
    expect(Number(screen.getByTestId('bits-overlay').style.zIndex)).toBeLessThan(2147483000);
  });

  it('draw through the 2D context when there is one', async () => {
    const calls: string[] = [];
    const ctx = new Proxy(
      {},
      {
        get: (_t, p: string) => {
          if (p === 'createRadialGradient') return () => ({ addColorStop: vi.fn() });
          return (...a: unknown[]) => {
            calls.push(p);
            return a;
          };
        },
        set: () => true,
      },
    );
    const spy = vi
      .spyOn(HTMLCanvasElement.prototype, 'getContext')
      .mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
    render(
      <BitsOverlay
        bits={withBits({ glowCursor: { enabled: true }, clickSpark: { enabled: true } })}
      />,
    );
    move(100, 100);
    act(() => {
      fireEvent(
        window,
        new MouseEvent('pointerdown', { clientX: 100, clientY: 100, bubbles: true }),
      );
    });
    await waitFor(() => expect(calls).toContain('fillRect'));
    await waitFor(() => expect(calls).toContain('stroke'));
    spy.mockRestore();
  });
});
