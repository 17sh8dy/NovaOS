/**
 * Safe arithmetic evaluator (recursive descent). Deliberately not eval()/Function:
 * the input is untrusted text. Supports + - * / % ^ , parentheses, unary minus, decimals.
 */
export function evaluate(input: string): number {
  const src = input.replace(/\s+/g, '').replace(/×/g, '*').replace(/÷/g, '/').replace(/−/g, '-');
  if (!src) throw new Error('empty');
  let pos = 0;

  const peek = () => src[pos];
  const parseExpr = (): number => {
    let v = parseTerm();
    while (peek() === '+' || peek() === '-') {
      const op = src[pos++];
      const r = parseTerm();
      v = op === '+' ? v + r : v - r;
    }
    return v;
  };
  const parseTerm = (): number => {
    let v = parsePower();
    while (peek() === '*' || peek() === '/' || peek() === '%') {
      const op = src[pos++];
      const r = parsePower();
      if (op === '*') v *= r;
      else if (r === 0) throw new Error('divide by zero');
      else v = op === '/' ? v / r : v % r;
    }
    return v;
  };
  const parsePower = (): number => {
    const base = parseUnary();
    if (peek() === '^') {
      pos++;
      return Math.pow(base, parsePower()); // right-associative
    }
    return base;
  };
  const parseUnary = (): number => {
    if (peek() === '-') {
      pos++;
      return -parseUnary();
    }
    if (peek() === '+') {
      pos++;
      return parseUnary();
    }
    return parseAtom();
  };
  const parseAtom = (): number => {
    if (peek() === '(') {
      pos++;
      const v = parseExpr();
      if (peek() !== ')') throw new Error('missing )');
      pos++;
      return v;
    }
    const m = /^\d*\.?\d+(?:e[+-]?\d+)?|^\d+\.?/i.exec(src.slice(pos));
    if (!m) throw new Error('unexpected input');
    pos += m[0].length;
    return parseFloat(m[0]);
  };

  const result = parseExpr();
  if (pos !== src.length) throw new Error('unexpected input');
  if (!Number.isFinite(result)) throw new Error('not a finite number');
  return result;
}

/** Trim float noise: 0.1+0.2 -> 0.3 */
export function formatResult(n: number): string {
  return String(parseFloat(n.toPrecision(12)));
}
