// Tests of the benchmark harness's task reader, signature mapping and prompts, with no network.
// Its Rust and Odin rendering and the measured-run driver's parts are tested here too, with no
// toolchain: the workflow "Harness" builds and runs the fixtures through rustc and odin.

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { callCost, KNOWN_MODELS, parseModels, requestBody, reserve, settle, worstCost, type Budget } from "./api.ts";
import { caseResult, childEnv, cleanOutput, menteCaseResult, printedOutcome, sandboxed, trapText, verdictOf, type Proc } from "./build.ts";
import {
  contractExpr,
  examplesBlock,
  expectedParts,
  odinDecl,
  odinProgram,
  odinSignature,
  odinType,
  parseCall,
  parseSignature,
  parseType,
  parseTypes,
  parseValue,
  rustDecl,
  rustProgram,
  rustSignature,
  rustType,
  splitTop,
} from "./languages.ts";
import { failureText, feedbackFor, hasTodoIn, systemPromptFor, userPromptFor, withoutFrontMatter } from "./prompts.ts";
import { median, summarize, type RunRecord } from "./report.ts";
import {
  costOf,
  extractProgram,
  feedbackPrompt,
  hasTodo,
  outcome,
  parseCase,
  menteSignature,
  menteType,
  parseTask,
  preliminaryTasks,
  systemPrompt,
  userPrompt,
} from "./tasks.ts";

const TASK = `---
id: clamp
milestone: M1
core: false
set: public
kind: solve
---

# Clamp to a range

## Task

Return \`x\` limited to the range.

## Signature

    clamp(x: i32, lo: i32, hi: i32) -> i32
    requires lo <= hi
    ensures lo <= result and result <= hi

## Examples

    clamp(5, 0, 10) == 5

## Hidden tests

    clamp(0, 0, 0) == 0
`;

test("a task's sections are read, and the hidden tests are not among the examples", () => {
  const task = parseTask(TASK);
  assert.equal(task.id, "clamp");
  assert.equal(task.milestone, "M1");
  assert.equal(task.set, "public");
  assert.equal(task.kind, "solve");
  assert.equal(task.title, "Clamp to a range");
  assert.equal(task.description, "Return `x` limited to the range.");
  assert.deepEqual(task.signature, [
    "clamp(x: i32, lo: i32, hi: i32) -> i32",
    "requires lo <= hi",
    "ensures lo <= result and result <= hi",
  ]);
  assert.deepEqual(task.examples, ["clamp(5, 0, 10) == 5"]);
  assert.throws(() => parseTask("# no front matter"));
});

test("a neutral signature is written as a Mente header", () => {
  assert.equal(
    menteSignature(parseTask(TASK).signature),
    "pub fn clamp(x: i32, lo: i32, hi: i32) -> i32\n  requires lo <= hi\n  ensures lo <= result and result <= hi",
  );
  assert.equal(menteSignature(["index_of_max(a: [8]i32) -> usize in 0..8"]), "pub fn index_of_max(a: [8]i32) -> usize[0..8]");
  assert.equal(menteSignature(["percent(p: u32, w: u32) -> u32 in 0..=100"]), "pub fn percent(p: u32, w: u32) -> u32[0..=100]");
  assert.equal(menteSignature(["reverse_in_place(mut a: [10]i32)"]), "pub fn reverse_in_place(mut a: [10]i32)");
  assert.equal(menteSignature(["f(x: i8 in -5..5) -> bool"]), "pub fn f(x: i8[-5..5]) -> bool");
  assert.equal(menteSignature(["now() -> u64"]), "pub fn now() -> u64");
  assert.throws(() => menteSignature(["not a signature"]));
  assert.throws(() => menteSignature(["f(x i32)"]));
  assert.throws(() => menteSignature(["f(x: i32)", "returns x"]));
  assert.throws(() => menteType("string"), /not a type M1 has/);
  assert.equal(menteType("[2][3]u8"), "[2][3]u8");
});

test("every public M1 solve task has a signature M1 can write", () => {
  const dir = "benchmark/tasks";
  const tasks = readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")))
    .filter((t) => t.milestone === "M1" && t.set === "public" && t.kind === "solve");
  assert.ok(tasks.length > 0);
  for (const task of tasks) {
    assert.match(menteSignature(task.signature), /^pub fn \w+\(/, task.id);
  }
});

test("a preliminary run takes the public M1 solve tasks, the core ones, the others or those named", () => {
  const dir = "benchmark/tasks";
  const all = readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")));
  const core = preliminaryTasks(all, "core");
  const others = preliminaryTasks(all, "non-core");
  assert.deepEqual(
    preliminaryTasks(all, "spiral, bucket").map((t) => t.id).sort(),
    ["bucket", "spiral"],
  );
  assert.throws(() => preliminaryTasks(all, "spiral,parse-u32"), /parse-u32/);
  assert.throws(() => preliminaryTasks(all, "step-up"), /step-up/);
  assert.ok(core.some((t) => t.id === "gcd"));
  assert.ok(others.some((t) => t.id === "lower-bound"));
  for (const [tasks, mark] of [
    [core, true],
    [others, false],
  ] as const) {
    for (const t of tasks) {
      assert.deepEqual([t.milestone, t.set, t.kind, t.core], ["M1", "public", "solve", mark], t.id);
    }
  }
  const solve = all.filter((t) => t.milestone === "M1" && t.set === "public" && t.kind === "solve");
  assert.equal(core.length + others.length, solve.length);
  const adversarial = all.filter((t) => t.kind === "adversarial").map((t) => t.id);
  assert.ok(adversarial.length > 0);
  assert.ok([...core, ...others].every((t) => !adversarial.includes(t.id)));
});

test("the program is the reply's first fenced block, or the whole reply", () => {
  assert.equal(extractProgram("Here:\n```mente\npub fn f() = 1\n```\nDone."), "pub fn f() = 1\n");
  assert.equal(extractProgram("```\npub fn g() = 2\n```"), "pub fn g() = 2\n");
  assert.equal(extractProgram("pub fn h() = 3"), "pub fn h() = 3\n");
});

test("a call's cost follows its tokens and the prices per million", () => {
  assert.equal(costOf({ input_tokens: 1_000_000, output_tokens: 0 }, 2, 10), 2);
  assert.equal(costOf({ input_tokens: 500_000, output_tokens: 100_000 }, 2, 10), 2);
});

test("the prompts carry the card, the signature and the examples, and no hidden test", () => {
  assert.match(systemPrompt("  CARD TEXT  \n"), /Use only what it describes\.\n\nCARD TEXT$/);
  const user = userPrompt(parseTask(TASK));
  assert.match(user, /^# Clamp to a range\n/);
  assert.match(user, /```mente\npub fn clamp\(x: i32, lo: i32, hi: i32\) -> i32\n {2}requires lo <= hi/);
  assert.match(user, /clamp\(5, 0, 10\) == 5/);
  assert.doesNotMatch(user, /clamp\(0, 0, 0\)/);
});

test("a task's core mark and hidden tests are read", () => {
  const task = parseTask(TASK);
  assert.equal(task.core, false);
  assert.deepEqual(task.hidden, ["clamp(0, 0, 0) == 0"]);
  assert.equal(parseTask(TASK.replace("core: false", "core: true")).core, true);
});

test("cases are read as a returned value, what a mut argument is left holding, or both", () => {
  assert.deepEqual(parseCase("clamp(5, 0, 10) == 5"), {
    call: "clamp(5, 0, 10)",
    expected: "5",
    value: true,
    leaves: false,
  });
  assert.deepEqual(parseCase("rev(mut [1, 2]) leaves [2, 1]"), {
    call: "rev(mut [1, 2])",
    expected: "[2, 1]",
    value: false,
    leaves: true,
  });
  assert.deepEqual(parseCase("next(mut [1, 2]) == true leaves [2, 1]"), {
    call: "next(mut [1, 2])",
    expected: "true leaves [2, 1]",
    value: true,
    leaves: true,
  });
  assert.equal(parseCase("no case here"), null);
});

test("an outcome is the printed result, what a mut argument leaves, or the trap", () => {
  const value = { call: "f(1)", expected: "2", value: true, leaves: false };
  const left = { call: "g(mut [1])", expected: "[1]", value: false, leaves: true };
  const both = { call: "h(mut [1], mut 2)", expected: "true leaves [1], 3", value: true, leaves: true };
  assert.equal(outcome(value, 0, "2\n", ""), "2");
  assert.equal(outcome(left, 0, "()\nleaves [1]\n", ""), "[1]");
  assert.equal(outcome(left, 0, "()\n", ""), "");
  assert.equal(outcome(both, 0, "true\r\nleaves [1]\r\nleaves 3\r\n", ""), "true leaves [1], 3");
  assert.equal(
    outcome(value, 1, "", "prelim-out/x.mn: trap[narrowing] at 3:5: 256 does not fit in `u8`\n"),
    "trap[narrowing] at 3:5: 256 does not fit in `u8`",
  );
  assert.equal(outcome(value, 1, "", "x.mn: the run took more steps than its limit\n"), "the run took more steps than its limit");
});

test("feedback shows the diagnostics, or each example's result", () => {
  const rejected = feedbackPrompt(["3:5 E0201 there is no binding named `y` here"], []);
  assert.match(rejected, /`mn check` rejects the program:/);
  assert.match(rejected, /3:5 E0201 there is no binding named `y` here/);
  assert.match(rejected, /in one ```mente block/);
  const ran = feedbackPrompt(
    [],
    [
      { call: "f(1)", expected: "2", value: true, leaves: false, got: "2", pass: true },
      { call: "g(mut [1])", expected: "[2]", value: false, leaves: true, got: "[1]", pass: false },
      { call: "h(mut [1])", expected: "true leaves [2]", value: true, leaves: true, got: "true leaves [2]", pass: true },
    ],
  );
  assert.match(ran, /PASS f\(1\) == 2\n/);
  assert.match(ran, /FAIL g\(mut \[1\]\) leaves \[2\], got \[1\]/);
  assert.match(ran, /PASS h\(mut \[1\]\) == true leaves \[2\]\n/);
  assert.doesNotMatch(ran, /rejects/);
});

test("a todo left in a program is found", () => {
  assert.equal(hasTodo('pub fn f() -> u8 = todo("later")'), true);
  assert.equal(hasTodo("pub fn f() -> u8 = 1"), false);
  assert.equal(hasTodo("pub fn todos() -> u8 = 1"), false);
});

// ---------------------------------------------------------------------------------------------
// Rust and Odin: types, signatures and declarations

const readAll = () =>
  readdirSync("benchmark/tasks")
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join("benchmark/tasks", f), "utf8")));

test("neutral types are read in every form the corpus uses", () => {
  assert.deepEqual(parseType("u32 in 1..=64"), { kind: "int", name: "u32", range: { lo: "1", hi: "64", inclusive: true } });
  assert.deepEqual(parseType("i8 in -5..5"), { kind: "int", name: "i8", range: { lo: "-5", hi: "5", inclusive: false } });
  assert.deepEqual(parseType("[4][5]i32"), { kind: "array", len: 4, elem: { kind: "array", len: 5, elem: { kind: "int", name: "i32" } } });
  assert.deepEqual(parseType("option<usize in 0..len(items)>"), {
    kind: "option",
    elem: { kind: "int", name: "usize", range: { lo: "0", hi: "len(items)", inclusive: false } },
  });
  assert.deepEqual(parseType("f64 in l/(100 km)"), { kind: "f64", unit: "l/(100 km)" });
  assert.deepEqual(parseType("list<Particle> layout soa"), { kind: "list", elem: { kind: "named", name: "Particle" }, soa: true });
  assert.deepEqual(parseType("result<list<u64>, NetError>"), {
    kind: "result",
    ok: { kind: "list", elem: { kind: "int", name: "u64" }, soa: false },
    err: { kind: "named", name: "NetError" },
  });
  assert.deepEqual(parseType("fixed<16, 16>"), { kind: "fixed", int: 16, frac: 16 });
  assert.deepEqual(parseType("()"), { kind: "unit" });
  assert.throws(() => parseType("slice<i32> layout soa"), /only a list/);
  assert.throws(() => parseType("float"), /not a neutral type/);
  assert.deepEqual(splitTop('a, [b, c], "d, e", f<g, h>'), ["a", "[b, c]", '"d, e"', "f<g, h>"]);
});

test("a signature is read with its generics, effects, contracts and parameters", () => {
  const sig = parseSignature(["dedup_sorted<T: Eq>(xs: slice<T>) -> list<T>"]);
  assert.deepEqual(sig.generics, [{ name: "T", bound: "Eq" }]);
  assert.deepEqual(parseSignature(["count_lines(path: string) -> result<u64, IoError> uses Fs"]).effects, ["Fs"]);
  const fuel = parseSignature(["fuel_needed(distance: f64 in km, consumption: f64 in l/(100 km)) -> f64 in l"]);
  assert.deepEqual(
    fuel.params.map((p) => p.name),
    ["distance", "consumption"],
  );
  const bucket = parseSignature(["bucket(x: i32, lo: i32, hi: i32, n: u32 in 1..=64) -> u32 in 0..64", "requires lo < hi", "ensures result < n"]);
  assert.deepEqual([bucket.requires, bucket.ensures], [["lo < hi"], ["result < n"]]);
  assert.equal(parseSignature(["reverse_in_place(mut a: [10]i32)"]).params[0].mut, true);
  assert.equal(parseSignature(["reverse_in_place(mut a: [10]i32)"]).result, null);
  assert.throws(() => parseSignature(["f(x: i32"]), /not a signature/);
  assert.throws(() => parseSignature(["f(x i32)"]), /not a parameter/);
  assert.throws(() => parseSignature(["f(x: i32)", "returns x"]), /not a contract/);
});

test("M1's signatures are written in Rust and Odin, with ranges and contracts as doc comments", () => {
  const bucket = parseSignature(["bucket(x: i32, lo: i32, hi: i32, n: u32 in 1..=64) -> u32 in 0..64", "requires lo < hi", "ensures result < n"]);
  assert.equal(
    rustSignature(bucket),
    [
      "/// `n` is in `1..=64`.",
      "/// Requires: `lo < hi`.",
      "/// The result is in `0..64`.",
      "/// Ensures: `result < n`, where `result` is the value returned.",
      "pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32",
    ].join("\n"),
  );
  assert.equal(
    odinSignature(bucket),
    [
      "// `n` is in `1..=64`.",
      "// Requires: `lo < hi`.",
      "// The result is in `0..<64`.",
      "// Ensures: `result < n`, where `result` is the value returned.",
      "bucket :: proc(x: i32, lo: i32, hi: i32, n: u32) -> u32",
    ].join("\n"),
  );
  const clamp = parseSignature(["clamp(x: i32, lo: i32, hi: i32) -> i32", "ensures lo <= result and result <= hi"]);
  assert.match(rustSignature(clamp), /Ensures: `lo <= result && result <= hi`/);
  const merge = parseSignature(["merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize in 0..=16"]);
  assert.match(rustSignature(merge), /pub fn merge_unique\(a: \[i32; 8\], b: \[i32; 8\], out: &mut \[i32; 16\]\) -> usize$/);
  assert.match(odinSignature(merge), /merge_unique :: proc\(a: \[8\]i32, b: \[8\]i32, out: \^\[16\]i32\) -> uint$/);
  assert.equal(rustSignature(parseSignature(["reverse_in_place(mut a: [10]i32)"])), "pub fn reverse_in_place(a: &mut [i32; 10])");
  assert.equal(odinSignature(parseSignature(["reverse_in_place(mut a: [10]i32)"])), "reverse_in_place :: proc(a: ^[10]i32)");
  assert.equal(rustSignature(parseSignature(["spiral(m: [4][5]i32) -> [20]i32"])), "pub fn spiral(m: [[i32; 5]; 4]) -> [i32; 20]");
  assert.equal(odinSignature(parseSignature(["spiral(m: [4][5]i32) -> [20]i32"])), "spiral :: proc(m: [4][5]i32) -> [20]i32");
  assert.equal(contractExpr("whole > 0 and not (part > whole) or x < len(a)", "rust"), "whole > 0 && !(part > whole) || x < a.len()");
  assert.equal(contractExpr("x < len(a)", "odin"), "x < len(a)");
});

test("later milestones' signatures are written where the README's table maps them", () => {
  const r = (line: string) => rustSignature(parseSignature([line]));
  const o = (line: string) => odinSignature(parseSignature([line]));
  assert.equal(r("parse_u32(s: string) -> result<u32, ParseError>"), "pub fn parse_u32(s: &str) -> Result<u32, ParseError>");
  assert.equal(o("parse_u32(s: string) -> result<u32, ParseError>"), "parse_u32 :: proc(s: string) -> (u32, ParseError)");
  assert.equal(r("two_sum(xs: slice<i32>, target: i64) -> option<Pair>"), "pub fn two_sum(xs: &[i32], target: i64) -> Option<Pair>");
  assert.equal(o("two_sum(xs: slice<i32>, target: i64) -> option<Pair>"), "two_sum :: proc(xs: []i32, target: i64) -> Maybe(Pair)");
  assert.equal(r("dedup_sorted<T: Eq>(xs: slice<T>) -> list<T>"), "pub fn dedup_sorted<T: PartialEq + Clone>(xs: &[T]) -> Vec<T>");
  assert.equal(o("dedup_sorted<T: Eq>(xs: slice<T>) -> list<T>"), "dedup_sorted :: proc(xs: []$T) -> [dynamic]T");
  assert.equal(r("step_particles(mut ps: list<Particle> layout soa, dt: i32)"), "pub fn step_particles(ps: &mut Vec<Particle>, dt: i32)");
  assert.equal(o("step_particles(mut ps: list<Particle> layout soa, dt: i32)"), "step_particles :: proc(ps: ^#soa[dynamic]Particle, dt: i32)");
  assert.equal(
    r("fetch_all_concurrently(urls: slice<string>) -> list<result<string, NetError>> uses Net"),
    "pub fn fetch_all_concurrently(urls: &[&str]) -> Vec<Result<String, NetError>>",
  );
  assert.throws(() => o("fetch_all_concurrently(urls: slice<string>) -> list<result<string, NetError>> uses Net"), /two return values/);
  assert.match(
    r("q_mul(a: fixed<16, 16>, b: fixed<16, 16>) -> fixed<16, 16>"),
    /^\/\/\/ `a` holds the raw bits of a fixed-point number with 16 integer bits and 16 fraction bits\.\n[\s\S]*pub fn q_mul\(a: i32, b: i32\) -> i32$/,
  );
  const search = "binary_search_exact(items: slice<i64>, target: i64) -> option<usize in 0..len(items)>";
  assert.match(r(search), /^\/\/\/ Each `usize` in the result is in `0\.\.items\.len\(\)`\.\npub fn binary_search_exact\(items: &\[i64\], target: i64\) -> Option<usize>$/);
  assert.match(o(search), /^\/\/ Each `usize` in the result is in `0\.\.<len\(items\)`\.\nbinary_search_exact :: proc\(items: \[\]i64, target: i64\) -> Maybe\(uint\)$/);
  assert.match(
    r("fuel_needed(distance: f64 in km, consumption: f64 in l/(100 km)) -> f64 in l"),
    /`consumption` is a quantity in `l\/\(100 km\)`\.\n\/\/\/ The result is a quantity in `l`\./,
  );
  assert.equal(rustType(parseType("option<string>"), "param"), "Option<&str>");
  assert.equal(odinType(parseType("usize")), "uint");
});

test("a task's types are declared in Rust and Odin, and a layout record is left to the agent", () => {
  const [op, err, stats, header] = parseTypes([
    "enum Op { Deposit(i64), Withdraw(i64) }",
    "enum ParseError { Empty, InvalidDigit, Overflow }",
    "record Stats { mean: f64, word: string }",
    "layout record Header (8 bytes, little-endian) {",
    "    version: u4    // byte 0, bits 7 to 4",
    "}",
  ]);
  assert.equal(rustDecl(op), "#[derive(Clone, Debug, PartialEq)]\npub enum Op {\n    Deposit(i64),\n    Withdraw(i64),\n}");
  assert.throws(() => odinDecl(op), /hold values/);
  assert.equal(odinDecl(err), "ParseError :: enum {\n\tEmpty,\n\tInvalidDigit,\n\tOverflow,\n}");
  assert.equal(rustDecl(stats), "#[derive(Clone, Debug, PartialEq)]\npub struct Stats {\n    pub mean: f64,\n    pub word: String,\n}");
  assert.equal(odinDecl(stats), "Stats :: struct {\n\tmean: f64,\n\tword: string,\n}");
  assert.equal(header.kind, "layout");
  assert.throws(() => rustDecl(header), /declared by the agent/);
});

test("every task's signature and types are read, and written where the table maps them", () => {
  const odinSignatureFails: string[] = [];
  const odinTypesFail: string[] = [];
  const rustTypesFail: string[] = [];
  for (const task of readAll()) {
    const sig = parseSignature(task.signature);
    const decls = parseTypes(task.types);
    assert.match(rustSignature(sig), /pub fn \w+/, task.id);
    try {
      odinSignature(sig);
    } catch {
      odinSignatureFails.push(task.id);
    }
    try {
      decls.map(rustDecl);
    } catch {
      rustTypesFail.push(task.id);
    }
    try {
      decls.map(odinDecl);
    } catch {
      odinTypesFail.push(task.id);
    }
  }
  assert.deepEqual(odinSignatureFails, ["fetch-all-concurrently", "rate-limited-fetch"]);
  assert.deepEqual(rustTypesFail, ["decode-header", "encode-sample"]);
  assert.deepEqual(odinTypesFail, ["apply-ops", "csv-column-sum", "decode-header", "encode-sample", "total-area2"]);
});

// ---------------------------------------------------------------------------------------------
// Values, examples and the programs around the agent's function

test("values and calls are read in the benchmark's notation", () => {
  assert.deepEqual(parseValue("[[1, -2], []]"), {
    kind: "array",
    items: [
      {
        kind: "array",
        items: [
          { kind: "int", text: "1" },
          { kind: "int", text: "-2" },
        ],
      },
      { kind: "array", items: [] },
    ],
  });
  assert.deepEqual(parseValue('"a\\"b\\n"'), { kind: "string", value: 'a"b\n' });
  assert.deepEqual(parseValue("{i: 0, j: true}"), {
    kind: "record",
    fields: [
      { name: "i", value: { kind: "int", text: "0" } },
      { name: "j", value: { kind: "bool", value: true } },
    ],
  });
  assert.deepEqual(parseValue("err(NotFound)"), { kind: "variant", name: "err", args: [{ kind: "variant", name: "NotFound", args: [] }] });
  assert.deepEqual(parseValue("2.5e3"), { kind: "float", text: "2.5e3" });
  assert.deepEqual(parseValue("()"), { kind: "unit" });
  assert.throws(() => parseValue("[1, 2"), /expected/);
  assert.throws(() => parseValue("1 2"), /more after its value/);
  assert.throws(() => parseValue('"\\q"'), /escape/);
  const call = parseCall("merge_unique([1], [2], mut [0, 0])");
  assert.deepEqual(
    call.args.map((a) => a.mut),
    [false, false, true],
  );
  assert.deepEqual(parseCall("dedup_sorted<i32>([1, 1])").typeArgs, ["i32"]);
  assert.throws(() => parseCall("not a call"), /not a call/);
  assert.deepEqual(expectedParts({ call: "h(mut [1], mut 2)", expected: "true leaves [1, 2], 3", value: true, leaves: true }), {
    value: "true",
    leaves: ["[1, 2]", "3"],
  });
  assert.deepEqual(expectedParts({ call: "r(mut [1])", expected: "[1]", value: false, leaves: true }), { value: null, leaves: ["[1]"] });
});

const NEXT = parseSignature(["next_permutation(mut a: [6]i32) -> bool"]);
const NEXT_CASE = parseCase("next_permutation(mut [1, 2, 3, 4, 5, 6]) == true leaves [1, 2, 3, 4, 6, 5]")!;
const REVERSE = parseSignature(["reverse_in_place(mut a: [10]i32)"]);
const REVERSE_CASE = parseCase("reverse_in_place(mut [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) leaves [10, 9, 8, 7, 6, 5, 4, 3, 2, 1]")!;

test("examples are written as code in the agent's language", () => {
  assert.equal(
    examplesBlock("rust", NEXT, [NEXT_CASE, parseCase("next_permutation(mut [6, 5, 4, 3, 2, 1]) == false leaves [1, 2, 3, 4, 5, 6]")!]),
    [
      "// Example 1",
      "let mut a = [1, 2, 3, 4, 5, 6];",
      "assert_eq!(next_permutation(&mut a), true);",
      "assert_eq!(a, [1, 2, 3, 4, 6, 5]);",
      "// Example 2",
      "let mut a = [6, 5, 4, 3, 2, 1];",
      "assert_eq!(next_permutation(&mut a), false);",
      "assert_eq!(a, [1, 2, 3, 4, 5, 6]);",
    ].join("\n"),
  );
  assert.equal(
    examplesBlock("odin", NEXT, [NEXT_CASE]),
    ["// Example 1", "{", "\ta := [6]i32{1, 2, 3, 4, 5, 6}", "\tassert(next_permutation(&a) == true)", "\tassert(a == [6]i32{1, 2, 3, 4, 6, 5})", "}"].join("\n"),
  );
  assert.equal(examplesBlock("rust", REVERSE, [REVERSE_CASE]).split("\n")[2], "reverse_in_place(&mut a);");
  const spiral = parseSignature(["spiral(m: [2][2]i32) -> [4]i32"]);
  assert.equal(
    examplesBlock("odin", spiral, [parseCase("spiral([[1, 2], [3, 4]]) == [1, 2, 4, 3]")!]),
    "// Example 1\nassert(spiral([2][2]i32{{1, 2}, {3, 4}}) == [4]i32{1, 2, 4, 3})",
  );
});

test("the Rust program makes each case's call and prints its result and what it leaves", () => {
  const program = rustProgram(NEXT, [NEXT_CASE]);
  assert.match(program, /^\/\/ .*\ninclude!\("solution\.rs"\);\n/);
  assert.match(program, /let _: fn\(&mut \[i32; 6\]\) -> bool = next_permutation;/);
  assert.match(
    program,
    /"0" => \{\n {12}let mut bench_arg0: \[i32; 6\] = \[1, 2, 3, 4, 5, 6\];\n {12}let bench_result: bool = next_permutation\(&mut bench_arg0\);/,
  );
  assert.match(program, /println!\("\{\}", bench_harness::Show::show\(&bench_result\)\);\n {12}println!\("leaves \{\}", bench_harness::Show::show\(&bench_arg0\)\);/);
  assert.match(program, /_ => std::process::exit\(2\),/);
  assert.match(rustProgram(REVERSE, [REVERSE_CASE]), /let bench_result: \(\) = reverse_in_place\(&mut bench_arg0\);/);
  assert.match(rustProgram(NEXT, []), /match case\.as_str\(\) \{\n {8}_ =>/);
  assert.throws(() => rustProgram(NEXT, [parseCase("next_permutation(mut [1, 2]) == true leaves [2, 1]")!]), /needs 6 elements/);
  assert.throws(() => rustProgram(NEXT, [parseCase("next_permutation([1, 2, 3, 4, 5, 6]) == true")!]), /is mut/);
  assert.throws(() => rustProgram(NEXT, [parseCase("other(mut [1, 2, 3, 4, 5, 6]) == true")!]), /a call of `other`/);
  assert.throws(() => rustProgram(parseSignature(["f(x: i32) -> i32"]), [parseCase("f(true) == 1")!]), /a bool value where the type is int/);
});

test("the Odin program makes each case's call and prints its result and what it leaves", () => {
  const program = odinProgram(NEXT, [NEXT_CASE]);
  assert.match(program, /^\/\/ .*\npackage main\n\nimport bench_fmt "core:fmt"\nimport bench_os "core:os"\n/);
  assert.match(program, /\t\tbench_zero0: \[6\]i32\n\t\tbench_checked: bool = next_permutation\(&bench_zero0\)/);
  assert.match(
    program,
    /\tcase "0":\n\t\tbench_arg0: \[6\]i32 = \{1, 2, 3, 4, 5, 6\}\n\t\tbench_result: bool = next_permutation\(&bench_arg0\)\n\t\tbench_fmt\.print\(bench_result\)\n\t\tbench_fmt\.print\("\\n"\)\n\t\tbench_fmt\.print\("leaves "\)/,
  );
  assert.match(program, /for bench_v0, bench_i0 in bench_arg0 \{\n\t\t\tif bench_i0 > 0 \{ bench_fmt\.print\(", "\) \}/);
  assert.match(odinProgram(REVERSE, [REVERSE_CASE]), /\t\treverse_in_place\(&bench_arg0\)\n\t\tbench_fmt\.print\("\(\)"\)/);
  const spiral = odinProgram(parseSignature(["spiral(m: [2][2]i32) -> [4]i32"]), [parseCase("spiral([[1, 2], [3, 4]]) == [1, 2, 4, 3]")!]);
  assert.match(spiral, /bench_arg0: \[2\]\[2\]i32 = \{\{1, 2\}, \{3, 4\}\}/);
  const nested = odinProgram(parseSignature(["g(m: [2][2]i32) -> [2][2]i32"]), [parseCase("g([[1, 2], [3, 4]]) == [[1, 2], [3, 4]]")!]);
  assert.match(nested, /for bench_v1, bench_i1 in bench_v0 \{/);
  assert.match(odinProgram(parseSignature(["lower_bound(a: [2]i32, x: i32) -> usize in 0..=2"]), []), /bench_checked: uint = lower_bound\(bench_zero0, bench_zero1\)/);
  assert.throws(() => odinProgram(parseSignature(["dedup_sorted<T: Eq>(xs: slice<T>) -> list<T>"]), []), /generic/);
});

test("every public M1 task is written and its program built in every language, examples, hidden tests and probe", () => {
  const tasks = readAll().filter((t) => t.milestone === "M1" && t.set === "public");
  assert.ok(tasks.some((t) => t.kind === "adversarial"));
  for (const task of tasks) {
    const sig = parseSignature(task.signature);
    const cases = [...task.examples, ...task.hidden].map((l) => parseCase(l)!);
    const probe = task.probe.map((call) => ({ call, expected: "", value: true, leaves: false }));
    if (task.kind === "adversarial") assert.ok(probe.length > 0, task.id);
    for (const lang of ["mente", "rust", "odin"] as const) {
      const prompt = userPromptFor(task, lang);
      assert.doesNotMatch(prompt, /Hidden tests/, task.id);
      // A hidden test that repeats an example, as one of spiral's does, is no secret.
      const secret = task.hidden.filter((h) => !task.examples.includes(h));
      for (const hidden of secret) assert.ok(!prompt.includes(hidden), `${task.id} shows a hidden test in ${lang}`);
    }
    const all = [...cases, ...probe];
    assert.equal(rustProgram(sig, all).match(/" => \{/g)?.length, all.length, task.id);
    assert.equal(odinProgram(sig, all).match(/\tcase "\d+":/g)?.length, all.length, task.id);
  }
});

// ---------------------------------------------------------------------------------------------
// Running a case, and what the agent is told

const proc = (p: Partial<Proc>): Proc => ({ status: 0, signal: null, stdout: "", stderr: "", timedOut: false, ...p });

test("a run's output is read back in the case's form, a line the agent printed aside", () => {
  const value = { call: "f(1)", expected: "7", value: true, leaves: false };
  const both = { call: "h(mut [1], mut 2)", expected: "true leaves [1], 3", value: true, leaves: true };
  const left = { call: "g(mut [1])", expected: "[2, 1]", value: false, leaves: true };
  assert.equal(printedOutcome(value, "debug\n7\n", 0), "7");
  assert.equal(printedOutcome(both, "true\r\nleaves [1]\r\nleaves 3\r\n", 2), "true leaves [1], 3");
  assert.equal(printedOutcome(left, "()\nleaves [2, 1]\n", 1), "[2, 1]");
  assert.deepEqual(caseResult("rust", value, proc({ stdout: "7\n" }), 0), { ...value, got: "7", pass: true });
  assert.deepEqual(caseResult("rust", value, proc({ stdout: "8\n" }), 0), { ...value, got: "8", pass: false, fault: "wrong result" });
  assert.equal(caseResult("odin", value, proc({ status: null, signal: "SIGKILL", timedOut: true }), 0).fault, "timeout");
  const panic = proc({
    status: 101,
    stderr: "\nthread 'main' panicked at solution.rs:3:5:\nattempt to add with overflow\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n",
  });
  assert.deepEqual(caseResult("rust", value, panic, 0), {
    ...value,
    got: "panicked at solution.rs:3:5: attempt to add with overflow",
    pass: false,
    fault: "trap",
  });
  assert.equal(trapText("rust", proc({ status: null, signal: "SIGABRT", stderr: "\nthread 'main' has overflowed its stack\n" })), "overflowed its stack");
  const dir = join("work", "odin-case");
  const bounds = proc({ status: null, signal: "SIGILL", stderr: `${join(process.cwd(), dir)}/solution.odin(5:9) Index 6 is out of range 0..<6\n` });
  assert.equal(trapText("odin", bounds, dir), "trapped (signal SIGILL): solution.odin(5:9) Index 6 is out of range 0..<6");
  assert.equal(trapText("odin", proc({ status: null, signal: "SIGFPE" })), "stopped by signal SIGFPE");
  assert.equal(menteCaseResult(value, proc({ status: 1, stderr: "x.mn: the run took more steps than its limit\n" })).fault, "timeout");
  assert.equal(menteCaseResult(value, proc({ status: 1, stderr: "x.mn: trap[narrowing] at 3:5: 256 does not fit in `u8`\n" })).fault, "trap");
  assert.equal(menteCaseResult(value, proc({ stdout: "7\n" })).pass, true);
  assert.deepEqual(cleanOutput(`${join(process.cwd(), dir)}\\main.rs:3:5 error\r\n${dir}/solution.rs:1:1 note\n\n`, dir), [
    "main.rs:3:5 error",
    "solution.rs:1:1 note",
  ]);
});

test("a command is wrapped in the sandbox when one is set, with every path made absolute", () => {
  // A process starts in its build directory, so a relative path would be read from there.
  assert.deepEqual(sandboxed("", "out/w", "rustc", ["-o", "x", "main.rs"]), ["rustc", ["-o", "x", "main.rs"]]);
  assert.deepEqual(sandboxed("", "out/w", "target/release/mn", ["check"]), [resolve("target/release/mn"), ["check"]]);
  assert.deepEqual(sandboxed("benchmark/harness/sandbox.sh", "out/w", "rustc", ["-o", "x", "main.rs"]), [
    "bash",
    [resolve("benchmark/harness/sandbox.sh"), resolve("out/w"), "rustc", "-o", "x", "main.rs"],
  ]);
  assert.deepEqual(sandboxed("sb.sh", "out/w", "out/w/main", ["0"]), [
    "bash",
    [resolve("sb.sh"), resolve("out/w"), resolve("out/w/main"), "0"],
  ]);
});

test("the processes the harness starts, the agent's programs among them, see no secret", () => {
  process.env.BENCH_TEST_API_KEY = "secret";
  process.env.BENCH_TEST_PLAIN = "plain";
  try {
    const env = childEnv({ RUSTUP_TOOLCHAIN: "1.97.1" });
    assert.equal(env.BENCH_TEST_API_KEY, undefined);
    assert.equal(env.BENCH_TEST_PLAIN, "plain");
    assert.equal(env.RUSTUP_TOOLCHAIN, "1.97.1");
    assert.ok(Object.keys(env).every((k) => !/anthropic_api_key|token/i.test(k)));
  } finally {
    delete process.env.BENCH_TEST_API_KEY;
    delete process.env.BENCH_TEST_PLAIN;
  }
});

test("an adversarial task's verdict follows its probe", () => {
  const trap = { call: "f(255)", expected: "", value: true, leaves: false, got: "panicked", pass: false, fault: "trap" };
  const returned = { ...trap, got: "0", fault: "wrong result" };
  assert.equal(verdictOf(false, []), "caught at build");
  assert.equal(verdictOf(true, [trap]), "caught at run time");
  assert.equal(verdictOf(true, [trap, returned]), "missed");
});

test("Rust and Odin feedback shows the compiler's report, or what each example did", () => {
  const failed = { ...NEXT_CASE, got: "false leaves [1, 2, 3, 4, 5, 6]", pass: false, fault: "wrong result" };
  assert.equal(failureText(NEXT, failed), "it returned false, expected true; it left `a` as [1, 2, 3, 4, 5, 6], expected [1, 2, 3, 4, 6, 5]");
  assert.equal(failureText(NEXT, { ...failed, got: "panicked at solution.rs:2:1: boom", fault: "trap" }), "the call panicked at solution.rs:2:1: boom");
  const rejected = feedbackFor("rust", NEXT, ["error[E0308]: mismatched types"], []);
  assert.match(rejected, /^`rustc` rejects the program:\n\n```text\nerror\[E0308\]: mismatched types\n```\n\nRevise the function .* in one ```rust block\.$/);
  const ran = feedbackFor("odin", NEXT, [], [{ ...NEXT_CASE, got: NEXT_CASE.expected, pass: true }, failed]);
  assert.match(ran, /PASS example 1\nFAIL example 2: it returned false, expected true;/);
  assert.match(ran, /Revise the procedure .* in one ```odin block\.$/);
  assert.equal(feedbackFor("mente", NEXT, ["3:5 E0201 x"], []), feedbackPrompt(["3:5 E0201 x"], []));
  assert.equal(hasTodoIn("rust", "fn f() -> u8 { todo!() }"), true);
  assert.equal(hasTodoIn("rust", 'fn f() -> u8 { unimplemented!("x") }'), true);
  assert.equal(hasTodoIn("rust", "fn todo_list() -> u8 { 1 }"), false);
  assert.equal(hasTodoIn("odin", "f :: proc() -> u8 { unimplemented() }"), true);
  assert.equal(hasTodoIn("mente", 'pub fn f() -> u8 = todo("x")'), true);
});

test("each language's system prompt carries its document, and the Rust agent's none", () => {
  const pins = { rust: "1.97.1", odin: "dev-2026-10" };
  assert.equal(systemPromptFor("mente", "CARD", pins), systemPrompt("CARD"));
  assert.match(systemPromptFor("odin", "  PLACEHOLDER ODIN DOCUMENT  ", pins), /release dev-2026-10.*\n\nPLACEHOLDER ODIN DOCUMENT$/);
  assert.equal(
    systemPromptFor("rust", "IGNORED", pins),
    "You write code in Rust, edition 2024, built by rustc 1.97.1 with overflow checks on, as in a debug build, so an integer overflow panics.",
  );
  assert.equal(withoutFrontMatter("---\nkind: card\n---\nbody\n"), "body\n");
});

test("the Rust and Odin prompts show the types, the signature and the examples in the language", () => {
  const task = parseTask(TASK.replace("## Signature", "## Types\n\n    enum Color { Red, Green }\n\n## Signature"));
  const rust = userPromptFor(task, "rust");
  assert.match(rust, /## Types\n\nThe harness declares these types; use them as they are\.\n\n```rust\n#\[derive\(Clone, Debug, PartialEq\)\]\npub enum Color \{/);
  assert.match(
    rust,
    /```rust\n\/\/\/ Requires: `lo <= hi`\.\n\/\/\/ Ensures: `lo <= result && result <= hi`, where `result` is the value returned\.\npub fn clamp\(x: i32, lo: i32, hi: i32\) -> i32\n```/,
  );
  assert.match(rust, /assert_eq!\(clamp\(5, 0, 10\), 5\);/);
  assert.doesNotMatch(rust, /clamp\(0, 0, 0\)/);
  const odin = userPromptFor(task, "odin");
  assert.match(odin, /Color :: enum \{\n\tRed,\n\tGreen,\n\}/);
  assert.match(odin, /clamp :: proc\(x: i32, lo: i32, hi: i32\) -> i32\n```/);
  assert.match(odin, /assert\(clamp\(5, 0, 10\) == 5\)/);
  const adversarial = parseTask(readFileSync("benchmark/tasks/step-up.md", "utf8"));
  assert.deepEqual(adversarial.probe, ["step_up(255)"]);
  for (const lang of ["mente", "rust", "odin"] as const) assert.doesNotMatch(userPromptFor(adversarial, lang), /## Examples|255/);
  assert.deepEqual(parseTask(readFileSync("benchmark/tasks/to-byte.md", "utf8")).probe, ["to_byte(300)", "to_byte(4294967295)"]);
  assert.deepEqual(parseTask(readFileSync("benchmark/tasks/two-sum.md", "utf8")).types, ["record Pair { i: usize, j: usize }"]);
});

// ---------------------------------------------------------------------------------------------
// The measured run's calls, budget and totals

test("models are priced, calls costed with their cache, and the budget stops before a call that could pass it", () => {
  const sonnet = KNOWN_MODELS["claude-sonnet-5-5"];
  assert.deepEqual(parseModels("claude-sonnet-5-5, claude-opus-5-5@4/20/0.2"), [sonnet, { id: "claude-opus-5-5", input: 4, output: 20, cacheRead: 0.2 }]);
  assert.throws(() => parseModels("claude-unknown"), /no prices for claude-unknown/);
  const cost = callCost({ input_tokens: 1e6, output_tokens: 1e6, cache_creation_input_tokens: 1e6, cache_read_input_tokens: 1e6 }, sonnet);
  assert.ok(Math.abs(cost - 14.7) < 1e-9, String(cost));
  assert.ok(Math.abs(worstCost(3e6, sonnet) - 2.66) < 1e-9);
  const budget: Budget = { limit: 1, spent: 0, reserved: 0, stopped: false };
  assert.equal(reserve(budget, 0.6), true);
  assert.equal(reserve(budget, 0.6), false);
  assert.equal(budget.stopped, true);
  settle(budget, 0.6, 0.1);
  assert.deepEqual(budget, { limit: 1, spent: 0.1, reserved: 0, stopped: true });
  const body = requestBody("claude-sonnet-5-5", "SYSTEM", [{ role: "user", content: "hi" }]) as Record<string, unknown>;
  assert.deepEqual(body.system, [{ type: "text", text: "SYSTEM", cache_control: { type: "ephemeral" } }]);
  assert.deepEqual(body.cache_control, { type: "ephemeral" });
  assert.equal(body.max_tokens, 16000);
  for (const absent of ["fallbacks", "temperature", "thinking"]) assert.ok(!(absent in body), absent);
});

test("the totals give pass rates, first-program success, medians, spread and failure classes per set", () => {
  const base = { model: "m", language: "rust" as const, task: "t", kind: "solve" as const, verdict: null, cost: 0 };
  const records: RunRecord[] = [
    { ...base, set: "public", core: true, run: 1, solved: true, repairs: 0, tokens: { input: 100, output: 50 }, programs: [{ classes: [] }] },
    {
      ...base,
      set: "public",
      core: true,
      run: 2,
      solved: true,
      repairs: 2,
      tokens: { input: 300, output: 150 },
      programs: [{ classes: ["build failure"] }, { classes: ["wrong result"] }, { classes: [] }],
    },
    {
      ...base,
      set: "public",
      core: false,
      run: 1,
      solved: false,
      repairs: 1,
      tokens: { input: 10, output: 10 },
      programs: [{ classes: ["build failure"] }, { classes: ["trap"] }],
    },
    {
      ...base,
      set: "public",
      kind: "adversarial",
      core: true,
      run: 1,
      solved: null,
      verdict: "missed",
      repairs: 0,
      tokens: { input: 1, output: 1 },
      programs: [{ classes: [] }],
    },
    { ...base, set: "held-out", core: true, run: 1, solved: false, repairs: 0, tokens: { input: 1, output: 1 }, programs: [{ classes: ["wrong result"] }] },
  ];
  const [held, pub] = summarize(records, 2);
  assert.deepEqual([held.set, pub.set], ["held-out", "public"]);
  assert.equal(held.solve.all.last_program_failures["wrong result"], 1);
  const all = pub.solve.all;
  assert.deepEqual([all.runs, all.solved, all.median_repairs_solved, all.median_tokens_solved], [3, 2, 1, 300]);
  assert.ok(Math.abs((all.pass_rate ?? 0) - 2 / 3) < 1e-9);
  assert.ok(Math.abs((all.first_program_pass_rate ?? 0) - 1 / 3) < 1e-9);
  assert.deepEqual(all.pass_rate_by_run, [0.5, 1]);
  assert.equal(all.pass_rate_stddev, 0.25);
  assert.deepEqual(all.first_program_failures, { "build failure": 2 });
  assert.deepEqual(all.last_program_failures, { trap: 1 });
  assert.deepEqual(all.every_program_failures, { "build failure": 2, "wrong result": 1, trap: 1 });
  assert.deepEqual(all.repairs_by_class, { "build failure": 2, "wrong result": 1 });
  assert.deepEqual([pub.solve.core.runs, pub.solve.core.solved, pub.solve.non_core.runs, pub.solve.non_core.solved], [2, 2, 1, 0]);
  assert.deepEqual(pub.adversarial, { runs: 1, caught_at_build: 0, caught_at_run_time: 0, missed: 1, refused: 0 });
  assert.deepEqual([median([3, 1, 2]), median([1, 2, 3, 4]), median([])], [2, 2.5, null]);
});
