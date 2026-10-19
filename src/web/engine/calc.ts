// Deterministic arithmetic for the `calculate` tool: a small recursive-descent evaluator. No eval, no
// Function constructor. Supports + - * / % ^, parentheses, constants and common functions.

const FUNCS: Record<string, (...a: number[]) => number> = {
  sqrt: Math.sqrt, cbrt: Math.cbrt, abs: Math.abs, sin: Math.sin, cos: Math.cos, tan: Math.tan, asin: Math.asin, acos: Math.acos, atan: Math.atan,
  ln: Math.log, log: Math.log10, log10: Math.log10, log2: Math.log2, exp: Math.exp, floor: Math.floor, ceil: Math.ceil, round: Math.round, trunc: Math.trunc,
  min: Math.min, max: Math.max, pow: Math.pow, sign: Math.sign,
};
const CONSTS: Record<string, number> = { pi: Math.PI, e: Math.E, tau: Math.PI * 2 };

export class CalcError extends Error {}

export function calculate(expression: string): number {
  const src = expression.replace(/×/g, '*').replace(/÷/g, '/').replace(/−/g, '-').replace(/,(?=\d{3}\b)/g, '').replace(/(\d)%\s*of\s*/gi, '$1/100*');
  if (src.length > 500) throw new CalcError('expression too long');
  let i = 0;
  const ws = () => {
    while (i < src.length && /\s/.test(src[i])) i++;
  };
  const peek = () => (ws(), src[i]);
  const num = (): number => {
    ws();
    const m = /^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/.exec(src.slice(i));
    if (!m) throw new CalcError(`unexpected "${src[i] ?? 'end of input'}"`);
    i += m[0].length;
    return parseFloat(m[0]);
  };
  const atom = (): number => {
    const c = peek();
    if (c === '(') {
      i++;
      const v = expr();
      if (peek() !== ')') throw new CalcError('missing ")"');
      i++;
      return v;
    }
    if (c === '-') {
      i++;
      return -power();
    }
    if (c === '+') {
      i++;
      return power();
    }
    if (c !== undefined && /[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i))![0];
      i += m.length;
      const name = m.toLowerCase();
      if (peek() === '(') {
        i++;
        const args: number[] = [];
        if (peek() !== ')') {
          for (;;) {
            args.push(expr());
            if (peek() === ',') i++;
            else break;
          }
        }
        if (peek() !== ')') throw new CalcError('missing ")"');
        i++;
        const f = FUNCS[name];
        if (!f) throw new CalcError(`unknown function "${m}"`);
        return f(...args);
      }
      if (name in CONSTS) return CONSTS[name];
      throw new CalcError(`unknown name "${m}"`);
    }
    return num();
  };
  const power = (): number => {
    const base = atom();
    if (peek() === '^' || (peek() === '*' && src[i + 1] === '*')) {
      i += peek() === '^' ? 1 : 2;
      return Math.pow(base, unaryPower());
    }
    return base;
  };
  const unaryPower = (): number => {
    if (peek() === '-') {
      i++;
      return -unaryPower();
    }
    return power();
  };
  const term = (): number => {
    let v = power();
    for (;;) {
      const c = peek();
      if (c === '*' && src[i + 1] !== '*') {
        i++;
        v *= power();
      } else if (c === '/') {
        i++;
        const d = power();
        if (d === 0) throw new CalcError('division by zero');
        v /= d;
      } else if (c === '%') {
        i++;
        const d = power();
        if (d === 0) throw new CalcError('division by zero');
        v %= d;
      } else return v;
    }
  };
  const expr = (): number => {
    let v = term();
    for (;;) {
      const c = peek();
      if (c === '+') {
        i++;
        v += term();
      } else if (c === '-') {
        i++;
        v -= term();
      } else return v;
    }
  };
  const result = expr();
  if (peek() !== undefined) throw new CalcError(`unexpected "${src[i]}"`);
  if (!Number.isFinite(result)) throw new CalcError('the result is not a finite number');
  return result;
}

export function formatNumber(v: number): string {
  if (Number.isInteger(v) && Math.abs(v) < 1e15) return String(v);
  const s = v.toPrecision(12);
  return String(parseFloat(s));
}
