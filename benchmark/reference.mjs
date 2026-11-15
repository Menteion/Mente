// Reference solutions of the public agent-benchmark tasks, in JavaScript.
//
// `npm run check:benchmark` runs every example and hidden test of a task
// through its solution here and compares the result with the value the task
// file states. Values follow check.mjs: integers are BigInt, floats are
// numbers, enum variants are { $: name, args? }, records are plain objects.
// Each solution takes the context first (ctx.net, ctx.clock, ctx.fs: the
// harness's stand-ins for the effects, README.md "Effects"), then the
// arguments; a `mut` argument is changed in place.

import { own, setOwn } from "./own.mjs";

const some = (v) => ({ $: "some", args: [v] });
const none = { $: "none" };
const ok = (v) => ({ $: "ok", args: [v] });
const err = (v) => ({ $: "err", args: [v] });
const tag = (name, ...args) => (args.length ? { $: name, args } : { $: name });

const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;
const inI64 = (v) => v >= I64_MIN && v <= I64_MAX;
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const utf8Length = (s) => BigInt(new TextEncoder().encode(s).length);

function deepEqual(a, b) {
  return JSON.stringify(a, (_, v) => (typeof v === "bigint" ? `${v}n` : v)) === JSON.stringify(b, (_, v) => (typeof v === "bigint" ? `${v}n` : v));
}

export const tasks = {
  // ---------------------------------------------------------------- M1

  gcd(_, a, b) {
    while (b !== 0n) [a, b] = [b, a % b];
    return a;
  },

  isqrt(_, n) {
    let x = BigInt(Math.floor(Math.sqrt(Number(n))));
    while (x * x > n) x--;
    while ((x + 1n) * (x + 1n) <= n) x++;
    return x;
  },

  "reverse-bits"(_, x) {
    let r = 0n;
    for (let i = 0n; i < 8n; i++) if ((x >> i) & 1n) r |= 1n << (7n - i);
    return r;
  },

  sort8(_, a) {
    return [...a].sort(cmp);
  },

  "reverse-in-place"(_, a) {
    a.reverse();
  },

  "max-subarray-sum"(_, a) {
    let best = own(a, 0), here = own(a, 0);
    for (const x of a.slice(1)) {
      here = here + x > x ? here + x : x;
      if (here > best) best = here;
    }
    return best;
  },

  "count-primes-below"(_, n) {
    const m = Number(n);
    if (m < 3) return 0n;
    const composite = new Array(m).fill(false);
    let count = 0n;
    for (let i = 2; i < m; i++) {
      if (own(composite, i)) continue;
      count++;
      for (let j = i * i; j < m; j += i) setOwn(composite, j, true);
    }
    return count;
  },

  mean8(_, a) {
    return a.reduce((s, x) => s + x, 0n) / 8n; // BigInt division truncates toward zero
  },

  "index-of-max"(_, a) {
    let best = 0;
    for (let i = 1; i < a.length; i++) if (own(a, i) > own(a, best)) best = i;
    return BigInt(best);
  },

  clamp(_, x, lo, hi) {
    return x < lo ? lo : x > hi ? hi : x;
  },

  percent(_, part, whole) {
    return (part * 100n) / whole;
  },

  "lower-bound"(_, a, x) {
    const i = a.findIndex((v) => v >= x);
    return BigInt(i < 0 ? a.length : i);
  },

  // The harder M1 tasks (docs/design-review.md, benchmark-difficulty).

  spiral(_, m) {
    const out = [];
    let [top, bottom, left, right] = [0, m.length - 1, 0, own(m, 0).length - 1];
    while (top <= bottom && left <= right) {
      for (let c = left; c <= right; c++) out.push(own(own(m, top), c));
      for (let r = top + 1; r <= bottom; r++) out.push(own(own(m, r), right));
      if (top < bottom) for (let c = right - 1; c >= left; c--) out.push(own(own(m, bottom), c));
      if (left < right) for (let r = bottom - 1; r > top; r--) out.push(own(own(m, r), left));
      [top, bottom, left, right] = [top + 1, bottom - 1, left + 1, right - 1];
    }
    return out;
  },

  "multiply-digits"(_, a, b) {
    const value = (ds) => ds.reduceRight((n, d) => n * 10n + d, 0n);
    let p = value(a) * value(b);
    const out = [];
    for (let k = 0; k < 12; k++) {
      out.push(p % 10n);
      p /= 10n;
    }
    return out;
  },

  "next-permutation"(_, a) {
    let i = a.length - 2;
    while (i >= 0 && own(a, i) >= own(a, i + 1)) i--;
    if (i < 0) {
      a.reverse();
      return false;
    }
    let j = a.length - 1;
    while (own(a, j) <= own(a, i)) j--;
    const swapped = own(a, i);
    setOwn(a, i, own(a, j));
    setOwn(a, j, swapped);
    const rest = a.splice(i + 1).reverse();
    a.push(...rest);
    return true;
  },

  bucket(_, x, lo, hi, n) {
    if (x < lo) return 0n;
    if (x > hi) return n - 1n;
    return ((x - lo) * n) / (hi - lo + 1n);
  },

  "merge-unique"(_, a, b, out) {
    const all = [...new Set([...a, ...b])].sort(cmp);
    all.forEach((v, k) => {
      setOwn(out, k, v);
    });
    return BigInt(all.length);
  },

  "day-of-year"(_, year, month, day) {
    const leap = (year % 4n === 0n && year % 100n !== 0n) || year % 400n === 0n;
    const lengths = [31n, leap ? 29n : 28n, 31n, 30n, 31n, 30n, 31n, 31n, 30n, 31n, 30n, 31n];
    return lengths.slice(0, Number(month) - 1).reduce((d, n) => d + n, day);
  },

  // ---------------------------------------------------------------- M2

  "parse-u32"(_, s) {
    if (s === "") return err(tag("Empty"));
    if (!/^[0-9]+$/.test(s)) return err(tag("InvalidDigit"));
    const v = BigInt(s);
    return v > 4294967295n ? err(tag("Overflow")) : ok(v);
  },

  "word-count"(_, s) {
    return BigInt(s.split(/[ \t\n\r]+/).filter((w) => w !== "").length);
  },

  "rpn-eval"(_, tokens) {
    const stack = [];
    for (const t of tokens) {
      if (/^-?[0-9]+$/.test(t)) {
        const v = BigInt(t);
        if (!inI64(v)) return err(tag("Overflow"));
        stack.push(v);
        continue;
      }
      if (!["+", "-", "*", "/"].includes(t)) return err(tag("BadToken"));
      if (stack.length < 2) return err(tag("StackUnderflow"));
      const b = stack.pop(), a = stack.pop();
      if (t === "/" && b === 0n) return err(tag("DivisionByZero"));
      const r = t === "+" ? a + b : t === "-" ? a - b : t === "*" ? a * b : a / b;
      if (!inI64(r)) return err(tag("Overflow"));
      stack.push(r);
    }
    if (stack.length === 0) return err(tag("StackUnderflow"));
    if (stack.length > 1) return err(tag("Leftover"));
    return ok(own(stack, 0));
  },

  "two-sum"(_, xs, target) {
    for (let i = 0; i < xs.length; i++) for (let j = i + 1; j < xs.length; j++) if (own(xs, i) + own(xs, j) === target) return some({ i: BigInt(i), j: BigInt(j) });
    return none;
  },

  "run-length-encode"(_, s) {
    let out = "";
    for (let i = 0; i < s.length; ) {
      let j = i;
      while (j < s.length && own(s, j) === own(s, i)) j++;
      out += own(s, i) + String(j - i);
      i = j;
    }
    return out;
  },

  "merge-sorted"(_, a, b) {
    const out = [];
    let i = 0, j = 0;
    while (i < a.length && j < b.length) out.push(own(a, i) <= own(b, j) ? own(a, i++) : own(b, j++));
    return [...out, ...a.slice(i), ...b.slice(j)];
  },

  "apply-ops"(_, balance, ops) {
    for (let k = 0; k < ops.length; k++) {
      const amount = own(own(ops, k).args, 0);
      if (amount <= 0n) return err(tag("NonPositiveAmount", BigInt(k)));
      if (own(ops, k).$ === "Deposit") balance += amount;
      else if (amount > balance) return err(tag("Overdraft", BigInt(k)));
      else balance -= amount;
    }
    return ok(balance);
  },

  "total-area2"(_, shapes) {
    let sum = 0n;
    for (const s of shapes) {
      const [a, b] = s.args;
      sum += s.$ === "Rect" ? 2n * a * b : s.$ === "Square" ? 2n * a * a : a * b;
    }
    return sum;
  },

  "binary-search-exact"(_, items, target) {
    const i = items.findIndex((v) => v === target);
    return i < 0 ? none : some(BigInt(i));
  },

  // ---------------------------------------------------------------- M3

  "decode-header"(_, b) {
    return {
      version: own(b, 0) >> 4n,
      flags: own(b, 0) & 15n,
      kind: own(b, 1),
      length: own(b, 2) | (own(b, 3) << 8n),
      id: own(b, 4) | (own(b, 5) << 8n) | (own(b, 6) << 16n) | (own(b, 7) << 24n),
    };
  },

  "encode-sample"(_, s) {
    const be = (v, n) => Array.from({ length: n }, (_, i) => (v >> BigInt(8 * (n - 1 - i))) & 255n);
    const value = s.value < 0n ? s.value + 2n ** 32n : s.value;
    return [...be(s.sensor, 2), ...be(value, 4), ...be(s.flags, 1), ...be(s.time, 4)];
  },

  "step-particles"(_, ps, dt) {
    const axis = (pos, vel) => {
      let p = pos + vel * dt, v = vel;
      if (p < 0n) { p = -p; v = -v; } else if (p > 1000n) { p = 2000n - p; v = -v; }
      return [p, v];
    };
    for (let k = 0; k < ps.length; k++) {
      const [x, vx] = axis(own(ps, k).x, own(ps, k).vx);
      const [y, vy] = axis(own(ps, k).y, own(ps, k).vy);
      setOwn(ps, k, { x, y, vx, vy });
    }
  },

  // ---------------------------------------------------------------- M4

  "fetch-lengths"(ctx, urls) {
    const out = [];
    for (const u of urls) {
      const r = ctx.net.get(u);
      if (r.$ === "err") return r;
      out.push(utf8Length(own(r.args, 0)));
    }
    return ok(out);
  },

  "retry-get"(ctx, url, attempts) {
    for (let k = 1n; ; k++) {
      const r = ctx.net.get(url);
      if (r.$ === "ok" || own(r.args, 0).$ === "NotFound" || k === attempts) return r;
      ctx.clock.sleep_ms(100n * 2n ** (k - 1n));
    }
  },

  "dedup-sorted"(_, xs) {
    const out = [];
    for (const x of xs) if (out.length === 0 || !deepEqual(own(out, out.length - 1), x)) out.push(x);
    return out;
  },

  // ---------------------------------------------------------------- M5

  stats(_, xs) {
    if (xs.length === 0) return none;
    const mean = xs.reduce((s, x) => s + x, 0) / xs.length;
    const variance = xs.reduce((s, x) => s + (x - mean) ** 2, 0) / xs.length;
    return some({ mean, stddev: Math.sqrt(variance) });
  },

  "q-mul"(_, a, b) {
    const p = a.$q * b.$q; // scale 2^32
    let r = p / 65536n; // truncates toward zero
    const rem = p % 65536n;
    if ((rem < 0n ? -rem : rem) * 2n >= 65536n) r += p < 0n ? -1n : 1n;
    const lo = -(2n ** 31n), hi = 2n ** 31n - 1n;
    return { $q: r < lo ? lo : r > hi ? hi : r };
  },

  "fuel-needed"(_, distance, consumption) {
    return (distance * consumption) / 100;
  },

  // ---------------------------------------------------------------- M6

  "count-lines"(ctx, p) {
    const r = ctx.fs.read_text(p);
    if (r.$ === "err") return r;
    const s = own(r.args, 0);
    if (s === "") return ok(0n);
    const newlines = BigInt(s.split("\n").length - 1);
    return ok(s.endsWith("\n") ? newlines : newlines + 1n);
  },

  "word-frequencies"(_, text) {
    const counts = new Map();
    for (const w of text.match(/[A-Za-z]+/g) ?? []) {
      const k = w.toLowerCase();
      counts.set(k, (counts.get(k) ?? 0n) + 1n);
    }
    return [...counts].sort((a, b) => cmp(own(b, 1), own(a, 1)) || cmp(own(a, 0), own(b, 0))).map(([word, count]) => ({ word, count }));
  },

  "csv-column-sum"(ctx, p, column) {
    const r = ctx.fs.read_text(p);
    if (r.$ === "err") return err(tag("Io", own(r.args, 0)));
    const lines = own(r.args, 0).split("\n");
    if (own(lines, lines.length - 1) === "") lines.pop();
    if (lines.length === 0) return err(tag("MissingColumn"));
    const idx = own(lines, 0).split(",").indexOf(column);
    if (idx < 0) return err(tag("MissingColumn"));
    let sum = 0n;
    for (let k = 1; k < lines.length; k++) {
      if (own(lines, k) === "") continue;
      const field = own(own(lines, k).split(","), idx);
      if (field === undefined || !/^-?[0-9]+$/.test(field)) return err(tag("BadNumber", BigInt(k + 1)));
      sum += BigInt(field);
      if (!inI64(BigInt(field)) || !inI64(sum)) return err(tag("Overflow"));
    }
    return ok(sum);
  },

  // ---------------------------------------------------------------- M7

  "fetch-all-concurrently"(ctx, urls) {
    return urls.map((u) => ctx.net.get(u));
  },

  "first-ok"(ctx, urls, timeout) {
    const arrivals = urls.map((u, i) => ({ ...ctx.net.getTimed(u), i }));
    const oks = arrivals.filter((a) => a.r.$ === "ok").sort((a, b) => cmp(a.after, b.after) || a.i - b.i);
    if (oks.length && own(oks, 0).after <= timeout) return ok(own(own(oks, 0).r.args, 0));
    const lastFailure = arrivals.reduce((m, a) => (a.after > m ? a.after : m), 0n);
    if (oks.length === 0 && lastFailure <= timeout) return err(tag("AllFailed"));
    return err(tag("TimedOut"));
  },

  "rate-limited-fetch"(ctx, urls) {
    return urls.map((u) => ctx.net.get(u));
  },
};
