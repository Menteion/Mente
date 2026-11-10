// Writing a benchmark task in Rust and in Odin, and the program the harness builds around an
// agent's function (benchmark/README.md, "Signatures and types" and "Values and tests").
//
// A neutral type, signature, declaration or value is parsed once, then written in each language:
// the signature and types the agent is shown, the examples as code in its language, and the
// program that makes the call a case names and prints its result, then what each `mut` argument
// leaves, in the benchmark's canonical value form, exactly as `mn run` prints them. Mente's own
// writing stays in tasks.ts.
//
// Refinements become plain types and contracts become doc comments, as the README's table says;
// a refinement's range, a quantity's unit and a fixed-point number's bits go into the doc comment
// too, so that the agent is told what the neutral signature states. The programs cover the values
// M1's tasks use (integers, `bool` and arrays, nested or not, as arguments, `mut` arguments and
// results); a later value form throws until the harness writes it.

import type { Case } from "./tasks.ts";

export type Lang = "mente" | "rust" | "odin";

export const LANGUAGES: Lang[] = ["mente", "rust", "odin"];

export const INTEGERS = ["i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "usize"];

/** A refinement's range, `lo..hi` or `lo..=hi`; its bounds are neutral expressions. */
export interface Range {
  lo: string;
  hi: string;
  inclusive: boolean;
}

export type Type =
  | { kind: "int"; name: string; range?: Range }
  | { kind: "bool" }
  | { kind: "unit" }
  | { kind: "array"; len: number; elem: Type }
  | { kind: "string" }
  | { kind: "slice"; elem: Type }
  | { kind: "list"; elem: Type; soa: boolean }
  | { kind: "option"; elem: Type }
  | { kind: "result"; ok: Type; err: Type }
  | { kind: "f64"; unit?: string }
  | { kind: "fixed"; int: number; frac: number }
  | { kind: "named"; name: string };

export interface Param {
  name: string;
  mut: boolean;
  type: Type;
}

export interface Signature {
  name: string;
  generics: { name: string; bound: string }[];
  params: Param[];
  result: Type | null;
  effects: string[];
  /** The `requires` clauses, without the keyword. */
  requires: string[];
  /** The `ensures` clauses, without the keyword. */
  ensures: string[];
}

export type Decl =
  | { kind: "record"; name: string; fields: { name: string; type: Type }[] }
  | { kind: "enum"; name: string; variants: { name: string; fields: Type[] }[] }
  | { kind: "layout"; name: string; lines: string[] };

export type Value =
  | { kind: "int"; text: string }
  | { kind: "float"; text: string }
  | { kind: "bool"; value: boolean }
  | { kind: "string"; value: string }
  | { kind: "array"; items: Value[] }
  | { kind: "record"; fields: { name: string; value: Value }[] }
  | { kind: "variant"; name: string; args: Value[] }
  | { kind: "unit" };

export interface Arg {
  mut: boolean;
  value: Value;
}

export interface Call {
  name: string;
  typeArgs: string[];
  args: Arg[];
}

// ---------------------------------------------------------------------------------------------
// Reading the neutral notation

/** Splits `text` at the commas no bracket or string encloses, dropping empty parts. */
export function splitTop(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '"') {
      for (i++; i < text.length && text[i] !== '"'; i++) if (text[i] === "\\") i++;
    } else if ("([{<".includes(ch)) depth++;
    else if (")]}>".includes(ch)) depth--;
    else if (ch === "," && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map((p) => p.trim()).filter((p) => p !== "");
}

function parseRange(text: string): Range {
  const m = /^(.+?)(\.\.=?)(.+)$/.exec(text.trim());
  if (!m) throw new Error(`\`${text}\` is not a range`);
  return { lo: m[1].trim(), hi: m[3].trim(), inclusive: m[2] === "..=" };
}

/** Reads a neutral type. */
export function parseType(text: string): Type {
  const t = text.trim();
  const refined = /^(\w+) in (.+)$/.exec(t);
  if (refined && INTEGERS.includes(refined[1])) {
    return { kind: "int", name: refined[1], range: parseRange(refined[2]) };
  }
  if (refined && refined[1] === "f64") return { kind: "f64", unit: refined[2].trim() };
  const array = /^\[(\d+)\](.+)$/.exec(t);
  if (array) return { kind: "array", len: Number(array[1]), elem: parseType(array[2]) };
  if (t === "()") return { kind: "unit" };
  if (INTEGERS.includes(t)) return { kind: "int", name: t };
  if (t === "bool") return { kind: "bool" };
  if (t === "string") return { kind: "string" };
  if (t === "f64") return { kind: "f64" };
  const applied = /^(\w+)<(.*)>(\s+layout soa)?$/.exec(t);
  if (applied) {
    const args = splitTop(applied[2]);
    const one = (kind: string): Type => {
      if (args.length !== 1) throw new Error(`\`${t}\`: ${kind} takes one type`);
      return parseType(args[0]);
    };
    if (applied[3] && applied[1] !== "list") throw new Error(`\`${t}\`: only a list has a layout`);
    switch (applied[1]) {
      case "slice":
        return { kind: "slice", elem: one("slice") };
      case "list":
        return { kind: "list", elem: one("list"), soa: Boolean(applied[3]) };
      case "option":
        return { kind: "option", elem: one("option") };
      case "result":
        if (args.length !== 2) throw new Error(`\`${t}\`: result takes two types`);
        return { kind: "result", ok: parseType(args[0]), err: parseType(args[1]) };
      case "fixed":
        if (args.length !== 2 || !args.every((a) => /^\d+$/.test(a))) throw new Error(`\`${t}\`: fixed takes two bit counts`);
        return { kind: "fixed", int: Number(args[0]), frac: Number(args[1]) };
    }
  }
  if (/^[A-Z]\w*$/.test(t)) return { kind: "named", name: t };
  throw new Error(`\`${t}\` is not a neutral type`);
}

/** Reads a task's neutral signature: its header line, then its `requires` and `ensures` lines. */
export function parseSignature(lines: string[]): Signature {
  const head = (lines[0] ?? "").trim();
  const start = /^(\w+)(?:<([^>]*)>)?\(/.exec(head);
  if (!start) throw new Error(`not a signature: ${head}`);
  let i = start[0].length;
  for (let depth = 1; depth > 0; i++) {
    if (i >= head.length) throw new Error(`not a signature: ${head}`);
    if (head[i] === "(") depth++;
    else if (head[i] === ")") depth--;
  }
  const inner = head.slice(start[0].length, i - 1);
  let rest = head.slice(i).trim();
  let effects: string[] = [];
  const uses = /(?:^|\s)uses\s+(.+)$/.exec(rest);
  if (uses) {
    effects = uses[1].split(",").map((e) => e.trim());
    rest = rest.slice(0, uses.index).trim();
  }
  let result: Type | null = null;
  if (rest.startsWith("->")) result = parseType(rest.slice(2));
  else if (rest !== "") throw new Error(`not a signature: ${head}`);
  const params = splitTop(inner).map((p) => {
    const m = /^(mut\s+)?(\w+):\s*(.+)$/.exec(p);
    if (!m) throw new Error(`not a parameter: ${p}`);
    return { name: m[2], mut: Boolean(m[1]), type: parseType(m[3]) };
  });
  const generics = splitTop(start[2] ?? "").map((g) => {
    const m = /^(\w+)(?::\s*(\w+))?$/.exec(g);
    if (!m) throw new Error(`not a type parameter: ${g}`);
    return { name: m[1], bound: m[2] ?? "" };
  });
  const requires: string[] = [];
  const ensures: string[] = [];
  for (const line of lines.slice(1)) {
    const m = /^(requires|ensures)\s+(.+)$/.exec(line.trim());
    if (!m) throw new Error(`not a contract: ${line}`);
    (m[1] === "requires" ? requires : ensures).push(m[2]);
  }
  return { name: start[1], generics, params, result, effects, requires, ensures };
}

/** Reads a task's `## Types`: records, enums and layout records, one or more lines each. */
export function parseTypes(lines: string[]): Decl[] {
  // A declaration runs until its braces balance.
  const groups: string[][] = [];
  let depth = 0;
  for (const line of lines) {
    if (line.trim() === "") continue;
    if (depth === 0) groups.push([line]);
    else groups[groups.length - 1].push(line);
    depth += (line.match(/\{/g) ?? []).length - (line.match(/\}/g) ?? []).length;
  }
  return groups.map((group): Decl => {
    const text = group.join("\n").trim();
    const layout = /^layout record (\w+)/.exec(text);
    if (layout) return { kind: "layout", name: layout[1], lines: group };
    const record = /^record (\w+)\s*\{([\s\S]*)\}$/.exec(text);
    if (record) {
      const fields = splitTop(record[2]).map((f) => {
        const m = /^(\w+):\s*(.+)$/.exec(f);
        if (!m) throw new Error(`not a field: ${f}`);
        return { name: m[1], type: parseType(m[2]) };
      });
      return { kind: "record", name: record[1], fields };
    }
    const en = /^enum (\w+)\s*\{([\s\S]*)\}$/.exec(text);
    if (en) {
      const variants = splitTop(en[2]).map((v) => {
        const m = /^(\w+)(?:\((.*)\))?$/.exec(v);
        if (!m) throw new Error(`not a variant: ${v}`);
        return { name: m[1], fields: splitTop(m[2] ?? "").map(parseType) };
      });
      return { kind: "enum", name: en[1], variants };
    }
    throw new Error(`not a declaration: ${text}`);
  });
}

interface Cursor {
  s: string;
  i: number;
}

function skipSpace(c: Cursor): void {
  while (c.i < c.s.length && /\s/.test(c.s[c.i])) c.i++;
}

function expect(c: Cursor, ch: string): void {
  skipSpace(c);
  if (c.s[c.i] !== ch) throw new Error(`expected \`${ch}\` at ${c.i} in \`${c.s}\``);
  c.i++;
}

/** Reads values separated by commas up to `close`, the opening bracket already read. */
function valuesUntil(c: Cursor, close: string): Value[] {
  const items: Value[] = [];
  skipSpace(c);
  if (c.s[c.i] === close) {
    c.i++;
    return items;
  }
  for (;;) {
    items.push(valueAt(c));
    skipSpace(c);
    if (c.s[c.i] === ",") c.i++;
    else {
      expect(c, close);
      return items;
    }
  }
}

function valueAt(c: Cursor): Value {
  skipSpace(c);
  const rest = c.s.slice(c.i);
  if (rest.startsWith("[")) {
    c.i++;
    return { kind: "array", items: valuesUntil(c, "]") };
  }
  if (rest.startsWith("()")) {
    c.i += 2;
    return { kind: "unit" };
  }
  if (rest.startsWith("{")) {
    c.i++;
    const fields: { name: string; value: Value }[] = [];
    skipSpace(c);
    if (c.s[c.i] === "}") {
      c.i++;
      return { kind: "record", fields };
    }
    for (;;) {
      skipSpace(c);
      const name = /^\w+/.exec(c.s.slice(c.i));
      if (!name) throw new Error(`expected a field name at ${c.i} in \`${c.s}\``);
      c.i += name[0].length;
      expect(c, ":");
      fields.push({ name: name[0], value: valueAt(c) });
      skipSpace(c);
      if (c.s[c.i] === ",") c.i++;
      else {
        expect(c, "}");
        return { kind: "record", fields };
      }
    }
  }
  if (rest.startsWith('"')) {
    let value = "";
    for (c.i++; c.s[c.i] !== '"'; c.i++) {
      if (c.i >= c.s.length) throw new Error(`an unclosed string in \`${c.s}\``);
      if (c.s[c.i] === "\\") {
        c.i++;
        const escaped: Record<string, string> = { '"': '"', "\\": "\\", n: "\n", t: "\t", r: "\r" };
        if (!(c.s[c.i] in escaped)) throw new Error(`the escape \\${c.s[c.i]} in \`${c.s}\``);
        value += escaped[c.s[c.i]];
      } else value += c.s[c.i];
    }
    c.i++;
    return { kind: "string", value };
  }
  const number = /^-?\d+(\.\d+)?([eE][+-]?\d+)?/.exec(rest);
  if (number) {
    c.i += number[0].length;
    return number[1] || number[2] ? { kind: "float", text: number[0] } : { kind: "int", text: number[0] };
  }
  const word = /^[A-Za-z_]\w*/.exec(rest);
  if (word) {
    c.i += word[0].length;
    if (word[0] === "true" || word[0] === "false") return { kind: "bool", value: word[0] === "true" };
    if (c.s[c.i] === "(") {
      c.i++;
      return { kind: "variant", name: word[0], args: valuesUntil(c, ")") };
    }
    return { kind: "variant", name: word[0], args: [] };
  }
  throw new Error(`not a value at ${c.i} in \`${c.s}\``);
}

/** Reads a value in the benchmark's notation (benchmark/README.md, "Values and tests"). */
export function parseValue(text: string): Value {
  const c = { s: text, i: 0 };
  const value = valueAt(c);
  skipSpace(c);
  if (c.i !== text.length) throw new Error(`\`${text}\` has more after its value`);
  return value;
}

/** Reads a test's call: `name(args)` or `name<T>(args)`, an argument marked `mut` where it is one. */
export function parseCall(text: string): Call {
  const m = /^(\w+)(?:<([^>]*)>)?\(([\s\S]*)\)$/.exec(text.trim());
  if (!m) throw new Error(`not a call: ${text}`);
  const args = splitTop(m[3]).map((a) => {
    const marked = /^mut\s+([\s\S]+)$/.exec(a);
    return { mut: Boolean(marked), value: parseValue(marked ? marked[1] : a) };
  });
  return { name: m[1], typeArgs: splitTop(m[2] ?? ""), args };
}

/** A case's expected result and what each `mut` argument must hold afterwards, as written. */
export function expectedParts(c: Case): { value: string | null; leaves: string[] } {
  if (c.value && c.leaves) {
    const at = c.expected.indexOf(" leaves ");
    return { value: c.expected.slice(0, at), leaves: splitTop(c.expected.slice(at + 8)) };
  }
  return c.leaves ? { value: null, leaves: splitTop(c.expected) } : { value: c.expected, leaves: [] };
}

// ---------------------------------------------------------------------------------------------
// Signatures and types

/** A contract's neutral expression in Rust or Odin: `and`, `or` and `not` as operators. */
export function contractExpr(e: string, lang: "rust" | "odin"): string {
  let s = e
    .replace(/\band\b/g, "&&")
    .replace(/\bor\b/g, "||")
    .replace(/\bnot\s+/g, "!");
  if (lang === "rust") s = s.replace(/\blen\((\w+)\)/g, "$1.len()");
  return s;
}

function rangeText(r: Range, lang: "rust" | "odin"): string {
  const lo = contractExpr(r.lo, lang);
  const hi = contractExpr(r.hi, lang);
  if (r.inclusive) return `${lo}..=${hi}`;
  return lang === "rust" ? `${lo}..${hi}` : `${lo}..<${hi}`;
}

/** What a neutral type states beyond the plain type it becomes, for a doc comment. */
function facts(t: Type, subject: string, lang: "rust" | "odin", nested = false): string[] {
  const inner = (x: Type): string[] => facts(x, subject, lang, true);
  switch (t.kind) {
    case "int":
      if (!t.range) return [];
      return [nested ? `each \`${t.name}\` in ${subject} is in \`${rangeText(t.range, lang)}\`` : `${subject} is in \`${rangeText(t.range, lang)}\``];
    case "f64":
      if (!t.unit) return [];
      return [nested ? `each \`f64\` in ${subject} is a quantity in \`${t.unit}\`` : `${subject} is a quantity in \`${t.unit}\``];
    case "fixed": {
      const what = `the raw bits of a fixed-point number with ${t.int} integer bits and ${t.frac} fraction bits`;
      return [nested ? `each fixed-point number in ${subject} is held as ${what}` : `${subject} holds ${what}`];
    }
    case "array":
    case "slice":
    case "list":
    case "option":
      return inner(t.elem);
    case "result":
      return [...inner(t.ok), ...inner(t.err)];
    default:
      return [];
  }
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** The doc comment's lines: what parameters and result state, then the contracts. */
export function docLines(sig: Signature, lang: "rust" | "odin"): string[] {
  const lines: string[] = [];
  const sentence = (fact: string) => `${capitalize(fact)}.`;
  for (const p of sig.params) lines.push(...facts(p.type, `\`${p.name}\``, lang).map(sentence));
  for (const r of sig.requires) lines.push(`Requires: \`${contractExpr(r, lang)}\`.`);
  if (sig.result) lines.push(...facts(sig.result, "the result", lang).map(sentence));
  for (const e of sig.ensures) {
    const where = /\bresult\b/.test(e) ? ", where `result` is the value returned" : "";
    lines.push(`Ensures: \`${contractExpr(e, lang)}\`${where}.`);
  }
  return lines;
}

function fixedBits(t: { int: number; frac: number }): number {
  const bits = t.int + t.frac;
  if (![8, 16, 32, 64, 128].includes(bits)) throw new Error(`fixed<${t.int}, ${t.frac}> has no integer type of its width`);
  return bits;
}

/**
 * A neutral type in Rust. A `string`, and a slice's elements, are borrowed in a parameter and
 * owned elsewhere: a result, a record's field, a `mut` parameter's value.
 */
export function rustType(t: Type, position: "param" | "owned" = "owned"): string {
  switch (t.kind) {
    case "int":
      return t.name;
    case "bool":
      return "bool";
    case "unit":
      return "()";
    case "array":
      return `[${rustType(t.elem, position)}; ${t.len}]`;
    case "string":
      return position === "param" ? "&str" : "String";
    case "slice":
      return `&[${rustType(t.elem, position)}]`;
    case "list":
      return `Vec<${rustType(t.elem, "owned")}>`;
    case "option":
      return `Option<${rustType(t.elem, position)}>`;
    case "result":
      return `Result<${rustType(t.ok, position)}, ${rustType(t.err, position)}>`;
    case "f64":
      return "f64";
    case "fixed":
      return `i${fixedBits(t)}`;
    case "named":
      return t.name;
  }
}

function rustParamType(p: Param): string {
  return p.mut ? `&mut ${rustType(p.type, "owned")}` : rustType(p.type, "param");
}

/** A task's signature as the Rust agent sees it: its doc comment, then the function's header. */
export function rustSignature(sig: Signature): string {
  const generics = sig.generics.map((g) => {
    if (g.bound !== "Eq") throw new Error(`no Rust mapping for the bound \`${g.bound || "(none)"}\``);
    return `${g.name}: PartialEq + Clone`;
  });
  const params = sig.params.map((p) => `${p.name}: ${rustParamType(p)}`);
  const result = sig.result ? ` -> ${rustType(sig.result, "owned")}` : "";
  const header = `pub fn ${sig.name}${generics.length ? `<${generics.join(", ")}>` : ""}(${params.join(", ")})${result}`;
  return [...docLines(sig, "rust").map((l) => `/// ${l}`), header].join("\n");
}

/** A task's declaration in Rust; a layout record is the agent's to write, so it has none. */
export function rustDecl(d: Decl): string {
  if (d.kind === "layout") throw new Error(`a layout record is declared by the agent (${d.name})`);
  if (d.kind === "record") {
    const fields = d.fields.map((f) => `    pub ${f.name}: ${rustType(f.type, "owned")},`);
    return ["#[derive(Clone, Debug, PartialEq)]", `pub struct ${d.name} {`, ...fields, "}"].join("\n");
  }
  const variants = d.variants.map((v) =>
    v.fields.length === 0 ? `    ${v.name},` : `    ${v.name}(${v.fields.map((f) => rustType(f, "owned")).join(", ")}),`,
  );
  return ["#[derive(Clone, Debug, PartialEq)]", `pub enum ${d.name} {`, ...variants, "}"].join("\n");
}

/**
 * A neutral type in Odin. `pending` holds the type parameters not yet written, so that the first
 * occurrence of each is declared as `$T`.
 */
export function odinType(t: Type, pending: Set<string> = new Set()): string {
  switch (t.kind) {
    case "int":
      return t.name === "usize" ? "uint" : t.name;
    case "bool":
      return "bool";
    case "unit":
      throw new Error("no Odin mapping for `()` inside a type");
    case "array":
      return `[${t.len}]${odinType(t.elem, pending)}`;
    case "string":
      return "string";
    case "slice":
      return `[]${odinType(t.elem, pending)}`;
    case "list":
      return `${t.soa ? "#soa" : ""}[dynamic]${odinType(t.elem, pending)}`;
    case "option":
      return `Maybe(${odinType(t.elem, pending)})`;
    case "result":
      throw new Error("Odin writes a result as two return values, so only a function's result can be one");
    case "f64":
      return "f64";
    case "fixed":
      return `i${fixedBits(t)}`;
    case "named":
      if (pending.delete(t.name)) return `$${t.name}`;
      return t.name;
  }
}

/** A task's signature as the Odin agent sees it: its comment, then the procedure's header. */
export function odinSignature(sig: Signature): string {
  for (const g of sig.generics) {
    if (g.bound !== "Eq") throw new Error(`no Odin mapping for the bound \`${g.bound || "(none)"}\``);
  }
  const pending = new Set(sig.generics.map((g) => g.name));
  const params = sig.params.map((p) => `${p.name}: ${p.mut ? "^" : ""}${odinType(p.type, pending)}`);
  let result = "";
  if (sig.result?.kind === "result") {
    result = ` -> (${odinType(sig.result.ok, pending)}, ${odinType(sig.result.err, pending)})`;
  } else if (sig.result) result = ` -> ${odinType(sig.result, pending)}`;
  return [...docLines(sig, "odin").map((l) => `// ${l}`), `${sig.name} :: proc(${params.join(", ")})${result}`].join("\n");
}

/** A task's declaration in Odin: a record as a struct, an enum without fields as an enum. */
export function odinDecl(d: Decl): string {
  if (d.kind === "layout") throw new Error(`a layout record is declared by the agent (${d.name})`);
  if (d.kind === "record") {
    return [`${d.name} :: struct {`, ...d.fields.map((f) => `\t${f.name}: ${odinType(f.type)},`), "}"].join("\n");
  }
  if (d.variants.some((v) => v.fields.length > 0)) {
    throw new Error(`no Odin mapping yet for an enum whose variants hold values (${d.name})`);
  }
  return [`${d.name} :: enum {`, ...d.variants.map((v) => `\t${v.name},`), "}"].join("\n");
}

// ---------------------------------------------------------------------------------------------
// Values as literals

function describe(t: Type): string {
  return t.kind === "named" ? t.name : t.kind;
}

function unsupported(t: Type, lang: string): Error {
  return new Error(`the harness does not yet write a ${describe(t)} value in ${lang}`);
}

function literalError(v: Value, t: Type, lang: string): Error {
  if (["int", "bool", "array"].includes(t.kind)) return new Error(`a ${v.kind} value where the type is ${describe(t)}`);
  return unsupported(t, lang);
}

/** A value as a Rust literal of type `t`. */
export function rustLiteral(v: Value, t: Type): string {
  if (t.kind === "int" && v.kind === "int") return v.text;
  if (t.kind === "bool" && v.kind === "bool") return String(v.value);
  if (t.kind === "array" && v.kind === "array") {
    if (v.items.length !== t.len) throw new Error(`a [${t.len}] array needs ${t.len} elements, not ${v.items.length}`);
    return `[${v.items.map((x) => rustLiteral(x, t.elem)).join(", ")}]`;
  }
  throw literalError(v, t, "Rust");
}

/** A value as an Odin literal of type `t`; an array's elements leave their type to the array's. */
export function odinLiteral(v: Value, t: Type, typed = true): string {
  if (t.kind === "int" && v.kind === "int") return v.text;
  if (t.kind === "bool" && v.kind === "bool") return String(v.value);
  if (t.kind === "array" && v.kind === "array") {
    if (v.items.length !== t.len) throw new Error(`a [${t.len}] array needs ${t.len} elements, not ${v.items.length}`);
    return `${typed ? odinType(t) : ""}{${v.items.map((x) => odinLiteral(x, t.elem, false)).join(", ")}}`;
  }
  throw literalError(v, t, "Odin");
}

/** Checks a call against the signature: its name, its arguments' number and their `mut` marks. */
function checkCall(sig: Signature, call: Call): void {
  if (call.name !== sig.name) throw new Error(`a call of \`${call.name}\` in a task about \`${sig.name}\``);
  if (call.args.length !== sig.params.length) {
    throw new Error(`\`${sig.name}\` takes ${sig.params.length} arguments, not ${call.args.length}`);
  }
  call.args.forEach((a, i) => {
    if (a.mut !== sig.params[i].mut) throw new Error(`argument ${i + 1} of \`${sig.name}\` is ${sig.params[i].mut ? "" : "not "}mut`);
  });
}

// ---------------------------------------------------------------------------------------------
// Examples as the agent reads them

/** A case as Rust code: the call in `assert_eq!`, and each `mut` argument in a variable. */
export function rustExample(sig: Signature, c: Case): string[] {
  const call = parseCall(c.call);
  checkCall(sig, call);
  const lines: string[] = [];
  const args = call.args.map((a, i) => {
    const p = sig.params[i];
    if (!p.mut) return rustLiteral(a.value, p.type);
    lines.push(`let mut ${p.name} = ${rustLiteral(a.value, p.type)};`);
    return `&mut ${p.name}`;
  });
  const expr = `${sig.name}(${args.join(", ")})`;
  const expected = expectedParts(c);
  const resultType = sig.result ?? { kind: "unit" as const };
  if (expected.value !== null) lines.push(`assert_eq!(${expr}, ${rustLiteral(parseValue(expected.value), resultType)});`);
  else lines.push(`${expr};`);
  const muts = sig.params.filter((p) => p.mut);
  expected.leaves.forEach((w, i) => lines.push(`assert_eq!(${muts[i].name}, ${rustLiteral(parseValue(w), muts[i].type)});`));
  return lines;
}

/** A case as Odin code: the call in `assert`, and each `mut` argument in a variable. */
export function odinExample(sig: Signature, c: Case): string[] {
  const call = parseCall(c.call);
  checkCall(sig, call);
  const lines: string[] = [];
  const args = call.args.map((a, i) => {
    const p = sig.params[i];
    if (!p.mut) return odinLiteral(a.value, p.type);
    lines.push(`${p.name} := ${odinLiteral(a.value, p.type)}`);
    return `&${p.name}`;
  });
  const expr = `${sig.name}(${args.join(", ")})`;
  const expected = expectedParts(c);
  if (expected.value !== null) {
    if (!sig.result) throw new Error(`\`${sig.name}\` returns nothing, yet a case expects a value`);
    lines.push(`assert(${expr} == ${odinLiteral(parseValue(expected.value), sig.result)})`);
  } else lines.push(expr);
  const muts = sig.params.filter((p) => p.mut);
  expected.leaves.forEach((w, i) => lines.push(`assert(${muts[i].name} == ${odinLiteral(parseValue(w), muts[i].type)})`));
  return lines;
}

/** The examples as one block of code in the agent's language, each under its number. */
export function examplesBlock(lang: "rust" | "odin", sig: Signature, cases: Case[]): string {
  const out: string[] = [];
  cases.forEach((c, i) => {
    out.push(`// Example ${i + 1}`);
    if (lang === "rust") out.push(...rustExample(sig, c));
    else {
      const lines = odinExample(sig, c);
      // A variable is declared once per scope in Odin, so an example with one gets its own block.
      if (lines.length > 1) out.push("{", ...lines.map((l) => `\t${l}`), "}");
      else out.push(...lines);
    }
  });
  return out.join("\n");
}

// ---------------------------------------------------------------------------------------------
// The programs the harness builds

const RUST_SHOW = `mod bench_harness {
    /// A value in the benchmark's canonical form, as \`mn run\` prints it.
    pub trait Show {
        fn show(&self) -> String;
    }
    macro_rules! show_display {
        ($($t:ty),*) => {
            $(impl Show for $t {
                fn show(&self) -> String {
                    self.to_string()
                }
            })*
        };
    }
    show_display!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, bool);
    impl Show for () {
        fn show(&self) -> String {
            String::from("()")
        }
    }
    impl<T: Show, const N: usize> Show for [T; N] {
        fn show(&self) -> String {
            let items: Vec<String> = self.iter().map(Show::show).collect();
            format!("[{}]", items.join(", "))
        }
    }
}`;

/** The statements that make one case's call in Rust and print what it gives. */
function rustCase(sig: Signature, c: Case): string[] {
  const call = parseCall(c.call);
  checkCall(sig, call);
  const lines: string[] = [];
  const args = call.args.map((a, i) => {
    const p = sig.params[i];
    const literal = rustLiteral(a.value, p.type);
    if (p.mut) {
      lines.push(`let mut bench_arg${i}: ${rustType(p.type, "owned")} = ${literal};`);
      return `&mut bench_arg${i}`;
    }
    lines.push(`let bench_arg${i}: ${rustType(p.type, "param")} = ${literal};`);
    return `bench_arg${i}`;
  });
  const result = sig.result ? rustType(sig.result, "owned") : "()";
  lines.push(`let bench_result: ${result} = ${sig.name}(${args.join(", ")});`);
  lines.push(`println!("{}", bench_harness::Show::show(&bench_result));`);
  sig.params.forEach((p, i) => {
    if (p.mut) lines.push(`println!("leaves {}", bench_harness::Show::show(&bench_arg${i}));`);
  });
  return lines;
}

/**
 * The Rust program around the agent's function, which it includes from `solution.rs`: run with a
 * case's number, it makes that case's call and prints the result, then what each `mut` argument
 * leaves. It also takes the function as a function pointer of the exact signature, so a program
 * whose signature differs does not build, even with no case to run.
 */
export function rustProgram(sig: Signature, cases: Case[]): string {
  const fnType = `fn(${sig.params.map(rustParamType).join(", ")})${sig.result ? ` -> ${rustType(sig.result, "owned")}` : ""}`;
  const lines = [
    "// The benchmark harness's program around the agent's function (benchmark/harness/languages.ts).",
    'include!("solution.rs");',
    "",
    RUST_SHOW,
    "",
    "fn main() {",
  ];
  if (sig.generics.length === 0) lines.push(`    let _: ${fnType} = ${sig.name};`);
  lines.push("    let case = std::env::args().nth(1).unwrap_or_default();", "    match case.as_str() {");
  cases.forEach((c, i) => {
    lines.push(`        "${i}" => {`, ...rustCase(sig, c).map((l) => `            ${l}`), "        }");
  });
  lines.push("        _ => std::process::exit(2),", "    }", "}", "");
  return lines.join("\n");
}

/** Odin statements that print `expr`, of type `t`, in the canonical form. */
function odinPrint(expr: string, t: Type, depth = 0): string[] {
  if (t.kind === "int" || t.kind === "bool") return [`bench_fmt.print(${expr})`];
  if (t.kind === "array") {
    const v = `bench_v${depth}`;
    const i = `bench_i${depth}`;
    return [
      'bench_fmt.print("[")',
      `for ${v}, ${i} in ${expr} {`,
      `\tif ${i} > 0 { bench_fmt.print(", ") }`,
      ...odinPrint(v, t.elem, depth + 1).map((l) => `\t${l}`),
      "}",
      'bench_fmt.print("]")',
    ];
  }
  throw unsupported(t, "Odin");
}

/** The statements that make one case's call in Odin and print what it gives. */
function odinCase(sig: Signature, c: Case): string[] {
  const call = parseCall(c.call);
  checkCall(sig, call);
  const lines: string[] = [];
  const args = call.args.map((a, i) => {
    const p = sig.params[i];
    lines.push(`bench_arg${i}: ${odinType(p.type)} = ${odinLiteral(a.value, p.type, false)}`);
    return p.mut ? `&bench_arg${i}` : `bench_arg${i}`;
  });
  const callText = `${sig.name}(${args.join(", ")})`;
  if (sig.result) {
    lines.push(`bench_result: ${odinType(sig.result)} = ${callText}`, ...odinPrint("bench_result", sig.result));
  } else lines.push(callText, 'bench_fmt.print("()")');
  lines.push('bench_fmt.print("\\n")');
  sig.params.forEach((p, i) => {
    if (p.mut) lines.push('bench_fmt.print("leaves ")', ...odinPrint(`bench_arg${i}`, p.type), 'bench_fmt.print("\\n")');
  });
  return lines;
}

/**
 * The Odin program around the agent's procedure, which sits in `solution.odin` in the same
 * package: run with a case's number, it makes that case's call and prints the result, then what
 * each `mut` argument leaves. A call it never makes, with arguments of the exact types and the
 * result kept in a variable of the exact type, keeps a program whose signature differs from
 * building, even with no case to run.
 */
export function odinProgram(sig: Signature, cases: Case[]): string {
  if (sig.generics.length > 0) throw new Error("the harness does not yet call a generic procedure in Odin");
  const check = sig.params.map((p, i) => `bench_zero${i}: ${odinType(p.type)}`);
  const zeroArgs = sig.params.map((p, i) => (p.mut ? `&bench_zero${i}` : `bench_zero${i}`)).join(", ");
  if (sig.result) check.push(`bench_checked: ${odinType(sig.result)} = ${sig.name}(${zeroArgs})`, "_ = bench_checked");
  else check.push(`${sig.name}(${zeroArgs})`);
  const lines = [
    "// The benchmark harness's program around the agent's procedure (benchmark/harness/languages.ts).",
    "package main",
    "",
    'import bench_fmt "core:fmt"',
    'import bench_os "core:os"',
    "",
    "main :: proc() {",
    "\tif len(bench_os.args) < 0 {",
    ...check.map((l) => `\t\t${l}`),
    "\t}",
    "\tif len(bench_os.args) < 2 {",
    "\t\tbench_os.exit(2)",
    "\t}",
    "\tswitch bench_os.args[1] {",
  ];
  cases.forEach((c, i) => {
    lines.push(`\tcase "${i}":`, ...odinCase(sig, c).map((l) => `\t\t${l}`));
  });
  lines.push("\tcase:", "\t\tbench_os.exit(2)", "\t}", "}", "");
  return lines.join("\n");
}

/** What a run printed, read back: its result line and what each `mut` argument leaves. */
export function printedParts(stdout: string): { result: string; leaves: string[] } {
  const lines = stdout.trim().split(/\r?\n/);
  return {
    result: lines[0] ?? "",
    leaves: lines.filter((l) => l.startsWith("leaves ")).map((l) => l.slice("leaves ".length)),
  };
}
