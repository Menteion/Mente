// Checks the agent-benchmark corpus that README.md describes:
//   - every task file's front matter, sections and marks;
//   - the corpus's composition against README.md's "Composition" table;
//   - every example and hidden test against the task's signature, and against
//     the task's reference solution in reference.mjs;
//   - corpus.lock, the digest of every public task and the commit of the
//     held-out set.
//
// Usage:
//   node benchmark/check.mjs                      check the public corpus
//   node benchmark/check.mjs --held-out <dir>     check the held-out set too
//   node benchmark/check.mjs --bless [...]        rewrite corpus.lock
//
// The file is also a module: the functions it exports parse and print the
// neutral values and run a test line through a reference solution.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { execFileSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOCK = path.join(HERE, "corpus.lock");
const HELD_OUT_REPO = "https://github.com/Menteion/mente-bench-heldout";
const EFFECTS = new Set(["Net", "Clock", "Fs", "FFI"]);
const BOUNDS = new Set(["Eq", "Ord"]);

// ---------------------------------------------------------------- tokens

const PUNCT = ["..=", "..", "->", "=>", "==", "<=", ">=", "!=", "(", ")", "[", "]", "{", "}", "<", ">", ",", ":", ".", "=", "+", "-", "*", "/", "%"];

export function tokenize(text) {
  const out = [];
  let i = 0;
  const valueEnd = () => {
    const last = out[out.length - 1];
    return last && (last.k === "num" || (last.k === "ident" && last.v !== "in") || last.k === "str" || last.v === ")" || last.v === "]");
  };
  while (i < text.length) {
    const c = text[i];
    if (c === " " || c === "\t") { i++; continue; }
    if (c === '"') {
      let s = "";
      i++;
      for (;;) {
        if (i >= text.length) throw new Error(`unterminated string in: ${text}`);
        const d = text[i];
        if (d === '"') { i++; break; }
        if (d === "\n") throw new Error(`a line break inside a string in: ${text}`);
        if (d === "\\") {
          const e = text[i + 1];
          const map = { '"': '"', "\\": "\\", n: "\n", t: "\t", r: "\r" };
          if (!(e in map)) throw new Error(`unknown escape \\${e} in: ${text}`);
          s += map[e];
          i += 2;
          continue;
        }
        s += d;
        i++;
      }
      out.push({ k: "str", v: s });
      continue;
    }
    const num = /^-?[0-9][0-9_]*(\.[0-9]+)?([eE][+-]?[0-9]+)?/.exec(text.slice(i));
    if (num && (c !== "-" || !valueEnd())) {
      const raw = num[0].replaceAll("_", "");
      out.push(num[1] || num[2] ? { k: "num", v: Number(raw), float: true } : { k: "num", v: BigInt(raw) });
      i += num[0].length;
      continue;
    }
    const id = /^[A-Za-z_][A-Za-z0-9_]*/.exec(text.slice(i));
    if (id) { out.push({ k: "ident", v: id[0] }); i += id[0].length; continue; }
    const p = PUNCT.find((q) => text.startsWith(q, i));
    if (!p) throw new Error(`unexpected character '${c}' in: ${text}`);
    out.push({ k: "punct", v: p });
    i += p.length;
  }
  return out;
}

class Cursor {
  constructor(text) { this.text = text; this.t = tokenize(text); this.i = 0; }
  peek(o = 0) { return this.t[this.i + o]; }
  at(v, o = 0) { const t = this.peek(o); return t !== undefined && (t.k === "punct" || t.k === "ident") && t.v === v; }
  done() { return this.i >= this.t.length; }
  next() { const t = this.t[this.i++]; if (!t) throw new Error(`unexpected end of: ${this.text}`); return t; }
  eat(v) { if (this.at(v)) { this.i++; return true; } return false; }
  expect(v) { if (!this.eat(v)) throw new Error(`expected '${v}' at token ${this.i} of: ${this.text}`); }
  ident() { const t = this.next(); if (t.k !== "ident") throw new Error(`expected a name at token ${this.i - 1} of: ${this.text}`); return t.v; }
  int() { const t = this.next(); if (t.k !== "num" || t.float) throw new Error(`expected an integer at token ${this.i - 1} of: ${this.text}`); return t.v; }
  end() { if (!this.done()) throw new Error(`unexpected '${this.peek().v}' at token ${this.i} of: ${this.text}`); }
}

// ---------------------------------------------------------------- values
//
// Integers are BigInt; floats are numbers; bool; strings; arrays for [N]T,
// slice<T> and list<T>; null for (); { $: tag, args? } for none, some, ok,
// err and enum variants; plain objects for records; { $q: BigInt } for a
// fixed-point value's raw bits; { $approx: number, tol } for an expected float.

function parseValueAt(c) {
  const t = c.next();
  if (t.k === "num") return t.v;
  if (t.k === "str") return t.v;
  if (t.k === "punct" && t.v === "(") { c.expect(")"); return null; }
  if (t.k === "punct" && t.v === "[") {
    const xs = [];
    while (!c.eat("]")) { xs.push(parseValueAt(c)); if (!c.at("]")) c.expect(","); }
    return xs;
  }
  if (t.k === "punct" && t.v === "{") {
    const r = {};
    while (!c.eat("}")) {
      const k = c.ident();
      if (k in r) throw new Error(`field ${k} given twice in: ${c.text}`);
      c.expect(":");
      r[k] = parseValueAt(c);
      if (!c.at("}")) c.expect(",");
    }
    return r;
  }
  if (t.k === "ident") {
    if (t.v === "true") return true;
    if (t.v === "false") return false;
    if (t.v === "q" && c.at("(")) { c.expect("("); const raw = c.int(); c.expect(")"); return { $q: raw }; }
    if (t.v === "approx" && c.at("(")) {
      c.expect("(");
      const a = c.next(); c.expect(","); const b = c.next(); c.expect(")");
      if (a.k !== "num" || b.k !== "num") throw new Error(`approx takes two numbers in: ${c.text}`);
      return { $approx: Number(a.v), tol: Number(b.v) };
    }
    if (c.at("(")) {
      c.expect("(");
      const args = [];
      while (!c.eat(")")) { args.push(parseValueAt(c)); if (!c.at(")")) c.expect(","); }
      return { $: t.v, args };
    }
    return { $: t.v };
  }
  throw new Error(`unexpected '${t.v}' in: ${c.text}`);
}

export function parseValue(text) { const c = new Cursor(text); const v = parseValueAt(c); c.end(); return v; }

function quote(s) {
  return '"' + s.replaceAll("\\", "\\\\").replaceAll('"', '\\"').replaceAll("\n", "\\n").replaceAll("\t", "\\t").replaceAll("\r", "\\r") + '"';
}

function printFloat(n) {
  if (!Number.isFinite(n)) throw new Error(`not a finite float: ${n}`);
  const s = String(n);
  return /[.e]/.test(s) ? s : s + ".0";
}

// `tol`, when given, prints every float as approx(value, tol).
export function printValue(v, tol) {
  if (typeof v === "bigint") return v.toString();
  if (typeof v === "number") return tol === undefined ? printFloat(v) : `approx(${printFloat(Number(v.toPrecision(15)))}, ${tol})`;
  if (typeof v === "boolean") return String(v);
  if (typeof v === "string") return quote(v);
  if (v === null) return "()";
  if (Array.isArray(v)) return "[" + v.map((x) => printValue(x, tol)).join(", ") + "]";
  if ("$q" in v) return `q(${v.$q})`;
  if ("$approx" in v) return `approx(${printFloat(v.$approx)}, ${v.tol})`;
  if ("$" in v) return v.args === undefined ? v.$ : `${v.$}(${v.args.map((x) => printValue(x, tol)).join(", ")})`;
  return "{" + Object.entries(v).map(([k, x]) => `${k}: ${printValue(x, tol)}`).join(", ") + "}";
}

export function equal(actual, expected) {
  if (expected !== null && typeof expected === "object" && "$approx" in expected) {
    return typeof actual === "number" && Math.abs(actual - expected.$approx) <= expected.tol * Math.max(1, Math.abs(expected.$approx));
  }
  if (typeof expected !== typeof actual) return false;
  if (expected === null || typeof expected !== "object") return actual === expected;
  if (actual === null) return false;
  if (Array.isArray(expected)) return Array.isArray(actual) && actual.length === expected.length && expected.every((x, i) => equal(actual[i], x));
  if ("$q" in expected) return "$q" in actual && actual.$q === expected.$q;
  if ("$" in expected) {
    if (actual.$ !== expected.$) return false;
    const a = actual.args ?? [], e = expected.args ?? [];
    return a.length === e.length && e.every((x, i) => equal(a[i], x));
  }
  const ka = Object.keys(actual), ke = Object.keys(expected);
  return ka.length === ke.length && ke.every((k) => k in actual && equal(actual[k], expected[k]));
}

// ---------------------------------------------------------------- types

const INT_RE = /^(i|u)(8|16|32|64|128)$/;

function intType(name, inLayout) {
  if (name === "usize") return { k: "int", name, lo: 0n, hi: 2n ** 64n - 1n };
  const m = INT_RE.exec(name) ?? (inLayout ? /^(u)([1-9][0-9]?)$/.exec(name) : null);
  if (!m) return null;
  const bits = BigInt(m[2]);
  return m[1] === "u" ? { k: "int", name, lo: 0n, hi: 2n ** bits - 1n } : { k: "int", name, lo: -(2n ** (bits - 1n)), hi: 2n ** (bits - 1n) - 1n };
}

function parseBound(c) {
  const t = c.peek();
  if (t && t.k === "num") return c.int();
  if (c.at("len")) { c.expect("len"); c.expect("("); const n = c.ident(); c.expect(")"); return { len: n }; }
  return { name: c.ident() };
}

// Reads a unit expression after `in`, up to a ',', ')', '->', '{', 'uses' or the end at depth 0.
function parseUnit(c) {
  const parts = [];
  let depth = 0;
  while (!c.done()) {
    const t = c.peek();
    if (depth === 0 && (t.v === "," || t.v === ")" || t.v === "->" || t.v === "{" || t.v === "uses")) break;
    if (t.v === "(") depth++;
    if (t.v === ")") depth--;
    parts.push(String(c.next().v));
  }
  if (parts.length === 0) throw new Error(`empty unit in: ${c.text}`);
  return parts.join(" ");
}

function parseType(c, generics, inLayout = false) {
  let t;
  if (c.eat("[")) { const n = c.int(); c.expect("]"); t = { k: "array", n, elem: parseType(c, generics, inLayout) }; return t; }
  if (c.eat("(")) { c.expect(")"); return { k: "unit" }; }
  const name = c.ident();
  const it = intType(name, inLayout);
  if (it) {
    t = it;
    if (c.at("in")) {
      c.expect("in");
      const lo = parseBound(c);
      const incl = c.at("..=");
      if (!c.eat("..=")) c.expect("..");
      const hi = parseBound(c);
      t = { ...t, ref: { lo, hi, incl } };
    }
    return t;
  }
  if (name === "bool" || name === "string") return { k: name };
  if (name === "f64") return c.at("in") ? (c.expect("in"), { k: "f64", unit: parseUnit(c) }) : { k: "f64" };
  if (name === "fixed") { c.expect("<"); const i = c.int(); c.expect(","); const f = c.int(); c.expect(">"); return { k: "fixed", i, f }; }
  if (name === "slice" || name === "list" || name === "option") {
    c.expect("<"); const elem = parseType(c, generics); c.expect(">");
    t = { k: name, elem };
    if (name === "list" && c.at("layout")) { c.expect("layout"); t.layout = c.ident(); if (t.layout !== "soa") throw new Error(`unknown layout ${t.layout}`); }
    return t;
  }
  if (name === "result" || name === "map") {
    c.expect("<"); const a = parseType(c, generics); c.expect(","); const b = parseType(c, generics); c.expect(">");
    return name === "result" ? { k: "result", ok: a, err: b } : { k: "map", key: a, val: b };
  }
  if (generics.has(name)) return { k: "generic", name };
  if (!/^[A-Z]/.test(name)) throw new Error(`unknown type ${name} in: ${c.text}`);
  return { k: "named", name };
}

const BUILTIN_DECLS = {
  NetError: { kind: "enum", variants: { Timeout: [], NotFound: [], Refused: [] } },
  IoError: { kind: "enum", variants: { NotFound: [], PermissionDenied: [], InvalidUtf8: [] } },
};

// "## Types": record, enum and layout record declarations, each on one line or
// spread over lines up to its closing brace; `//` starts a comment.
export function parseDecls(lines) {
  const decls = {};
  const text = lines.map((l) => l.replace(/\/\/.*$/, "")).join(" ");
  const c = new Cursor(text);
  while (!c.done()) {
    const layout = c.eat("layout");
    const kind = c.ident();
    const name = c.ident();
    if (name in decls || name in BUILTIN_DECLS) throw new Error(`type ${name} declared twice`);
    if (kind === "record") {
      let spec = null;
      if (layout) {
        c.expect("(");
        const bytes = c.int(); c.expect("bytes"); c.expect(",");
        const order = c.ident(); c.expect("-"); c.expect("endian");
        if (order !== "little" && order !== "big") throw new Error(`unknown byte order ${order}`);
        const packed = c.eat(",") ? (c.expect("packed"), true) : false;
        c.expect(")");
        spec = { bytes, order, packed };
      }
      c.expect("{");
      const fields = {};
      while (!c.eat("}")) {
        const f = c.ident(); c.expect(":");
        fields[f] = parseType(c, new Set(), layout);
        c.eat(",");
      }
      decls[name] = { kind: "record", fields, layout: spec };
    } else if (kind === "enum") {
      if (layout) throw new Error("an enum has no layout");
      c.expect("{");
      const variants = {};
      while (!c.eat("}")) {
        const v = c.ident();
        const args = [];
        if (c.eat("(")) { while (!c.eat(")")) { args.push(parseType(c, new Set())); if (!c.at(")")) c.expect(","); } }
        variants[v] = args;
        if (!c.at("}")) c.expect(",");
      }
      decls[name] = { kind: "enum", variants };
    } else throw new Error(`unknown declaration ${kind}`);
  }
  return decls;
}

// "## Signature": `name<T: Eq>(mut a: T, ...) -> R uses Net, Clock`, then
// `requires e` and `ensures e` lines.
export function parseSignature(lines) {
  const [head, ...rest] = lines;
  const c = new Cursor(head);
  const name = c.ident();
  const generics = new Map();
  if (c.eat("<")) {
    while (!c.eat(">")) {
      const g = c.ident(); const bounds = [];
      if (c.eat(":")) { do { const b = c.ident(); if (!BOUNDS.has(b)) throw new Error(`unknown bound ${b}`); bounds.push(b); } while (c.eat("+")); }
      generics.set(g, bounds);
      if (!c.at(">")) c.expect(",");
    }
  }
  c.expect("(");
  const params = [];
  while (!c.eat(")")) {
    const mut = c.eat("mut");
    const p = c.ident(); c.expect(":");
    params.push({ name: p, mut, type: parseType(c, new Set(generics.keys())) });
    if (!c.at(")")) c.expect(",");
  }
  const ret = c.eat("->") ? parseType(c, new Set(generics.keys())) : { k: "unit" };
  const uses = [];
  if (c.eat("uses")) { do { const e = c.ident(); if (!EFFECTS.has(e)) throw new Error(`unknown effect ${e}`); uses.push(e); } while (c.eat(",")); }
  c.end();
  const clauses = rest.map((l) => {
    const m = /^(requires|ensures) (.+)$/.exec(l.trim());
    if (!m) throw new Error(`a signature line after the first must be requires or ensures: ${l}`);
    return { kind: m[1], expr: m[2] };
  });
  return { name, generics, params, ret, uses, clauses, text: lines.join("\n") };
}

function substitute(t, env) {
  if (t.k === "generic") return env.get(t.name) ?? t;
  const out = { ...t };
  for (const key of ["elem", "ok", "err", "key", "val"]) if (t[key]) out[key] = substitute(t[key], env);
  return out;
}

export function checkValue(v, t, decls, where) {
  const fail = (msg) => { throw new Error(`${where}: ${printValue(v)} ${msg}`); };
  switch (t.k) {
    case "int": {
      if (typeof v !== "bigint") fail(`is not an integer of ${t.name}`);
      if (v < t.lo || v > t.hi) fail(`is outside ${t.name}`);
      if (t.ref) {
        const { lo, hi, incl } = t.ref;
        if (typeof lo === "bigint" && v < lo) fail("is below the refinement");
        if (typeof hi === "bigint" && (incl ? v > hi : v >= hi)) fail("is above the refinement");
      }
      return;
    }
    case "bool": if (typeof v !== "boolean") fail("is not a bool"); return;
    case "string": if (typeof v !== "string") fail("is not a string"); return;
    case "f64": if (typeof v !== "number" && !(v && typeof v === "object" && "$approx" in v)) fail("is not an f64"); return;
    case "fixed": {
      if (!v || typeof v !== "object" || !("$q" in v)) fail("is not a fixed-point value q(raw)");
      const bits = t.i + t.f;
      if (v.$q < -(2n ** (bits - 1n)) || v.$q > 2n ** (bits - 1n) - 1n) fail("is outside the fixed-point type");
      return;
    }
    case "unit": if (v !== null) fail("is not ()"); return;
    case "array":
      if (!Array.isArray(v) || BigInt(v.length) !== t.n) fail(`is not an array of ${t.n}`);
      v.forEach((x, i) => checkValue(x, t.elem, decls, `${where}[${i}]`));
      return;
    case "slice": case "list":
      if (!Array.isArray(v)) fail(`is not a ${t.k}`);
      v.forEach((x, i) => checkValue(x, t.elem, decls, `${where}[${i}]`));
      return;
    case "option":
      if (!v || v.$ === undefined || !((v.$ === "none" && v.args === undefined) || (v.$ === "some" && v.args?.length === 1))) fail("is not none or some(v)");
      if (v.$ === "some") checkValue(v.args[0], t.elem, decls, `${where}.some`);
      return;
    case "result":
      if (!v || (v.$ !== "ok" && v.$ !== "err") || v.args?.length !== 1) fail("is not ok(v) or err(e)");
      checkValue(v.args[0], v.$ === "ok" ? t.ok : t.err, decls, `${where}.${v.$}`);
      return;
    case "named": {
      const d = decls[t.name] ?? BUILTIN_DECLS[t.name];
      if (!d) throw new Error(`${where}: type ${t.name} is not declared`);
      if (d.kind === "record") {
        if (!v || typeof v !== "object" || Array.isArray(v) || "$" in v) fail(`is not a ${t.name} record`);
        const want = Object.keys(d.fields), got = Object.keys(v);
        if (want.join() !== got.join()) fail(`does not have the fields ${want.join(", ")} in order`);
        for (const f of want) checkValue(v[f], d.fields[f], decls, `${where}.${f}`);
      } else {
        if (!v || typeof v !== "object" || !(v.$ in d.variants)) fail(`is not a variant of ${t.name}`);
        const args = v.args ?? [];
        const want = d.variants[v.$];
        if (args.length !== want.length || (want.length === 0 && v.args !== undefined)) fail(`does not match ${v.$}'s fields`);
        args.forEach((x, i) => checkValue(x, want[i], decls, `${where}.${v.$}`));
      }
      return;
    }
    case "generic": throw new Error(`${where}: generic type ${t.name} was not instantiated`);
    default: throw new Error(`${where}: values of ${t.k} are not supported`);
  }
}

// ---------------------------------------------------------------- test lines

export function parseCall(text) {
  const c = new Cursor(text);
  const name = c.ident();
  const typeArgs = [];
  if (c.eat("<")) { while (!c.eat(">")) { typeArgs.push(parseType(c, new Set())); if (!c.at(">")) c.expect(","); } }
  c.expect("(");
  const args = [];
  while (!c.eat(")")) { const mut = c.eat("mut"); args.push({ mut, value: parseValueAt(c) }); if (!c.at(")")) c.expect(","); }
  const call = { name, typeArgs, args, text };
  if (c.eat("==")) call.expect = parseValueAt(c);
  if (c.eat("leaves")) { call.leaves = []; do call.leaves.push(parseValueAt(c)); while (c.eat(",")); }
  c.end();
  return call;
}

// given Net.get("u") => r [after N ms], r ...   given Net.post("u") => r ...   given Fs.read_text("p") => r
function parseGiven(text) {
  const c = new Cursor(text);
  c.expect("given");
  const target = `${c.ident()}.${(c.expect("."), c.ident())}`;
  if (!["Net.get", "Net.post", "Fs.read_text"].includes(target)) throw new Error(`unknown given: ${text}`);
  c.expect("(");
  const key = c.next();
  if (key.k !== "str") throw new Error(`given takes a string key: ${text}`);
  c.expect(")"); c.expect("=>");
  const responses = [];
  do {
    const r = parseValueAt(c);
    let after = 0n;
    if (c.eat("after")) { after = c.int(); c.expect("ms"); }
    responses.push({ r, after });
  } while (c.eat(","));
  c.end();
  return { target, key: key.v, responses };
}

// then Net.get calls [..]   then Net.get calls in any order [..]   then Net.post calls [..]
// then Clock.sleep_ms calls [..]   then elapsed under N ms   then Net.peak_in_flight at most N
function parseThen(text) {
  const c = new Cursor(text);
  c.expect("then");
  if (c.eat("elapsed")) { c.expect("under"); const ms = c.int(); c.expect("ms"); c.end(); return { harness: true, text }; }
  const target = `${c.ident()}.${(c.expect("."), c.ident())}`;
  if (target === "Net.peak_in_flight") { c.expect("at"); c.expect("most"); c.int(); c.end(); return { harness: true, text }; }
  if (!["Net.get", "Net.post", "Clock.sleep_ms", "Fs.read_text"].includes(target)) throw new Error(`unknown then: ${text}`);
  c.expect("calls");
  const anyOrder = c.eat("in") ? (c.expect("any"), c.expect("order"), true) : false;
  const calls = parseValueAt(c);
  c.end();
  if (!Array.isArray(calls)) throw new Error(`then ... calls takes a list: ${text}`);
  return { target, anyOrder, calls, text };
}

// Groups a block's lines into tests: `given` lines, one call line, `then` lines.
export function parseTests(lines) {
  const tests = [];
  let givens = [];
  for (const raw of lines) {
    const line = raw.trim();
    if (line === "") continue;
    if (line.startsWith("given ")) { givens.push(parseGiven(line)); continue; }
    if (line.startsWith("then ")) {
      if (tests.length === 0 || givens.length) throw new Error(`a then line must follow a call: ${line}`);
      tests[tests.length - 1].thens.push(parseThen(line));
      continue;
    }
    tests.push({ givens, call: parseCall(line), thens: [] });
    givens = [];
  }
  if (givens.length) throw new Error("given lines at the end of a block, with no call after them");
  return tests;
}

// ---------------------------------------------------------------- running a reference

const NOT_FOUND = { $: "err", args: [{ $: "NotFound" }] };

export function makeContext(givens) {
  const queues = new Map();
  for (const g of givens) queues.set(`${g.target} ${g.key}`, [...g.responses]);
  const log = { "Net.get": [], "Net.post": [], "Clock.sleep_ms": [], "Fs.read_text": [] };
  let now = 0n;
  const take = (target, key, fallback) => {
    const q = queues.get(`${target} ${key}`);
    if (!q) return { r: structuredClone(fallback), after: 0n };
    return structuredClone(q.length > 1 ? q.shift() : q[0]);
  };
  return {
    log,
    net: {
      get(url) { log["Net.get"].push(url); return take("Net.get", url, NOT_FOUND).r; },
      getTimed(url) { log["Net.get"].push(url); return take("Net.get", url, NOT_FOUND); },
      post(url, body) { log["Net.post"].push({ url, body }); return take("Net.post", url, { $: "ok", args: [null] }).r; },
    },
    clock: {
      now_ms() { return now; },
      sleep_ms(ms) { log["Clock.sleep_ms"].push(ms); now += ms; },
    },
    fs: {
      read_text(p) { log["Fs.read_text"].push(p); return take("Fs.read_text", p, NOT_FOUND).r; },
    },
  };
}

// Runs one test through `fn`; returns the result, the arguments after the
// call (for `mut` parameters) and the context's call log.
export function runCall(fn, test) {
  const ctx = makeContext(test.givens);
  const args = test.call.args.map((a) => structuredClone(a.value));
  const result = fn(ctx, ...args);
  return { result: result === undefined ? null : result, args, log: ctx.log };
}

function sameMultiset(a, b) {
  if (a.length !== b.length) return false;
  const rest = [...b];
  return a.every((x) => { const i = rest.findIndex((y) => equal(x, y)); if (i < 0) return false; rest.splice(i, 1); return true; });
}

// ---------------------------------------------------------------- task files

const SOLVE_SECTIONS = ["Task", "Signature", "Examples", "Hidden tests"];
const ADVERSARIAL_SECTIONS = ["Task", "Signature", "Probe", "Caught when", "Missed when"];
const OPTIONAL_SECTIONS = ["Types"];

export function readTask(file) {
  const text = fs.readFileSync(file, "utf8").replaceAll("\r\n", "\n");
  const where = path.basename(file);
  const m = /^---\n([\s\S]*?)\n---\n/.exec(text);
  if (!m) throw new Error(`${where}: no front matter`);
  const meta = {};
  for (const line of m[1].split("\n")) {
    const kv = /^([a-z]+): (.+)$/.exec(line);
    if (!kv) throw new Error(`${where}: bad front matter line: ${line}`);
    meta[kv[1]] = kv[2];
  }
  const body = text.slice(m[0].length);
  const sections = {};
  let current = null;
  for (const line of body.split("\n")) {
    const h = /^## (.+)$/.exec(line);
    if (h) {
      current = h[1];
      if (current in sections) throw new Error(`${where}: section ${current} twice`);
      sections[current] = [];
      continue;
    }
    if (current) sections[current].push(line);
  }
  const block = (name) => (sections[name] ?? []).filter((l) => l.startsWith("    ")).map((l) => l.slice(4));
  return { file, where, text, meta, sections, block };
}

const has = (o, k) => Object.prototype.hasOwnProperty.call(o, k);

function usesNonCore(sig, decls) {
  if (sig.clauses.length) return true;
  const seen = new Set();
  const walk = (t) => {
    if (!t) return false;
    if (t.k === "int" && t.ref) return true;
    if (t.k === "f64" && t.unit) return true;
    if (t.k === "fixed" || t.layout) return true;
    if (t.k === "named" && !seen.has(t.name) && decls[t.name]) {
      seen.add(t.name);
      const d = decls[t.name];
      if (d.layout) return true;
      return Object.values(d.kind === "record" ? d.fields : d.variants).flat().some(walk);
    }
    return ["elem", "ok", "err", "key", "val"].some((k) => walk(t[k]));
  };
  return sig.params.some((p) => walk(p.type)) || walk(sig.ret);
}

// Checks one task file; returns { meta, tests run }.
export function checkTask(task, reference, expectedSet) {
  const { meta, where, sections } = task;
  const id = path.basename(task.file, ".md");
  for (const k of ["id", "milestone", "core", "set", "kind"]) if (!has(meta, k)) throw new Error(`${where}: front matter lacks ${k}`);
  for (const k of Object.keys(meta)) if (!["id", "milestone", "core", "set", "kind", "retired"].includes(k)) throw new Error(`${where}: unknown front matter key ${k}`);
  if (meta.id !== id) throw new Error(`${where}: id ${meta.id} does not match the file name`);
  if (!/^M[1-9]$/.test(meta.milestone)) throw new Error(`${where}: milestone must be M1 to M9`);
  if (meta.core !== "true" && meta.core !== "false") throw new Error(`${where}: core must be true or false`);
  if (meta.set !== expectedSet) throw new Error(`${where}: set must be ${expectedSet} here`);
  if (meta.kind !== "solve" && meta.kind !== "adversarial") throw new Error(`${where}: kind must be solve or adversarial`);
  if (meta.kind === "adversarial" && meta.set !== "public") throw new Error(`${where}: adversarial tasks are public`);
  const required = meta.kind === "solve" ? SOLVE_SECTIONS : ADVERSARIAL_SECTIONS;
  for (const s of required) if (!has(sections, s)) throw new Error(`${where}: missing section ## ${s}`);
  for (const s of Object.keys(sections)) if (!required.includes(s) && !OPTIONAL_SECTIONS.includes(s)) throw new Error(`${where}: unknown section ## ${s}`);
  if (sections.Task.join("\n").trim() === "") throw new Error(`${where}: the task has no text`);

  const decls = parseDecls(task.block("Types"));
  const sig = parseSignature(task.block("Signature"));
  if (sig.name.replaceAll("_", "-") !== id) throw new Error(`${where}: function ${sig.name} does not match the id`);
  const nonCore = usesNonCore(sig, decls);
  if ((meta.core === "true") === nonCore) {
    throw new Error(`${where}: marked core: ${meta.core}, but its signature ${nonCore ? "uses" : "does not use"} refinements, contracts, units, fixed-point or layouts`);
  }

  if (meta.kind === "adversarial") {
    for (const s of ["Probe", "Caught when", "Missed when"]) if (sections[s].join("").trim() === "") throw new Error(`${where}: ## ${s} is empty`);
    return { meta, tests: 0 };
  }

  const fn = reference[id];
  if (typeof fn !== "function") throw new Error(`${where}: no reference solution for ${id}`);
  let count = 0;
  for (const section of ["Examples", "Hidden tests"]) {
    const tests = parseTests(task.block(section));
    if (tests.length === 0) throw new Error(`${where}: ## ${section} has no tests`);
    for (const test of tests) {
      const { call } = test;
      const at = `${where}, ${section}: ${call.text}`;
      if (call.name !== sig.name) throw new Error(`${at}: calls ${call.name}, not ${sig.name}`);
      if (call.typeArgs.length !== sig.generics.size) throw new Error(`${at}: needs ${sig.generics.size} type arguments`);
      const env = new Map([...sig.generics.keys()].map((g, i) => [g, call.typeArgs[i]]));
      if (call.args.length !== sig.params.length) throw new Error(`${at}: takes ${sig.params.length} arguments`);
      sig.params.forEach((p, i) => {
        if (call.args[i].mut !== p.mut) throw new Error(`${at}: argument ${p.name} must ${p.mut ? "" : "not "}be marked mut`);
        checkValue(call.args[i].value, substitute(p.type, env), decls, `${at}, ${p.name}`);
      });
      const ret = substitute(sig.ret, env);
      if (ret.k === "unit" ? has(call, "expect") : !has(call, "expect")) throw new Error(`${at}: ${ret.k === "unit" ? "returns nothing, so takes no ==" : "needs == expected"}`);
      if (has(call, "expect")) checkValue(call.expect, ret, decls, `${at}, result`);
      const muts = sig.params.map((p, i) => [p, i]).filter(([p]) => p.mut);
      if ((call.leaves?.length ?? 0) !== muts.length) throw new Error(`${at}: needs leaves with ${muts.length} values`);
      muts.forEach(([p], j) => checkValue(call.leaves[j], substitute(p.type, env), decls, `${at}, leaves ${p.name}`));
      for (const g of test.givens) if (!sig.uses.includes(g.target.split(".")[0])) throw new Error(`${at}: given ${g.target}, but the function does not use ${g.target.split(".")[0]}`);

      const out = runCall(fn, test);
      if (has(call, "expect") && !equal(out.result, call.expect)) throw new Error(`${at}: the reference returns ${printValue(out.result)}`);
      muts.forEach(([p, i], j) => { if (!equal(out.args[i], call.leaves[j])) throw new Error(`${at}: the reference leaves ${p.name} = ${printValue(out.args[i])}`); });
      for (const th of test.thens) {
        if (th.harness) continue;
        const got = out.log[th.target];
        if (!(th.anyOrder ? sameMultiset(got, th.calls) : equal(got, th.calls))) throw new Error(`${at}: the reference makes ${th.target} calls ${printValue(got)}`);
      }
      count++;
    }
  }
  return { meta, tests: count };
}

// ---------------------------------------------------------------- composition and lock

function readComposition() {
  const text = fs.readFileSync(path.join(HERE, "README.md"), "utf8").replaceAll("\r\n", "\n");
  const sec = /\n## Composition\n([\s\S]*?)(\n## |$)/.exec(text);
  if (!sec) throw new Error("README.md has no ## Composition section");
  const rows = {};
  for (const line of sec[1].split("\n")) {
    const m = /^\| (M[1-9]) \| ([0-9]+) \| ([0-9]+) \| ([0-9]+) \|$/.exec(line);
    if (m) rows[m[1]] = { tuning: Number(m[2]), heldOut: Number(m[3]), adversarial: Number(m[4]) };
  }
  if (Object.keys(rows).length === 0) throw new Error("README.md's composition table has no rows");
  return rows;
}

function countBy(items, key) {
  const out = {};
  for (const x of items) out[key(x)] = (out[key(x)] ?? 0) + 1;
  return out;
}

function compareCounts(label, got, rows, field) {
  for (const ms of new Set([...Object.keys(rows), ...Object.keys(got)])) {
    const want = rows[ms]?.[field] ?? 0;
    if ((got[ms] ?? 0) !== want) throw new Error(`composition: ${label} has ${got[ms] ?? 0} tasks for ${ms}, README.md says ${want}`);
  }
}

const digest = (task) => "sha256:" + crypto.createHash("sha256").update(task.text, "utf8").digest("hex");
const coreMark = (meta) => (meta.core === "true" ? "core" : "non-core");

function readLock() {
  if (!fs.existsSync(LOCK)) return { tasks: [], heldOut: null, heldOutTasks: [] };
  const lock = { tasks: [], heldOut: null, heldOutTasks: [] };
  for (const raw of fs.readFileSync(LOCK, "utf8").replaceAll("\r\n", "\n").split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const f = line.split(" ");
    if (f[0] === "task" && f.length === 6) lock.tasks.push(line);
    else if (f[0] === "held-out" && f.length === 3) lock.heldOut = { repo: f[1], commit: f[2] };
    else if (f[0] === "held-out-task" && f.length === 3) lock.heldOutTasks.push(line);
    else throw new Error(`corpus.lock: bad line: ${line}`);
  }
  return lock;
}

function writeLock(taskLines, heldOut, heldOutLines) {
  const text = [
    "# The fixed agent-benchmark corpus: README.md, \"Changing the corpus\", says when it may change.",
    "# `npm run check:benchmark` recomputes every line; `npm run check:benchmark -- --bless` rewrites them.",
    "# task <id> <milestone> <core|non-core> <solve|adversarial> <sha256 of the file, LF line endings>",
    ...taskLines,
    "# The held-out set: its repository and commit, then one line per task, without its name.",
    `held-out ${heldOut.repo} ${heldOut.commit}`,
    ...heldOutLines,
    "",
  ].join("\n");
  fs.writeFileSync(LOCK, text);
}

function git(dir, ...args) { return execFileSync("git", ["-C", dir, ...args], { encoding: "utf8" }).trim(); }

async function loadTasks(dir, set) {
  const tdir = path.join(dir, "tasks");
  const files = fs.readdirSync(tdir).filter((f) => f.endsWith(".md")).sort();
  const reference = (await import(pathToFileURL(path.join(dir, "reference.mjs")).href)).tasks;
  const tasks = files.map((f) => readTask(path.join(tdir, f)));
  let tests = 0;
  for (const t of tasks) tests += checkTask(t, reference, set).tests;
  const ids = new Set(tasks.map((t) => t.meta.id));
  for (const k of Object.keys(reference)) if (!ids.has(k)) throw new Error(`${dir}: reference.mjs has a solution for ${k}, which is not a task`);
  return { tasks, tests };
}

async function main() {
  const argv = process.argv.slice(2);
  const bless = argv.includes("--bless");
  const hi = argv.indexOf("--held-out");
  const heldOutDir = hi >= 0 ? path.resolve(argv[hi + 1] ?? "") : null;
  if (hi >= 0 && !argv[hi + 1]) throw new Error("--held-out needs a directory");

  const rows = readComposition();
  const pub = await loadTasks(HERE, "public");
  const solve = pub.tasks.filter((t) => t.meta.kind === "solve");
  const adversarial = pub.tasks.filter((t) => t.meta.kind === "adversarial");
  compareCounts("the tuning set", countBy(solve, (t) => t.meta.milestone), rows, "tuning");
  compareCounts("the adversarial set", countBy(adversarial, (t) => t.meta.milestone), rows, "adversarial");

  const taskLines = pub.tasks.map((t) => `task ${t.meta.id} ${t.meta.milestone} ${coreMark(t.meta)} ${t.meta.kind} ${digest(t)}`);
  const lock = readLock();
  let heldOut = lock.heldOut;
  let heldOutLines = lock.heldOutTasks;
  let heldTests = 0;
  if (heldOutDir) {
    const held = await loadTasks(heldOutDir, "held-out");
    if (held.tasks.some((t) => t.meta.kind !== "solve")) throw new Error("the held-out set holds solve tasks only");
    heldTests = held.tests;
    const lines = held.tasks.map((t) => `held-out-task ${t.meta.milestone} ${coreMark(t.meta)}`).sort();
    const dirty = git(heldOutDir, "status", "--porcelain");
    if (dirty) throw new Error(`${heldOutDir}: uncommitted changes; commit them before checking or blessing`);
    const commit = git(heldOutDir, "rev-parse", "HEAD");
    if (bless) { heldOut = { repo: HELD_OUT_REPO, commit }; heldOutLines = lines; }
    else {
      if (!heldOut || heldOut.commit !== commit) throw new Error(`corpus.lock names held-out commit ${heldOut?.commit}, but ${heldOutDir} is at ${commit}`);
      if (lines.join("\n") !== [...heldOutLines].sort().join("\n")) throw new Error("corpus.lock's held-out tasks do not match the held-out set");
    }
  }
  if (!heldOut) throw new Error("corpus.lock names no held-out commit; bless it with --held-out <dir>");
  compareCounts("the held-out set", countBy(heldOutLines, (l) => l.split(" ")[1]), rows, "heldOut");

  if (bless) {
    writeLock(taskLines, heldOut, [...heldOutLines].sort());
    console.log("corpus.lock rewritten");
  } else {
    const want = new Set(taskLines), got = new Set(lock.tasks);
    const changed = [...want].filter((l) => !got.has(l)).map((l) => l.split(" ")[1]);
    const gone = [...got].filter((l) => !want.has(l)).map((l) => l.split(" ")[1]).filter((id) => !changed.includes(id));
    if (changed.length || gone.length) {
      throw new Error(`corpus.lock does not match the tasks (changed or new: ${changed.join(", ") || "none"}; removed: ${gone.join(", ") || "none"}). ` +
        "A change to the corpus follows README.md, \"Changing the corpus\"; then run `npm run check:benchmark -- --bless`.");
    }
  }
  console.log(`benchmark: ${pub.tasks.length} public tasks${heldOutDir ? " and the held-out set" : ""} checked; ${pub.tests + heldTests} tests agree with their references`);
}

// Unconditionally, however the file is started: a guard comparing the path it was started by with
// the module's own skips main() when the two spell one file differently, and the check passes.
// Nothing imports this file; its tests run it as a process.
main().catch((e) => { console.error(`check:benchmark: ${e.message}`); process.exit(1); });
