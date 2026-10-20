import { describe, expect, it } from 'vitest';
import { evaluate, formatResult } from './evaluate';

describe('calculator evaluate', () => {
  it('respects precedence and parentheses', () => {
    expect(evaluate('2+3*4')).toBe(14);
    expect(evaluate('(2+3)*4')).toBe(20);
    expect(evaluate('2^3^2')).toBe(512);
    expect(evaluate('-3+5')).toBe(2);
    expect(evaluate('10%3')).toBe(1);
  });
  it('handles decimals and display glyphs', () => {
    expect(formatResult(evaluate('0.1+0.2'))).toBe('0.3');
    expect(evaluate('6×7÷2')).toBe(21);
  });
  it('rejects bad input instead of executing it', () => {
    expect(() => evaluate('')).toThrow();
    expect(() => evaluate('1/0')).toThrow('divide by zero');
    expect(() => evaluate('2+')).toThrow();
    expect(() => evaluate('(1+2')).toThrow();
    expect(() => evaluate('alert(1)')).toThrow();
    expect(() => evaluate('1;2')).toThrow();
  });
});
