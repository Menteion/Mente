// Tests of the benchmark harness's task reader, signature mapping and prompts, with no network.
// Its Rust and Odin rendering and the measured-run driver's parts are tested here too, with no
// toolchain: the workflow "Harness" builds and runs the fixtures through rustc and odin.

import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { inspect } from "node:util";
import { budgetedCall, callCost, KNOWN_MODELS, MAX_TOKENS, NETWORK_RETRIES, NetworkError, parseModels, requestBody, reserve, RunAnomaly, send, settle, worstCost, type Budget, type Usage } from "./api.ts";
import { Broker, httpPost, LOG_LINE_MAX, lineReader, logLine, readLimits, scrub, serve, shape, thinkingBlocks, unpricedField, usageCost, utf8Length, worstCost as brokerWorst, wrongModel, type Answer, type Price } from "./broker.ts";
import { BrokerBroken, BrokerRefused, connectBroker, refusalOf } from "./brokered.ts";
import {
  BASH,
  REPOSITORY,
  SANDBOX,
  SANDBOX_CONF,
  caseResult,
  checkUnderRuns,
  childEnv,
  cleanOutput,
  failureClass,
  readSandboxRoots,
  sandboxRoots,
  insideAny,
  menteCaseResult,
  namedCommand,
  printedOutcome,
  runProcess,
  sandboxed,
  trapText,
  verdictOf,
  type Proc,
} from "./build.ts";
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
import { failureOf, firstFailure, HeldOutError, heldOut, heldOutAsync, median, summarize, type RunRecord } from "./report.ts";
import { own, setOwn } from "../own.mjs";
import {
  benchmarkTasks,
  extractProgram,
  feedbackPrompt,
  hasTodo,
  heldOutTasks,
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
  // A retired task says so in its front matter, with or without a reason, and none other is.
  assert.equal(task.retired, false);
  assert.equal(parseTask(TASK.replace("core: false", "core: false\nretired: 2026-11-16 Changing the corpus")).retired, true);
  assert.equal(parseTask(TASK.replace("core: false", "core: false\nretired:")).retired, true);
  assert.equal(parseTask(TASK.replace("# Clamp to a range", "# Clamp to a range\n\nretired: in the body, not the front matter")).retired, false);
  // And a measured run gives the agents no retired task, public or held out.
  const of = (id: string, more = "") => parseTask(TASK.replace("id: clamp", `id: ${id}`).replace("core: false", `core: false${more}`));
  const tasks = [of("kept"), of("gone", "\nretired: 2026-11-16 broken"), { ...of("later"), milestone: "M2" }];
  assert.deepEqual(benchmarkTasks(tasks, "M1", "all").map((x) => x.id), ["kept"]);
  assert.deepEqual(benchmarkTasks(tasks, "M1", "kept").map((x) => x.id), ["kept"]);
  assert.throws(() => benchmarkTasks(tasks, "M1", "gone"), /no public M1 task gone/);
  assert.throws(() => benchmarkTasks(tasks, "M1", "kept, nothing"), /no public M1 task nothing/);
  const held = [{ ...of("h1"), set: "held-out", kind: "solve" }, { ...of("h2", "\nretired: 2026-11-16 broken"), set: "held-out", kind: "solve" }, { ...of("h3"), set: "held-out", kind: "adversarial" }];
  assert.deepEqual(heldOutTasks(held, "M1").map((x) => x.id), ["h1"]);
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
    BASH,
    [resolve("benchmark/harness/sandbox.sh"), resolve("out/w"), "rustc", "-o", "x", "main.rs"],
  ]);
  assert.deepEqual(sandboxed("sb.sh", "out/w", "out/w/main", ["0"]), [
    BASH,
    [resolve("sb.sh"), resolve("out/w"), resolve("out/w/main"), "0"],
  ]);
  // The sandbox takes no path to hide: what it covers comes from its root-owned configuration.
  assert.equal(sandboxed("sb.sh", "/t/w", "odin", ["build", "."]).at(1)?.includes("--hide"), false);
});

test("an output directory outside the sandbox's runs root is refused, and the roots are read from its configuration", () => {
  assert.equal(insideAny("/r/repo", ["/r/repo"]), true);
  assert.equal(insideAny("/r/repo/bench-out", ["/r/repo"]), true);
  assert.equal(insideAny("/r/repository-out", ["/r/repo"]), false);
  assert.equal(insideAny("/t/bench-out", ["/r/repo", "/r/held"]), false);
  // A link inside a root that points out of it lies outside: the paths are compared as the file
  // system resolves them (the review of aafe59b: a held-out-set link to $RUNNER_TEMP passed for the
  // hidden workspace).
  const linked = mkdtempSync(join(tmpdir(), "mente-inside-"));
  try {
    mkdirSync(join(linked, "root"));
    mkdirSync(join(linked, "elsewhere"));
    symlinkSync(join(linked, "elsewhere"), join(linked, "root", "held-out-set"), "junction");
    assert.equal(insideAny(join(linked, "root", "held-out-set"), [join(linked, "root")]), false, "a link out of the root passed for inside it");
    assert.equal(insideAny(join(linked, "root"), [join(linked, "root")]), true);
  } finally {
    rmSync(linked, { recursive: true, force: true });
  }
  const roots = sandboxRoots("hide /w/repo\nhide /t/broker\nruns /t/runs\n");
  assert.deepEqual(roots, { hide: ["/w/repo", "/t/broker"], runs: "/t/runs" });
  assert.throws(() => sandboxRoots("hide /w/repo\n"), /names no runs root/);
  // Strictly under the runs root, which the sandbox covers but for each build's own directory.
  assert.doesNotThrow(() => checkUnderRuns(resolve("/t/runs/bench"), roots));
  for (const out of ["/t/runs", "/t/other", "/t/runs-out", "/w/repo/bench-out"]) assert.throws(() => checkUnderRuns(resolve(out), roots), /must lie under the sandbox's runs root/, out);
  assert.ok(existsSync(join(REPOSITORY, "benchmark", "README.md")), "REPOSITORY is this repository's root");
});

test("no process that runs beside the key starts mn but through the sandbox", async () => {
  // The final review of PR 137: failureClass ran `mn explain` directly, as the runner's user,
  // beside the process that holds the key. mn is code of the branch; the sandbox is its only cage.
  const tools = { mn: "mn", native: false, rustc: "rustc", rustToolchain: "", odin: "odin", sandbox: "", out: "" };
  await assert.rejects(failureClass(tools, "E0001"), /outside the sandbox/);
  // Every module a job that holds the key runs starts mn only through `sandboxed(...)`: no spawn
  // takes mn as its command.
  const direct = /\b(?:runProcess|spawnSync|spawn|execFileSync|execFile|execSync|exec)\(\s*(?:tools\.mn|mn)\b/;
  for (const file of ["bench.ts", "build.ts", "prelim.ts"]) {
    const text = readFileSync(join(REPOSITORY, "benchmark", "harness", file), "utf8");
    assert.doesNotMatch(text, direct, `${file} starts mn directly, outside the sandbox`);
  }
  for (const plant of ['runProcess(mn, ["explain", code])', 'spawnSync(tools.mn, ["check"])', "spawn( mn, args)"]) assert.match(plant, direct, plant);
  // And by what runs, not how it is spelled (the review of 35bbe0f: `const c = tools.mn` and an
  // environment of `process.env` got past the spelling): runProcess, the harness's one way to
  // start a process, starts only a named command while a secret is in reach, the benchmark's
  // sandbox or a toolchain's probe; and the review of b2be87b: a shell running mn, a copy of mn
  // under another name, and another script given to bash got past a refusal of mn by its name.
  const refused: [string, string[]][] = [
    [tools.mn, ["explain", "E0001"]],
    [join("target", "release", "mn.exe"), []],
    [BASH, ["-c", 'mn explain "$1"', "x", "E0001"]],
    [BASH, [join(REPOSITORY, "benchmark", "harness", "leak.sh"), "/tmp", "mn", "explain"]],
    [BASH, [SANDBOX]],
    // The review of 41a54b6: bash found through PATH, where ~/.local/bin comes first.
    ["bash", [SANDBOX, ".", tools.mn, "explain", "E0001"]],
    [process.execPath, ["-e", "console.log(1)"]],
    [join("bin", "rustc"), ["-o", "x", "main.rs"]],
    [join("bin", "rustc"), ["--version", "--print", "sysroot"]],
    [join("copy", "rustc"), ["--print", "cfg"]],
    ["odin", ["run", "."]],
  ];
  for (const [c, a] of refused) assert.ok(!namedCommand(c, a), `${c} ${a.join(" ")} is taken for a named command`);
  const named: [string, string[]][] = [
    sandboxed(SANDBOX, ".", tools.mn, ["explain", "E0001"], [REPOSITORY]),
    sandboxed(join("benchmark", "harness", "sandbox.sh"), ".", "rustc", ["-o", "main", "main.rs"]),
    [join("toolchain", "bin", "rustc"), ["--version"]],
    [join("toolchain", "bin", "rustc.exe"), ["--print", "sysroot"]],
    ["odin", ["version"]],
  ];
  for (const [c, a] of named) assert.ok(namedCommand(c, a), `${c} ${a.join(" ")} is not taken for a named command`);
  process.env.BENCH_TEST_API_KEY = "secret";
  try {
    for (const [c, a] of refused) await assert.rejects(runProcess(c, a, { env: process.env, timeoutMs: 5000 }), /no command the harness starts beside a secret/);
    // A named one starts, here a probe of a rustc that is not there, and fails as a process.
    const p = await runProcess(join(REPOSITORY, "target", "no-such-toolchain", "rustc"), ["--version"], { env: { ...process.env }, timeoutMs: 30_000 });
    assert.equal(p.status, null);
  } finally {
    delete process.env.BENCH_TEST_API_KEY;
  }
  // A copy of build.ts loaded afresh, which records whether a secret was in reach as it loaded.
  let loads = 0;
  const fresh = async () => (await import(`./build.ts?load=${++loads}`)) as typeof import("./build.ts");
  const held = Object.entries(process.env).filter(([name]) => /key|token|secret|password|credential/i.test(name));
  for (const [name] of held) delete process.env[name];
  try {
    // A secret in reach as the module loaded binds it after the secret leaves the environment,
    // where it stays in memory (the review of 41a54b6: `delete process.env.ANTHROPIC_API_KEY`).
    process.env.BENCH_TEST_API_KEY = "secret";
    const loadedBeside = await fresh();
    delete process.env.BENCH_TEST_API_KEY;
    await assert.rejects(loadedBeside.runProcess(process.execPath, ["-e", "1"], { timeoutMs: 5000 }), /no command the harness starts beside a secret/);
    // With no secret in reach, then or now, any command runs, and no child is given a variable
    // that could hold one, whatever environment it is handed.
    const loadedAlone = await fresh();
    const p = await loadedAlone.runProcess(process.execPath, ["-e", "console.log(process.env.BENCH_TEST_TOKEN ?? 'none')"], { env: { ...process.env, BENCH_TEST_TOKEN: "" }, timeoutMs: 30_000 });
    assert.equal(p.stdout.trim(), "none");
  } finally {
    delete process.env.BENCH_TEST_API_KEY;
    for (const [name, value] of held) process.env[name] = value;
  }
});

test("own and setOwn reach only own properties, whatever the key makes of itself", () => {
  // The re-check of da58409: a key whose toString answered "a" first and "__proto__" after passed
  // the check of an own "a" and then read Object.prototype, which setOwn then wrote.
  const turning = () => {
    let calls = 0;
    return { toString: () => (calls++ === 0 ? "a" : "__proto__") };
  };
  assert.equal(own({ a: 1 }, turning()), 1);
  assert.equal(own({}, turning()), undefined);
  assert.equal(own({ a: 1 }, "__proto__"), undefined);
  assert.equal(own(() => 0, "constructor"), undefined);
  assert.equal(own([5, 6], 1), 6);
  assert.equal(own("ab", 1), "b");
  assert.equal(own(null, "a"), undefined);
  const target = { a: 1 };
  setOwn(target, turning(), 2);
  assert.equal(target.a, 2);
  setOwn(target, "__proto__", { shell: "/bin/sh" });
  assert.equal(Object.getPrototypeOf(target), Object.prototype, "setOwn changed a prototype");
  assert.equal(({} as Record<string, unknown>).shell, undefined, "setOwn polluted Object.prototype");
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
  const cost = callCost({ input_tokens: 1e6, output_tokens: 1e6, cache_creation_input_tokens: 1e6, cache_read_input_tokens: 1e6 }, sonnet, 0);
  assert.ok(Math.abs(cost - 14.6) < 1e-9, String(cost));
  // A usage the harness cannot read counts at the call's worst, as the broker counts it.
  assert.equal(callCost({ input_tokens: 10 } as Usage, sonnet, 0.5), 0.5);
  // The worst case: every byte of the body an input token written to the cache, a character
  // beyond ASCII its bytes, and the most output; the broker's, from the same body.
  const small = { x: "\u00e9" };
  assert.equal(JSON.stringify(small).length, 9);
  assert.ok(Math.abs(worstCost(small, sonnet) - (10 * 2 * 1.25 + MAX_TOKENS * 10) / 1e6) < 1e-12, String(worstCost(small, sonnet)));
  assert.equal(worstCost(small, sonnet), brokerWorst(JSON.stringify(small), MAX_TOKENS, sonnet, readLimits(LIMITS_TEXT), 0));
  // A thinking block of an earlier reply counts at maxTokens input tokens more, on both sides.
  const replied = requestBody("claude-sonnet-5-5", "S", [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "s".repeat(200) }, { type: "text", text: "y" }] }, { role: "user", content: "z" }]);
  assert.equal(worstCost(replied, sonnet), brokerWorst(JSON.stringify(replied), MAX_TOKENS, sonnet, readLimits(LIMITS_TEXT), 1));
  assert.ok(worstCost(replied, sonnet) - brokerWorst(JSON.stringify(replied), MAX_TOKENS, sonnet, readLimits(LIMITS_TEXT), 0) > (MAX_TOKENS * 2 * 1.25) / 1e6 - 1e-12);
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

// ------------------------------------------------------------------ the broker, which holds the key

/** permission.json: the Node version and the broker's flags, defined once. */
const PERMISSION = JSON.parse(readFileSync(join(REPOSITORY, "benchmark", "harness", "permission.json"), "utf8"));
/** The broker's flags, each --allow-fs-read given in turn the paths a test allows. */
const brokerFlags = (reads: string[]): string[] => {
  let k = 0;
  return (PERMISSION.broker.flags as string[]).map((f) => (f.startsWith("--allow-fs-read=") ? `--allow-fs-read=${reads.at(k++) ?? reads.at(0)}` : f));
};
/** The environment `env -i PATH="$PATH"` leaves, with what Windows needs to start a process at all. */
const cleared = (): NodeJS.ProcessEnv => ({ PATH: process.env.PATH, ...(process.platform === "win32" ? { SystemRoot: process.env.SystemRoot } : {}) });

test("the harness's tests run on the Node version the broker's permission model is pinned to", () => {
  assert.match(PERMISSION.node, /^\d+\.\d+\.\d+$/);
  // In CI the policy requires every job that runs these tests to set up exactly this version.
  if (process.env.CI) assert.equal(process.version, `v${PERMISSION.node}`, "the permission tests ran on another Node than the broker's");
});

/**
 * The routes out of the broker, each a planted script that tries one and prints whether it got
 * through: a process in every form, a process put in this one's place, a worker, an addon, WASI,
 * the inspector, the bindings, a file read or written beyond what is allowed, code from a string,
 * and child_process through require in a module and in a CommonJS script.
 */
const ESCAPES: [string, string][] = [
  ["spawnSync", `const r = (await import("node:child_process")).spawnSync(process.execPath, ["-e", "0"]); if (r.error) throw r.error;`],
  ["execFileSync", `(await import("node:child_process")).execFileSync(process.execPath, ["-e", "0"]);`],
  ["exec", `const cp = await import("node:child_process"); await new Promise((ok, no) => { cp.exec(JSON.stringify(process.execPath) + " -e 0", (e) => (e ? no(e) : ok())); });`],
  ["execve", `process.execve(process.execPath, [process.execPath, "-e", "console.log(JSON.stringify({ outcome: 'allowed', code: '' }))"]);`],
  ["worker", `const { Worker } = await import("node:worker_threads"); const w = new Worker("0", { eval: true }); await new Promise((ok, no) => { w.on("exit", ok); w.on("error", no); });`],
  ["dlopen", `try { process.dlopen({ exports: {} }, "missing.node"); } catch (e) { if (e.code === "ERR_DLOPEN_DISABLED" || e.code === "ERR_ACCESS_DENIED") throw e; }`],
  ["wasi", `const { WASI } = await import("node:wasi"); new WASI({ version: "preview1" });`],
  ["inspector", `const i = await import("node:inspector"); i.open(0, "127.0.0.1"); i.close();`],
  ["binding", `process.binding("fs");`],
  ["getBuiltinModule", `const r = process.getBuiltinModule("node:child_process").spawnSync(process.execPath, ["-e", "0"]); if (r.error) throw r.error;`],
  ["createRequire", `const { createRequire } = await import("node:module"); const r = createRequire(import.meta.url)("node:child_process").spawnSync(process.execPath, ["-e", "0"]); if (r.error) throw r.error;`],
  ["read", `(await import("node:fs")).readFileSync(process.argv.at(2));`],
  ["write", `(await import("node:fs")).writeFileSync(process.argv.at(3), "x");`],
  ["codegen", `Function("return 1")();`],
  ["sqlite", `const m = process.getBuiltinModule("node:sqlite"); if (m === undefined) throw Object.assign(new Error("no node:sqlite"), { code: "ERR_UNKNOWN_BUILTIN_MODULE" }); new m.DatabaseSync(process.argv.at(3) + ".db").exec("create table t (a)");`],
];
/** A planted script: the route's code, and one line saying whether it got through. */
const planted = (code: string) =>
  `const report = (outcome, code = "") => console.log(JSON.stringify({ outcome, code }));\ntry {\n  ${code}\n  report("allowed");\n} catch (e) {\n  report("refused", String(e?.code ?? e?.message ?? e));\n}\n`;
/** The same through require, in a CommonJS script. */
const PLANTED_CJS = `const report = (outcome, code = "") => console.log(JSON.stringify({ outcome, code }));\ntry {\n  const r = require("child_process").spawnSync(process.execPath, ["-e", "0"]);\n  if (r.error) throw r.error;\n  report("allowed");\n} catch (e) {\n  report("refused", String(e && (e.code || e.message)));\n}\n`;

test("the broker's flags refuse every route out of it, which each script takes without them", () => {
  const dir = mkdtempSync(join(tmpdir(), "mente-escapes-"));
  try {
    const other = join(REPOSITORY, "package.json");
    const written = join(dir, "written.txt");
    const run = (file: string, flags: string[], env: NodeJS.ProcessEnv = cleared()) => {
      const r = spawnSync(process.execPath, [...flags, file, other, written], { cwd: dir, env, encoding: "utf8", timeout: 60_000 });
      const last = r.stdout.trim().split("\n").at(-1) ?? "";
      try {
        return JSON.parse(last) as { outcome: string; code: string };
      } catch {
        return { outcome: "no report", code: `status ${r.status}: ${r.stderr.trim().slice(-300)}` };
      }
    };
    const cases: [string, string][] = [...ESCAPES.map(([route, code]): [string, string] => [`${route}.mjs`, planted(code)]), ["require.cjs", PLANTED_CJS]];
    for (const [name, text] of cases) {
      const file = join(dir, name);
      writeFileSync(file, text);
      // Under the broker's flags, as the broker runs: the planted script alone readable.
      const confined = run(file, brokerFlags([file]));
      assert.equal(confined.outcome, "refused", `${name} got through the broker's flags: ${JSON.stringify(confined)}`);
      if (name === "codegen.mjs") assert.match(confined.code, /Code generation from strings disallowed/, name);
      else if (name === "dlopen.mjs") assert.equal(confined.code, "ERR_DLOPEN_DISABLED", name);
      else if (name === "sqlite.mjs") assert.equal(confined.code, "ERR_UNKNOWN_BUILTIN_MODULE", name);
      else if (name === "execve.mjs" && process.platform === "win32") assert.match(confined.code, /ERR_ACCESS_DENIED|ERR_FEATURE_UNAVAILABLE_ON_PLATFORM/, name);
      else assert.equal(confined.code, "ERR_ACCESS_DENIED", `${name} was refused, but not by the permission model: ${confined.code}`);
      // Without them, as a control, the same script gets through, so that each refusal proves
      // something; Windows cannot put a process in another's place.
      if (name === "execve.mjs" && process.platform === "win32") continue;
      const free = run(file, []);
      assert.equal(free.outcome, "allowed", `${name} cannot take its route even without the broker's flags, so its refusal proves nothing: ${JSON.stringify(free)}`);
    }
    // NODE_OPTIONS can add --allow-child-process to the flags, which is why the broker starts with
    // an environment cleared but for PATH (env -i, which the policy pins).
    const spawnFile = join(dir, "spawnSync.mjs");
    const widened = run(spawnFile, brokerFlags([spawnFile]), { ...cleared(), NODE_OPTIONS: "--allow-child-process" });
    assert.equal(widened.outcome, "allowed", "NODE_OPTIONS no longer widens the permission model; env -i may be dropped from the pin");
    // The flags themselves name none of the refused ones.
    for (const f of PERMISSION.broker.flags as string[]) for (const r of PERMISSION.refused as string[]) assert.ok(!f.startsWith(r), `the broker's flags hold ${r}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

/**
 * The routes the broker's flags leave open on this Node, each tried under them and recorded as
 * open, so that a Node that closes one, or a change that opens another, is noticed. None is
 * reachable from a request: the broker's code takes none of them, and what it is sent is cut to a
 * shape that steers no code (broker.ts, shape). Each is listed in the design notes.
 */
const KNOWN_OPEN: [string, string][] = [
  ["a module from a data: URL", `const m = await import("data:text/javascript,export default 41+1"); if (m.default !== 42) throw new Error("no");`],
  ["vm.runInThisContext", `if (process.getBuiltinModule("node:vm").runInThisContext("41+1") !== 42) throw new Error("no");`],
  ["vm.compileFunction", `if (process.getBuiltinModule("node:vm").compileFunction("return 41+1")() !== 42) throw new Error("no");`],
  ["a connection to any socket", `const net = process.getBuiltinModule("node:net"); await new Promise((ok, no) => { const c = net.connect(process.argv.at(2)); c.on("connect", () => { c.end(); ok(); }); c.on("error", no); });`],
  ["a listening socket, a new file system entry", `const net = process.getBuiltinModule("node:net"); await new Promise((ok, no) => { const s = net.createServer(); s.on("error", no); s.listen(process.argv.at(3), () => s.close(ok)); });`],
  ["a signal to another process of the same user", `process.kill(Number(process.argv.at(4)), 0);`],
];

test("the routes the broker's flags leave open are the ones the design notes list", async () => {
  const dir = mkdtempSync(join(tmpdir(), "mente-open-"));
  const server = net.createServer((c) => c.end());
  const target = socketPath(dir, "target");
  await new Promise<void>((done) => server.listen(target, done));
  try {
    for (const [route, code] of KNOWN_OPEN) {
      const file = join(dir, "open.mjs");
      writeFileSync(file, planted(code));
      const r = await new Promise<string>((done) => {
        const child = spawn(process.execPath, [...brokerFlags([file]), file, target, socketPath(dir, "made"), String(process.pid)], { cwd: dir, env: cleared() });
        let out = "";
        child.stdout.on("data", (d) => (out += d));
        child.on("close", () => done(out.trim().split("\n").at(-1) ?? ""));
      });
      assert.equal(JSON.parse(r || '{"outcome":"no report"}').outcome, "allowed", `${route} is closed on this Node: move it to ESCAPES and out of the design notes' open routes (${r})`);
    }
  } finally {
    await new Promise((done) => server.close(done));
    rmSync(dir, { recursive: true, force: true });
  }
});

/** The real limits, and a broker over a fake transport that records each POST. */
const LIMITS_TEXT = readFileSync(join(REPOSITORY, "benchmark", "harness", "broker-limits.json"), "utf8");
const KEY = "sk-ant-test-0123456789abcdef";
const fakeBroker = (budget: number, answer: (body: string) => Promise<Answer> | Answer, limits = readLimits(LIMITS_TEXT)) => {
  const posts: { url: string; headers: Record<string, string>; body: string }[] = [];
  const notes: string[] = [];
  const broker = new Broker(
    KEY,
    limits,
    budget,
    async (url, headers, body) => {
      posts.push({ url, headers, body });
      return answer(body);
    },
    (line) => notes.push(line),
  );
  return { broker, posts, notes };
};

/**
 * Every usage the API has answered with that the broker has met, from runs with the real key
 * (benchmark/harness/fixtures/usage.json), one to a line: the next real answer is one line more.
 */
const REAL_USAGE = JSON.parse(readFileSync(join(REPOSITORY, "benchmark", "harness", "fixtures", "usage.json"), "utf8")) as { from: string; model: string; usage: Record<string, unknown> }[];
const request = (id: number, body: object) => JSON.stringify({ id, body });
const haiku = (max = 100) => ({ model: "claude-haiku-5-5", max_tokens: max, messages: [{ role: "user", content: "hi" }] });
/** What an honest API answers to the body the broker sent: `fields`, written by the model the body names. */
const asAsked = (body: string, fields: object) => JSON.stringify({ model: JSON.parse(body).model, ...fields });

test("the broker reads its limits, and refuses limits that are not whole", () => {
  const limits = readLimits(LIMITS_TEXT);
  assert.equal(limits.url, "https://api.anthropic.com/v1/messages");
  assert.ok(limits.models.has("claude-haiku-5-5"));
  // The harness reads its prices and caps from the same file.
  assert.equal(limits.maxTokens, MAX_TOKENS);
  assert.equal(own(KNOWN_MODELS, "claude-sonnet-5-5").output, (limits.models.get("claude-sonnet-5-5") as Price).output);
  // Each model it names has one ID and no dated snapshot, so each answers as itself alone.
  assert.deepEqual([...limits.answeredAs], [...limits.models.keys()].map((id) => [id, [id]]));
  const raw = JSON.parse(LIMITS_TEXT);
  for (const [what, change] of [
    ["another address", { url: "https://example.com/v1/messages" }],
    ["no version", { version: "latest" }],
    ["a price that is not positive", { models: { "claude-haiku-5-5": { input: 0, output: 1, cacheRead: 1, answeredAs: ["claude-haiku-5-5"] } } }],
    ["no model", { models: {} }],
    ["no request cap", { maxRequests: 0 }],
    ["no dollar cap", { maxDollars: -1 }],
    ["no standard usage", { standardUsage: {} }],
    ["a standard usage value that is no string", { standardUsage: { speed: 1 } }],
    ["no cap on requests in flight", { maxInFlight: 0 }],
    ["no cap on connections", { maxConnections: 0 }],
    ["a cap on body bytes that is no whole number", { maxBodyBytes: 1.5 }],
    ["a cache write cheaper than an input token", { cacheWrite: 0.9 }],
    ["a cache read dearer than a cache write", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 2, answeredAs: ["claude-haiku-5-5"] } } }],
    ["a long-prompt factor below 1", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, longPrompt: { above: 10, factor: 0.5 }, answeredAs: ["claude-haiku-5-5"] } } }],
    ["a long-prompt threshold that is no whole number", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, longPrompt: { above: 1.5, factor: 5 }, answeredAs: ["claude-haiku-5-5"] } } }],
    // A model with no names an answer to it may carry, or names that are not that.
    ["a model with no answer names", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1 } } }],
    ["an empty list of answer names", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, answeredAs: [] } } }],
    ["an answer name that is no string", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, answeredAs: [5] } } }],
    ["an empty answer name", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, answeredAs: [""] } } }],
    ["an answer name twice", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, answeredAs: ["claude-haiku-5-5", "claude-haiku-5-5"] } } }],
    ["answer names that are no list", { models: { "claude-haiku-5-5": { input: 1, output: 1, cacheRead: 0.1, answeredAs: "claude-haiku-5-5" } } }],
  ] as [string, object][]) {
    assert.throws(() => readLimits(JSON.stringify({ ...raw, ...change })), /broker-limits\.json/, `limits with ${what} are read`);
  }
});

test("the broker sends a request within the limits with the key, and refuses one beyond them", async () => {
  const ok = { status: 200, retryAfter: "", text: JSON.stringify({ model: "claude-haiku-5-5", content: [], usage: { input_tokens: 1000, output_tokens: 1000 } }) };
  const { broker, posts } = fakeBroker(1, () => ok);
  const sent = JSON.parse(await broker.handle(request(1, haiku())));
  assert.deepEqual([sent.id, sent.status], [1, 200]);
  assert.equal(posts.length, 1);
  assert.equal(posts[0]?.url, "https://api.anthropic.com/v1/messages");
  assert.equal(posts[0]?.headers["x-api-key"], KEY);
  assert.equal(posts[0]?.headers["anthropic-version"], readLimits(LIMITS_TEXT).version);
  // Charged by the usage the API reported: 1000 input and 1000 output tokens of Haiku.
  assert.ok(Math.abs(broker.spent - (1000 * 0.1 + 1000 * 0.5) / 1e6) < 1e-12, `spent ${broker.spent}`);
  for (const [what, line] of [
    ["a model it does not name", request(2, { ...haiku(), model: "gpt-x" })],
    ["no model", request(3, { max_tokens: 10, messages: [] })],
    ["max_tokens past the cap", request(4, haiku(MAX_TOKENS + 1))],
    ["max_tokens of 0", request(5, haiku(0))],
    ["max_tokens that is no whole number", request(6, { ...haiku(), max_tokens: 1.5 })],
    ["a streamed request", request(7, { ...haiku(), stream: true })],
    ["no body", JSON.stringify({ id: 8 })],
    ["a line that is no JSON", "not json"],
    // What the independent review of 5ad4091 had forwarded, reserving about a third of the cap.
    ["fast mode, a 1-hour cache, web search and a document by URL", request(9, REVIEWED_BODY)],
  ] as [string, string][]) {
    const answer = JSON.parse(await broker.handle(line));
    assert.equal(typeof answer.refused, "string", `the broker sent ${what}`);
    assert.equal(answer.exhausted, undefined, `the broker read ${what} as the run's limits reached`);
  }
  assert.equal(posts.length, 1, "a refused request reached the API");
  // The run's dollar cap is its budget or maxDollars, whichever is less.
  assert.equal(fakeBroker(1e9, () => ok).broker.cap, readLimits(LIMITS_TEXT).maxDollars);
});

test("the broker spends within the run's budget and requests, counting a lost answer at its most", async () => {
  // A budget a Haiku request of 16000 output tokens could pass at most twice ($0.008 each).
  let release = () => {};
  const held = new Promise<void>((done) => {
    release = done;
  });
  const slow = fakeBroker(0.017, async (body) => {
    await held;
    return { status: 200, retryAfter: "", text: asAsked(body, { usage: { input_tokens: 10, output_tokens: 10 } }) };
  });
  // Two requests in flight reserve their most; a third would pass the budget while they are.
  const first = slow.broker.handle(request(1, haiku(MAX_TOKENS)));
  const second = slow.broker.handle(request(2, haiku(MAX_TOKENS)));
  const third = JSON.parse(await slow.broker.handle(request(3, haiku(MAX_TOKENS))));
  assert.match(third.refused, /past its \$0\.017/);
  assert.equal(third.exhausted, true, "a refusal at the budget does not say the run's limits are reached");
  release();
  await Promise.all([first, second]);
  // Settled at what they cost, the budget has room again.
  assert.equal(JSON.parse(await slow.broker.handle(request(4, haiku(MAX_TOKENS)))).status, 200);
  // A lost answer counts at its most; an error status costs nothing.
  const lost = fakeBroker(1, () => {
    throw new Error("connection reset");
  });
  const answer = JSON.parse(await lost.broker.handle(request(1, haiku(MAX_TOKENS))));
  assert.equal(answer.status, -1);
  assert.ok(lost.broker.spent > 0.0079, `a lost answer counted ${lost.broker.spent}`);
  const failing = fakeBroker(1, () => ({ status: 529, retryAfter: "3", text: "{}" }));
  assert.equal(JSON.parse(await failing.broker.handle(request(1, haiku()))).retryAfter, "3");
  assert.equal(failing.broker.spent, 0);
  // At most maxRequests requests.
  const counted = fakeBroker(1, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, {}) }), { ...readLimits(LIMITS_TEXT), maxRequests: 2 });
  for (const id of [1, 2]) assert.equal(JSON.parse(await counted.broker.handle(request(id, haiku()))).status, 200);
  const past = JSON.parse(await counted.broker.handle(request(3, haiku())));
  assert.match(past.refused, /past the run's 2/);
  assert.equal(past.exhausted, true, "a refusal at the request cap does not say the run's limits are reached");
});

test("the broker takes the key out of every byte it writes", async () => {
  // A transport that echoes the key back, in the answer, its header and an error.
  const echo = fakeBroker(1, (body) => ({ status: 400, retryAfter: KEY, text: `the key was ${KEY}, the body ${body}` }));
  const line = await echo.broker.handle(request(1, { ...haiku(), messages: [{ role: "user", content: `the key is ${KEY}` }] }));
  assert.ok(!line.includes(KEY), `the key in the broker's answer: ${line}`);
  assert.match(line, /\[the API key\]/);
  const thrown = fakeBroker(1, () => {
    throw new Error(`refused ${KEY}`);
  });
  assert.ok(!(await thrown.broker.handle(request(1, haiku()))).includes(KEY), "the key in an error the broker wrote");
  assert.ok(!(await echo.broker.handle(`{"id": "${KEY}"`)).includes(KEY), "the key in the answer to a line that is no JSON");
  assert.equal(scrub(`a ${KEY} b ${KEY}`, KEY), "a [the API key] b [the API key]");
  assert.throws(() => new Broker("", readLimits(LIMITS_TEXT), 1, async () => ({ status: 200, retryAfter: "", text: "" })), /no API key/);
  // A redirect is never followed: the final pass of 05fe2f6 found fetch's default follows one, and
  // a cross-origin redirect drops Authorization but sends x-api-key on. A fake transport that
  // follows redirects unless told not to, as fetch does, shows the broker never lets it.
  const seen: { url: string; key: string | null }[] = [];
  const redirecting = (async (url: string | URL | Request, init?: RequestInit) => {
    const headers = new Headers(init?.headers);
    seen.push({ url: String(url), key: headers.get("x-api-key") });
    if (init?.redirect === "error") throw new TypeError("fetch failed: a redirect");
    seen.push({ url: "https://elsewhere.example/v1/messages", key: headers.get("x-api-key") });
    return new Response("{}", { status: 200 });
  }) as typeof fetch;
  const redirected = new Broker(KEY, readLimits(LIMITS_TEXT), 1, httpPost(redirecting));
  const lost = JSON.parse(await redirected.handle(request(1, haiku())));
  assert.equal(lost.status, -1, "the broker followed a redirect");
  assert.deepEqual(seen.map((s) => s.url), ["https://api.anthropic.com/v1/messages"], "the key went to another host");
  await assert.rejects(httpPost(redirecting)("https://elsewhere.example/v1/messages", {}, "{}"), /no request goes to/);
  // A key JSON would write otherwise than as itself is refused, so that scrubbing an answer line,
  // which is JSON, finds every copy; the refusal does not print it.
  for (const odd of ['sk-ant-"quoted"-0123', "sk-ant-back\\slash-0123", "sk-ant-\u00e9-0123456"]) {
    assert.throws(
      () => new Broker(odd, readLimits(LIMITS_TEXT), 1, async () => ({ status: 200, retryAfter: "", text: "" })),
      (e: Error) => /no API key/.test(e.message) && !e.message.includes(odd),
      `the broker took the key ${odd}`,
    );
  }
});

test("the harness stops at its budget before the broker refuses, and as at its budget when the broker does", async () => {
  // The harness and the broker reckon one body alike. Each call's body holds 30000 characters
  // of two bytes each, so that a reckoning by characters would put a call within a budget that
  // the broker's, by bytes, puts beyond it: with a budget of 1.6 calls at their most, and answers
  // billed at their most but for the cache write, one call fits and the second does not.
  const sonnet = own(KNOWN_MODELS, "claude-sonnet-5-5");
  const bodyOf = (k: number) => requestBody("claude-sonnet-5-5", "é".repeat(30000), [{ role: "user", content: `call ${k}` }]);
  const limit = 1.6 * brokerWorst(JSON.stringify(bodyOf(0)), MAX_TOKENS, sonnet, readLimits(LIMITS_TEXT), 0);
  const most = (body: string) => ({ status: 200, retryAfter: "", text: asAsked(body, { content: [], usage: { input_tokens: new TextEncoder().encode(body).length, output_tokens: MAX_TOKENS } }) });
  const { broker } = fakeBroker(limit, most);
  const refusals: string[] = [];
  const post = async (body: object) => {
    const answer = JSON.parse(await broker.handle(request(1, body)));
    if (typeof answer.refused === "string") {
      refusals.push(answer.refused);
      throw new BrokerRefused(answer.refused, answer.exhausted === true);
    }
    return answer as Answer;
  };
  const budget: Budget = { limit, spent: 0, reserved: 0, stopped: false };
  let calls = 0;
  for (let k = 0; k < 10; k++) {
    if (!(await budgetedCall(post, budget, bodyOf(k), sonnet))) break;
    calls++;
  }
  assert.ok(calls === 1 && budget.stopped, `the harness made ${calls} calls`);
  assert.deepEqual(refusals, [], "the broker refused before the harness stopped");
  assert.ok(Math.abs(budget.spent - broker.spent) < 1e-12, `the harness counted $${budget.spent}, the broker $${broker.spent}`);
  // The broker's own limits, reached first: the call returns as at the budget, the harness's
  // budget stopped too.
  const capped = fakeBroker(10, most, { ...readLimits(LIMITS_TEXT), maxRequests: 1 }).broker;
  const cappedPost = async (body: object) => {
    const answer = JSON.parse(await capped.handle(request(1, body)));
    if (typeof answer.refused === "string") throw new BrokerRefused(answer.refused, answer.exhausted === true);
    return answer as Answer;
  };
  const wide: Budget = { limit: 10, spent: 0, reserved: 0, stopped: false };
  assert.ok(await budgetedCall(cappedPost, wide, requestBody("claude-sonnet-5-5", "S", [{ role: "user", content: "x" }]), sonnet));
  assert.equal(await budgetedCall(cappedPost, wide, requestBody("claude-sonnet-5-5", "S", [{ role: "user", content: "x" }]), sonnet), null);
  assert.deepEqual([wide.stopped, wide.reserved], [true, 0]);
  // An answer from another model than the one asked fails the call: billed, the harness counts it as
  // the broker does, at the requested model's worst, and then, like the broker, admits nothing more.
  const swapped = fakeBroker(10, () => ({ status: 200, retryAfter: "", text: JSON.stringify({ model: "claude-haiku-5-5", content: [], usage: { input_tokens: 1, output_tokens: 1 } }) })).broker;
  const swappedPost = async (body: object) => {
    const answer = JSON.parse(await swapped.handle(request(1, body)));
    if (typeof answer.refused === "string") throw refusalOf(answer);
    return answer as Answer;
  };
  const ledger: Budget = { limit: 10, spent: 0, reserved: 0, stopped: false };
  const asked = requestBody("claude-sonnet-5-5", "S", [{ role: "user", content: "x" }]);
  await assert.rejects(budgetedCall(swappedPost, ledger, asked, sonnet), (e: unknown) => e instanceof BrokerRefused && e.anomaly === "another model" && e.billed);
  assert.ok(Math.abs(ledger.spent - worstCost(asked, sonnet)) < 1e-12, `the harness counted $${ledger.spent}, not Sonnet's most`);
  assert.ok(Math.abs(ledger.spent - swapped.spent) < 1e-12, `the harness counted $${ledger.spent}, the broker $${swapped.spent}`);
  assert.deepEqual([ledger.reserved, ledger.stopped], [0, true]);
  assert.equal(await budgetedCall(swappedPost, ledger, asked, sonnet), null, "the harness made a call after another model's answer");
  assert.equal(swapped.requests, 1);
  // A request the broker does not send fails the run.
  await assert.rejects(budgetedCall(async () => Promise.reject(new BrokerRefused("a model it does not name", false)), { limit: 10, spent: 0, reserved: 0, stopped: false }, {}, sonnet), /does not name/);
  // A lost answer is sent again NETWORK_RETRIES times, each counted at its most, then fails.
  const lost: Budget = { limit: 10, spent: 0, reserved: 0, stopped: false };
  let tries = 0;
  const losing = async () => {
    tries++;
    return { status: -1, retryAfter: "", text: "the request failed" };
  };
  await assert.rejects(budgetedCall(losing, lost, {}, sonnet), NetworkError);
  assert.equal(tries, NETWORK_RETRIES + 1);
  assert.ok(Math.abs(lost.spent - tries * worstCost({}, sonnet)) < 1e-12, String(lost.spent));
});

/** The body the independent review of 5ad4091 had the broker forward, 338 bytes that bill far past their size. */
const REVIEWED_BODY = {
  model: "claude-opus-5-5",
  max_tokens: 16000,
  speed: "fast",
  cache_control: { type: "ephemeral", ttl: "1h" },
  tools: [{ type: "web_search_20260209", name: "web_search" }],
  messages: [{ role: "user", content: [{ type: "document", source: { type: "url", url: "https://example.com/big.pdf" } }] }],
};

test("the broker sends only the shape the harness makes, written anew, and refuses every other", () => {
  const limits = readLimits(LIMITS_TEXT);
  const refused = (body: unknown) => {
    const s = shape(body, limits);
    return "refused" in s ? s.refused : "";
  };
  // What the harness sends passes, and is written byte for byte as the harness wrote it, so that
  // the harness's reckoning of a request and the broker's are of one text.
  const bench = requestBody("claude-sonnet-5-5", "SYSTEM", [
    { role: "user", content: "write it" },
    { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "c2ln" }, { type: "text", text: "pub fn f() = 1" }] },
    { role: "user", content: "again" },
  ]);
  const sent = shape(bench, limits);
  assert.ok(!("refused" in sent), JSON.stringify(sent));
  assert.equal((sent as { text: string }).text, JSON.stringify(bench));
  const prelim = { model: "claude-haiku-5-5", max_tokens: MAX_TOKENS, system: "S", messages: [{ role: "user", content: "x" }] };
  assert.equal((shape(prelim, limits) as { text: string }).text, JSON.stringify(prelim));
  // A reply's text block with citations: null loses it; a redacted thinking block passes.
  const replied = { ...prelim, messages: [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "redacted_thinking", data: "ZGF0YQ==" }, { type: "text", text: "y", citations: null }] }] };
  assert.equal((shape(replied, limits) as { text: string }).text, JSON.stringify({ ...prelim, messages: [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "redacted_thinking", data: "ZGF0YQ==" }, { type: "text", text: "y" }] }] }));
  // The review's body names what goes past the shape.
  assert.match(refused(REVIEWED_BODY), /speed, tools/);
  // Each field the harness never sends, each one by itself.
  const base = { model: "claude-opus-5-5", max_tokens: 100, messages: [{ role: "user", content: "x" }] };
  for (const [what, body] of [
    ["fast mode", { ...base, speed: "fast" }],
    ["a tool", { ...base, tools: [{ type: "web_search_20260209", name: "web_search" }] }],
    ["metadata", { ...base, metadata: { user_id: "u" } }],
    ["a thinking setting", { ...base, thinking: { type: "adaptive" } }],
    ["an output setting", { ...base, output_config: { effort: "max" } }],
    ["inference in one region", { ...base, inference_geo: "us" }],
    ["a service tier", { ...base, service_tier: "priority" }],
    ["fallbacks", { ...base, fallbacks: "default" }],
    ["a stream", { ...base, stream: true }],
    ["a 1-hour cache", { ...base, cache_control: { type: "ephemeral", ttl: "1h" } }],
    ["a 1-hour cache on the system prompt", { ...base, system: [{ type: "text", text: "s", cache_control: { type: "ephemeral", ttl: "1h" } }] }],
    ["a system block that is not text", { ...base, system: [{ type: "document", source: { type: "url", url: "https://x" } }] }],
    ["a system prompt that is a number", { ...base, system: 1 }],
    ["a document by URL", { ...base, messages: [{ role: "user", content: [{ type: "document", source: { type: "url", url: "https://x/big.pdf" } }] }] }],
    ["an image by URL", { ...base, messages: [{ role: "user", content: [{ type: "image", source: { type: "url", url: "https://x/big.png" } }] }] }],
    ["a cache breakpoint inside a message", { ...base, messages: [{ role: "user", content: [{ type: "text", text: "x", cache_control: { type: "ephemeral", ttl: "1h" } }] }] }],
    ["a user block with citations", { ...base, messages: [{ role: "user", content: [{ type: "text", text: "x", citations: null }] }] }],
    ["a reply's text with citations", { ...base, messages: [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "text", text: "y", citations: [{ type: "char_location" }] }] }] }],
    ["a thinking block in a user message", { ...base, messages: [{ role: "user", content: [{ type: "thinking", thinking: "", signature: "s" }] }] }],
    ["a thinking block with more", { ...base, messages: [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "s", extra: 1 }] }] }],
    ["a tool use in a reply", { ...base, messages: [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "tool_use", id: "t", name: "n", input: {} }] }] }],
    ["a system message", { ...base, messages: [{ role: "user", content: "x" }, { role: "system", content: "y" }] }],
    ["a message with more than its role and content", { ...base, messages: [{ role: "user", content: "x", name: "n" }] }],
    ["no messages", { ...base, messages: [] }],
    ["empty content", { ...base, messages: [{ role: "user", content: [] }] }],
  ] as [string, unknown][]) {
    assert.notEqual(refused(body), "", `the broker sends ${what}`);
  }
});

test("the broker prices every usage the API has answered it with, at what its tokens cost", () => {
  // The first run with the real key, the syntax probe of 2026-11-16, found every call counted at
  // its worst, since each usage held output_tokens_details, a field the broker did not know and the
  // tests, whose usage objects were written by hand, had never shown it. Each real usage priced
  // here is one the broker met; a new field, or a value that costs more, fails here, not in a run.
  const limits = readLimits(LIMITS_TEXT);
  assert.ok(REAL_USAGE.length >= 5, "no real usage to price");
  for (const { from, model, usage } of REAL_USAGE) {
    const price = limits.models.get(model) as Price;
    assert.ok(price, `${from}: no price for ${model}`);
    assert.equal(unpricedField(usage, limits), null, `${from}: the broker cannot price it`);
    const n = (k: string) => Number(usage[k] ?? 0);
    const prompt = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    const factor = price.longPrompt !== undefined && prompt > price.longPrompt.above ? price.longPrompt.factor : 1;
    const expected = (factor * (n("input_tokens") * price.input + n("cache_creation_input_tokens") * price.input * limits.cacheWrite + n("cache_read_input_tokens") * price.cacheRead + n("output_tokens") * price.output)) / 1e6;
    assert.ok(Math.abs(usageCost(usage, price, limits, 99) - expected) < 1e-12, `${from}: priced at ${usageCost(usage, price, limits, 99)}, not ${expected}`);
  }
  // output_tokens_details costs nothing more in exactly one shape, {thinking_tokens: N}, N a whole
  // number no larger than output_tokens, which it is a share of; in any other, the worst.
  const sonnetPrice = limits.models.get("claude-sonnet-5-5") as Price;
  const base = { input_tokens: 100, output_tokens: 50 };
  for (const details of [{ thinking_tokens: 0 }, { thinking_tokens: 50 }, null]) {
    assert.equal(unpricedField({ ...base, output_tokens_details: details }, limits), null, JSON.stringify(details));
  }
  for (const details of [{ thinking_tokens: 51 }, { thinking_tokens: -1 }, { thinking_tokens: 1.5 }, { thinking_tokens: "1" }, { thinking_tokens: 1, cached_tokens: 0 }, {}, [1], 5, "x"]) {
    assert.deepEqual(unpricedField({ ...base, output_tokens_details: details }, limits), { field: "output_tokens_details" }, `priced output_tokens_details ${JSON.stringify(details)}`);
    assert.equal(usageCost({ ...base, output_tokens_details: details }, sonnetPrice, limits, 99), 99);
  }
  assert.deepEqual(unpricedField({ output_tokens_details: { thinking_tokens: 1 } }, limits), { field: "output_tokens_details" }, "a share of output tokens the usage does not report");
  // Any other field the broker does not know still counts at the worst, and is named, whatever its
  // name: the review of 0949a1e found an empty one priced, "" having meant everything priced, and
  // one named usage logged as no usage at all.
  assert.deepEqual(unpricedField({ ...base, surcharge_tokens: 5 }, limits), { field: "surcharge_tokens" });
  assert.deepEqual(unpricedField({ ...base, "": 12345 }, limits), { field: "" });
  assert.equal(usageCost({ ...base, "": 12345 }, sonnetPrice, limits, 99), 99);
  assert.deepEqual(unpricedField({ ...base, usage: 5 }, limits), { field: "usage" });
  assert.deepEqual(unpricedField("none", limits), { noUsage: true });
  assert.deepEqual(unpricedField(undefined, limits), { noUsage: true });
});

test("the broker says, once a field, which usage field it counted at the worst", async () => {
  const answers = [{ input_tokens: 1, output_tokens: 1, surcharge_tokens: 5 }, { input_tokens: 1, output_tokens: 1, surcharge_tokens: 7 }, { input_tokens: 1, output_tokens: 1 }];
  let k = 0;
  const { broker, notes } = fakeBroker(10, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { usage: answers.at(k++) }) }));
  for (const id of [1, 2, 3]) assert.equal(JSON.parse(await broker.handle(request(id, haiku()))).status, 200);
  assert.deepEqual(notes, ['the broker counts a request at its worst: it cannot price the usage field "surcharge_tokens"']);
  // An answer with no usage to read is said once too, and the key is never in a note.
  const bare = fakeBroker(10, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { said: KEY }) }));
  for (const id of [1, 2]) await bare.broker.handle(request(id, haiku()));
  assert.deepEqual(bare.notes, ["the broker counts a request at its worst: it cannot price an answer without a usage object"]);
  // Fields named so as to pass for a sentinel or to write a line of the log's own, each said as the
  // field it is, in one line (the review of 0949a1e): the empty name, a field named usage, a name
  // that would start a workflow command on a line of its own, and a name past any length.
  const odd = ["", "usage", "x\n::warning::injected", "y".repeat(1000)];
  let j = 0;
  const named = fakeBroker(10, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { usage: { input_tokens: 1, output_tokens: 1, [odd.at(j++) ?? "z"]: 1 } }) }));
  for (const id of [1, 2, 3, 4]) await named.broker.handle(request(id, haiku()));
  assert.equal(named.notes.length, 4, named.notes.join(" | "));
  assert.equal(named.notes.at(0), 'the broker counts a request at its worst: it cannot price the usage field ""');
  assert.equal(named.notes.at(1), 'the broker counts a request at its worst: it cannot price the usage field "usage"');
  assert.equal(named.notes.at(2), 'the broker counts a request at its worst: it cannot price the usage field "x\\n::warning::injected"');
  for (const n of named.notes) assert.ok(!n.includes("\n") && n.length <= LOG_LINE_MAX, `a note of ${n.length} characters, or with a line break: ${n.slice(0, 80)}`);
});

test("the broker hands back no answer from another model than the one asked: it refuses it at the worst it reserved, says so, and admits nothing more", async () => {
  // The second run with the real key (2026-11-16): Haiku's one answer had Sonnet's text and usage to
  // the token, and nothing read or recorded which model had written it. A fake API that answers a
  // request for Haiku as Sonnet, and others that answer with no model, with a name that would start
  // a workflow command of its own, and with no JSON.
  const limits = readLimits(LIMITS_TEXT);
  const usage = { input_tokens: 10, output_tokens: 10 };
  const answering = (model: unknown) => fakeBroker(10, () => ({ status: 200, retryAfter: "", text: JSON.stringify({ model, content: [{ type: "text", text: "a program" }], usage }) }));
  const { broker, notes, posts } = answering("claude-sonnet-5-5");
  const first = JSON.parse(await broker.handle(request(1, haiku())));
  assert.equal(first.refused, 'the broker refuses an answer from the model "claude-sonnet-5-5" to a request for "claude-haiku-5-5", which may come back only as "claude-haiku-5-5"');
  assert.deepEqual([first.anomaly, first.billed, first.exhausted, first.status, first.text], ["another model", true, undefined, undefined, undefined], "the broker handed back another model's answer");
  // Billed, at the requested model's worst, which admission reserved, so that the run's cap holds.
  assert.ok(Math.abs(broker.spent - brokerWorst(JSON.stringify(haiku()), 100, limits.models.get("claude-haiku-5-5") as Price, limits, 0)) < 1e-12, `spent ${broker.spent}`);
  assert.equal(broker.reserved, 0);
  // The run fails on it, so the broker admits nothing more: the next request is refused, unsent.
  const after = JSON.parse(await broker.handle(request(2, haiku())));
  assert.equal(after.refused, "the broker admits nothing more this run: an answer came from another model than the one asked");
  assert.deepEqual([after.anomaly, after.billed, after.exhausted], ["another model", undefined, undefined]);
  assert.equal(posts.length, 1);
  assert.deepEqual(notes, [first.refused]);
  // An answer that names no model, and one whose name would write a line of its own: each refused,
  // and said on one line.
  const none = answering(undefined);
  assert.equal(JSON.parse(await none.broker.handle(request(1, haiku()))).refused, 'the broker refuses an answer that names no model to a request for "claude-haiku-5-5"');
  const injected = answering("x\n::warning::injected");
  assert.match(JSON.parse(await injected.broker.handle(request(1, haiku()))).refused, /from the model "x\\n::warning::injected"/);
  assert.equal(none.notes.length + injected.notes.length, 2);
  for (const n of [...none.notes, ...injected.notes]) assert.ok(!n.includes("\n") && n.length <= LOG_LINE_MAX, n);
  // An answer that is no JSON names no model either, and the key never reaches the refusal.
  const garbled = fakeBroker(10, () => ({ status: 200, retryAfter: "", text: `no JSON ${KEY}` }));
  const line = await garbled.broker.handle(request(1, haiku()));
  assert.equal(JSON.parse(line).anomaly, "another model");
  assert.ok(!line.includes(KEY));
  // An answer from the model asked is handed back, priced by its usage; an error status, which
  // names no model and is not billed, is handed back as it was.
  const honest = fakeBroker(10, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { usage }) }));
  assert.equal(JSON.parse(await honest.broker.handle(request(1, haiku()))).status, 200);
  assert.ok(Math.abs(honest.broker.spent - (10 * 0.1 + 10 * 0.5) / 1e6) < 1e-12);
  assert.deepEqual(honest.notes, []);
  const failing = fakeBroker(10, () => ({ status: 529, retryAfter: "", text: "{}" }));
  assert.equal(JSON.parse(await failing.broker.handle(request(1, haiku()))).status, 529);
  assert.equal(failing.broker.spent, 0);
  // The rule itself: the names answeredAs pins, and nothing else, a dated name it does not pin among them.
  assert.equal(wrongModel("claude-haiku-5-5", "claude-haiku-5-5", limits), null);
  assert.deepEqual(wrongModel("claude-haiku-5-5-20261101", "claude-haiku-5-5", limits), { answered: "claude-haiku-5-5-20261101" });
  assert.deepEqual(wrongModel("claude-sonnet-5-5", "claude-haiku-5-5", limits), { answered: "claude-sonnet-5-5" });
  assert.deepEqual(wrongModel(5, "claude-haiku-5-5", limits), { noModel: true });
  const dated = { ...limits, answeredAs: new Map([["claude-haiku-5-5", ["claude-haiku-5-5", "claude-haiku-5-5-20261101"]]]) };
  assert.equal(wrongModel("claude-haiku-5-5-20261101", "claude-haiku-5-5", dated), null);
});

test("answers from another model keep the run within its cap, whatever is in flight", async () => {
  // The review of b520209: counted at any model's most, a Haiku request of a 20 KB body answered as
  // Sonnet took $1.05 of a $0.05 cap, and each of 16 in flight could take up to about $51. Now each
  // counts at what admission reserved for it: four such requests in flight at once fit the cap, a
  // fifth is refused at it, and when the four come back as Sonnet the run has spent within its cap,
  // said so once, and admits nothing more.
  const limits = readLimits(LIMITS_TEXT);
  let release = () => {};
  const held = new Promise<void>((done) => {
    release = done;
  });
  const { broker, notes, posts } = fakeBroker(0.05, async () => {
    await held;
    return { status: 200, retryAfter: "", text: JSON.stringify({ model: "claude-sonnet-5-5", content: [], usage: { input_tokens: 5000, output_tokens: 16000 } }) };
  });
  const big = { ...haiku(MAX_TOKENS), messages: [{ role: "user", content: "x".repeat(20_000) }] };
  const flying = [1, 2, 3, 4].map((id) => broker.handle(request(id, big)));
  assert.match(JSON.parse(await broker.handle(request(5, big))).refused, /past its \$0\.05/);
  release();
  const answers = (await Promise.all(flying)).map((line) => JSON.parse(line));
  assert.ok(answers.every((a) => a.anomaly === "another model" && a.billed === true), JSON.stringify(answers));
  assert.ok(broker.spent <= broker.cap, `spent $${broker.spent} of a $${broker.cap} cap`);
  assert.ok(Math.abs(broker.spent - 4 * brokerWorst(JSON.stringify(big), MAX_TOKENS, limits.models.get("claude-haiku-5-5") as Price, limits, 0)) < 1e-12, `spent $${broker.spent}`);
  assert.ok(Math.abs(broker.reserved) < 1e-12, `reserved $${broker.reserved}`);
  assert.equal(notes.length, 1, "four refusals of one mismatch were said more than once");
  assert.match(JSON.parse(await broker.handle(request(6, haiku()))).refused, /admits nothing more/);
  assert.equal(posts.length, 4);
});

test("a run whose requests race its anomaly fails for that anomaly, and its first failure stands", async () => {
  // The re-check of 2ead2d4: each worker of the benchmark wrote its error over the last, so a racing
  // worker's refusal after the broker closed could replace the wrong-model failure, and a held-out
  // run then read "Error". Two workers' calls, each with its own budget, as if the harness's stop
  // had not yet reached the second: one answered as Haiku to a request for Sonnet, which closes the
  // broker, and one sent after it.
  const sonnet = own(KNOWN_MODELS, "claude-sonnet-5-5");
  const { broker, posts } = fakeBroker(10, () => ({ status: 200, retryAfter: "", text: JSON.stringify({ model: "claude-haiku-5-5", content: [], usage: { input_tokens: 1, output_tokens: 1 } }) }));
  const post = async (body: object) => {
    const answer = JSON.parse(await broker.handle(request(1, body)));
    if (typeof answer.refused === "string") throw refusalOf(answer);
    return answer as Answer;
  };
  const asked = requestBody("claude-sonnet-5-5", "S", [{ role: "user", content: "x" }]);
  const failed = () => budgetedCall(post, { limit: 10, spent: 0, reserved: 0, stopped: false }, asked, sonnet).then(() => null, (e: unknown) => e);
  const wrong = await failed();
  const after = await failed();
  assert.ok(wrong instanceof BrokerRefused && wrong.billed && wrong.anomaly === "another model", String(wrong));
  assert.ok(after instanceof BrokerRefused && !after.billed && after.anomaly === "another model", String(after));
  assert.equal(posts.length, 1);
  // Read in either order, a held-out run fails for the anomaly, and the first failure stands.
  for (const order of [
    [wrong, after],
    [after, wrong],
  ]) {
    let held = "";
    let open = "";
    for (const e of order) {
      held = firstFailure(held, e, "held-out", "held-out-3-mente");
      open = firstFailure(open, e, "public", "gcd-mente-claude-sonnet-5-5-1");
    }
    assert.equal(held, "a held-out run failed: an answer came from another model than the one asked");
    assert.equal(open, `gcd-mente-claude-sonnet-5-5-1: ${(order.at(0) as Error).message}`);
  }
  // The benchmark records its failures so.
  assert.match(readFileSync(join(REPOSITORY, "benchmark", "harness", "bench.ts"), "utf8"), /\bfailure = firstFailure\(failure, e, spec\.set, spec\.name\);/);
});

test("an answer billed past the most its request could cost counts as billed, and fails the run as another model's does", async () => {
  // The re-check of 2ead2d4: 200,000 output tokens against a max_tokens of 16,000, from the model
  // asked, spent $0.10 of a $0.05 cap, and the run went on. No honest usage costs more than its
  // request could: the money is spent, so both ledgers count what the usage says, and both then
  // admit nothing more, the broker saying so once.
  const limits = readLimits(LIMITS_TEXT);
  const haikuModel = own(KNOWN_MODELS, "claude-haiku-5-5");
  const usage = { input_tokens: 10, output_tokens: 200_000 };
  const { broker, notes, posts } = fakeBroker(0.05, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { content: [{ type: "text", text: "a program" }], stop_reason: "end_turn", usage }) }));
  const post = async (body: object) => {
    const answer = JSON.parse(await broker.handle(request(1, body)));
    if (typeof answer.refused === "string") throw refusalOf(answer);
    return answer as Answer;
  };
  const body = requestBody("claude-haiku-5-5", "S", [{ role: "user", content: "x" }]);
  const ledger: Budget = { limit: 0.05, spent: 0, reserved: 0, stopped: false };
  await assert.rejects(budgetedCall(post, ledger, body, haikuModel), (e: unknown) => e instanceof RunAnomaly && e.anomaly === "past its worst");
  const billed = usageCost(usage, limits.models.get("claude-haiku-5-5") as Price, limits, 0);
  assert.ok(billed > worstCost(body, haikuModel) && billed > 0.05, String(billed));
  assert.ok(Math.abs(broker.spent - billed) < 1e-12, `the broker counted $${broker.spent}, not the $${billed} billed`);
  assert.ok(Math.abs(ledger.spent - broker.spent) < 1e-12, `the harness counted $${ledger.spent}, the broker $${broker.spent}`);
  assert.deepEqual([broker.closed, ledger.stopped], ["past its worst", true]);
  assert.equal(notes.length, 1);
  assert.match(notes.at(0) ?? "", /past the \$[0-9.e-]+ it reserved for it/);
  // Nothing more is sent, by the harness or by the broker.
  assert.equal(await budgetedCall(post, ledger, body, haikuModel), null);
  const after = JSON.parse(await broker.handle(request(2, haiku())));
  assert.deepEqual([after.anomaly, after.billed], ["past its worst", undefined]);
  assert.equal(posts.length, 1);
  // A usage at the worst exactly is honest: every byte of the body written to the cache, and the
  // most output.
  const exact = fakeBroker(10, (sent) => ({ status: 200, retryAfter: "", text: asAsked(sent, { usage: { input_tokens: 0, cache_creation_input_tokens: utf8Length(sent), output_tokens: 100 } }) }));
  assert.equal(JSON.parse(await exact.broker.handle(request(1, haiku()))).status, 200);
  assert.deepEqual([exact.broker.closed, exact.notes], [null, []]);
});

/**
 * The scripts that call the API, each with the planning runs the test below makes of it: the
 * benchmark for every milestone of the public tasks, the preliminary run for both of its sets, and
 * the syntax probe.
 */
const PLANS: [string, string[][]][] = [
  ["bench.ts", [...new Set(readdirSync(join(REPOSITORY, "benchmark", "tasks")).filter((f) => f.endsWith(".md")).map((f) => parseTask(readFileSync(join(REPOSITORY, "benchmark", "tasks", f), "utf8")).milestone))].sort().map((m) => ["--budget", "1", "--milestone", m, "--runs", "1"])],
  ["prelim.ts", [["--tasks", "core", "--runs", "1"], ["--tasks", "non-core", "--runs", "1"]]],
  ["probe.ts", [["--runs", "1"]]],
];

test("every script that calls the API plans its runs with no broker and no key, as far as its first call", () => {
  // Run 3 of the plan (2026-11-16): bench.ts threw while choosing its tasks, before any call, with
  // the key in reach, since no test had ever run its main. Each script's path up to its first call
  // now runs here, on every change: its choice of tasks, its prompts, and its check that every task
  // can be written in every language. A new script that calls the API joins PLANS, or this fails.
  const harness = join(REPOSITORY, "benchmark", "harness");
  const callers = readdirSync(harness).filter((f) => f.endsWith(".ts") && f !== "api.ts" && f !== "harness.test.ts" && readFileSync(join(harness, f), "utf8").includes("budgetedCall("));
  assert.deepEqual(callers.sort(), PLANS.map(([f]) => f).sort(), "a script that calls the API is not planned here");
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => k !== "ANTHROPIC_API_KEY"));
  const plan = (file: string, args: string[]) => spawnSync(process.execPath, ["--disallow-code-generation-from-strings", join(harness, file), "--plan", ...args], { cwd: REPOSITORY, env, encoding: "utf8", timeout: 120_000 });
  const m1 = readdirSync(join(REPOSITORY, "benchmark", "tasks"))
    .filter((f) => f.endsWith(".md"))
    .map((f) => parseTask(readFileSync(join(REPOSITORY, "benchmark", "tasks", f), "utf8")))
    .filter((task) => task.milestone === "M1" && task.set === "public" && !task.retired);
  for (const [file, runs] of PLANS) {
    for (const args of runs) {
      const r = plan(file, args);
      const said = `${file} --plan ${args.join(" ")}: ${r.stderr.slice(-600)}`;
      if (file === "bench.ts" && args.at(3) !== "M1") {
        // A later milestone plans, or refuses as a run would: a task in a language the compiler or the
        // harness cannot write yet. Never another error.
        if (r.status !== 0) assert.match(r.stderr, /^Error: [\w-]+ in (mente|rust|odin): (`[^`]+` is not a type M\d has|the harness does not yet write .+)$/m, said);
        continue;
      }
      assert.equal(r.status, 0, said);
      const planned = JSON.parse(r.stdout.trim().split("\n").at(-1) ?? "{}");
      assert.ok(planned.planned_runs > 0, said);
      if (file === "bench.ts") assert.equal(planned.planned_runs, m1.length * 2 * 3, `every public M1 task, in each model and language: ${said}`);
    }
  }
  // The held-out branch too, on stand-in tasks (benchmark/harness/fixtures/held-out): their reading,
  // the filter that leaves the retired one out, and the check that each can be written, planned as
  // counts alone, no held-out task named (the review of aafe59b: no plan had run that branch).
  const held = plan("bench.ts", ["--budget", "1", "--milestone", "M1", "--runs", "1", "--held-out", join(harness, "fixtures", "held-out")]);
  assert.equal(held.status, 0, held.stderr);
  const heldPlan = JSON.parse(held.stdout.trim().split("\n").at(-1) ?? "{}");
  assert.equal(heldPlan.held_out, 1, held.stdout);
  assert.equal(heldPlan.planned_runs, (m1.length + 1) * 2 * 3, held.stdout);
  assert.ok(!held.stdout.includes("stand-in"), "a plan named a held-out task");
  // A held-out task that cannot be written, by its signature, a type, or an example: each plan
  // fails naming the task by its place alone, and no output of it holds anything of the task (the
  // final pass on 89c7691: a mangled signature, parsed outside the try that hid such errors,
  // printed itself). The stand-ins' every field holds "secret_heldout".
  const bad = join(harness, "fixtures", "held-out-bad");
  const cases = readdirSync(bad).sort();
  assert.deepEqual(cases, ["example", "signature", "type"]);
  const planned = cases.map((c) => [c, plan("bench.ts", ["--budget", "1", "--milestone", "M1", "--runs", "1", "--held-out", join(bad, c)])] as const);
  for (const [c, r] of planned) assert.doesNotMatch(`${r.stdout}${r.stderr}`, /secret_heldout/i, `the plan of a held-out task with a bad ${c} printed the task`);
  for (const [c, r] of planned) {
    assert.notEqual(r.status, 0, `a held-out task with a bad ${c} planned`);
    assert.match(r.stderr, /HeldOutError: held-out-1 cannot be written in (mente|rust|odin): Error$/m, `${c}: ${`${r.stdout}${r.stderr}`.slice(-600)}`);
  }
  for (const r of [held]) assert.doesNotMatch(`${r.stdout}${r.stderr}`, /stand-in|secret_heldout/i);
});

test("every step of a held-out task leaves through one boundary, which passes on nothing of the error it met", async () => {
  // The final pass on 89c7691. An error carrying a held-out task's text in its message, its name
  // and its cause leaves heldOut and heldOutAsync as a HeldOutError whose message, stack, cause and
  // printed form, as node prints an uncaught error, hold none of it; and the measured run's record
  // and log say only what the boundary made of it.
  const secret = "SECRET_HELDOUT_TEXT";
  const met = () => {
    const e = new Error(`not a signature: ${secret} a b`, { cause: new Error(secret) });
    e.name = `${secret}Error`;
    return e;
  };
  const caught: unknown[] = [];
  try {
    heldOut("held-out-2 cannot be written in rust", () => {
      throw met();
    });
  } catch (e) {
    caught.push(e);
  }
  await heldOutAsync("a held-out run failed", async () => {
    throw met();
  }).catch((e: unknown) => caught.push(e));
  assert.equal(caught.length, 2);
  for (const e of caught) {
    assert.ok(e instanceof HeldOutError);
    assert.equal(e.cause, undefined);
    for (const said of [e.message, String(e.stack), String(e), inspect(e)]) assert.ok(!said.includes(secret), `the boundary passed on the task's text: ${said}`);
  }
  assert.equal((caught[0] as Error).message, "held-out-2 cannot be written in rust: Error");
  assert.equal(failureOf(caught[1], "held-out", "held-out-2-rust-claude-haiku-5-5-1"), "a held-out run failed: Error");
  assert.equal(firstFailure("", caught[1], "held-out", "held-out-2-rust-claude-haiku-5-5-1"), "a held-out run failed: Error");
  // What it keeps of a run's error: the anomaly and the API's status.
  await heldOutAsync("a held-out run failed", async () => {
    throw new RunAnomaly("another model", `an answer from ${secret}`);
  }).catch((e: unknown) => {
    assert.equal((e as Error).message, "a held-out run failed: an answer came from another model than the one asked");
    // And the record and the log say what the boundary made of it.
    assert.equal(failureOf(e, "held-out", "held-out-2-rust-claude-haiku-5-5-1"), "a held-out run failed: an answer came from another model than the one asked");
  });
  await heldOutAsync("a held-out run failed", async () => {
    throw new Error(`the API answered 400: ${secret}`);
  }).catch((e: unknown) => assert.equal((e as Error).message, "a held-out run failed: the API answered 400"));
  // A value that returns passes unchanged.
  assert.equal(heldOut("x", () => 7), 7);
  // bench.ts sends every held-out step through it: the reading, each spec with its check, and each
  // run; and reads the held-out set nowhere else.
  const bench = readFileSync(join(REPOSITORY, "benchmark", "harness", "bench.ts"), "utf8");
  assert.match(bench, /const heldTasks = heldOutDir \? readHeldOut\(join\(heldOutDir, "tasks"\), milestone\) : \[\];/);
  assert.match(bench, /heldOut\(`held-out file \$\{k \+ 1\} could not be read`, \(\) => parseTask\(readFileSync\(join\(dir, f\), "utf8"\)\)\)/);
  assert.match(bench, /return heldOut\("the held-out set could not be chosen from", \(\) => heldOutTasks\(tasks, milestone\)\);/);
  assert.match(bench, /if \(set === "held-out"\) specs\.push\(heldOut\(`\$\{label\} cannot be written in \$\{lang\}`, specOf\)\);/);
  assert.match(bench, /await heldOutAsync\("a held-out run failed", \(\) => runOne\(ctx, spec\)\)/);
  assert.equal(bench.match(/\bheldOutDir\b/g)?.length, 6, "bench.ts reads the held-out set beyond its check and readHeldOut");
});

test("every harness that calls the API records, in each attempt, the model that wrote the answer", () => {
  // So that every result says which model produced it (the second run with the real key recorded
  // only the model it asked).
  const callers = readdirSync(join(REPOSITORY, "benchmark", "harness")).filter((f) => f.endsWith(".ts") && f !== "api.ts" && f !== "harness.test.ts" && readFileSync(join(REPOSITORY, "benchmark", "harness", f), "utf8").includes("budgetedCall("));
  assert.deepEqual(callers.sort(), ["bench.ts", "prelim.ts", "probe.ts"]);
  for (const f of callers) {
    const text = readFileSync(join(REPOSITORY, "benchmark", "harness", f), "utf8");
    assert.equal(text.split("answered_by: reply.model,").length - 1, 1, `${f} records no answering model`);
    // And a call that fails, an answer from another model among them, fails the job, its record
    // saying why.
    assert.equal(text.split("if (failure) process.exit(1);").length - 1, 1, `${f} does not fail the job on a failed call`);
  }
});

test("a measured run's failure names its run and error, and a held-out run's only the API's status or that another model answered", () => {
  const wrong = new BrokerRefused('the broker refused the request: the broker refuses an answer from the model "claude-sonnet-5-5" to a request for "claude-haiku-5-5"', false, "another model", true);
  assert.equal(failureOf(wrong, "public", "gcd-mente-claude-haiku-5-5-1"), `gcd-mente-claude-haiku-5-5-1: ${wrong.message}`);
  assert.equal(failureOf(wrong, "held-out", "held-out-3-mente"), "a held-out run failed: an answer came from another model than the one asked");
  assert.equal(failureOf(new Error("the API answered 400: a held-out task's text"), "held-out", "held-out-3-mente"), "a held-out run failed: the API answered 400");
  assert.equal(failureOf(new BrokerRefused("a model it does not name", false), "held-out", "held-out-3-mente"), "a held-out run failed: Error");
  assert.equal(failureOf("no error", "held-out", "held-out-3-mente"), "a held-out run failed: Error");
  assert.equal(failureOf("no error", "public", "gcd"), "gcd: no error");
  // An answer billed past its worst, and a refusal after either anomaly, are named as their anomaly.
  assert.equal(failureOf(new RunAnomaly("past its worst", "an answer was billed past the most its request could cost: $0.1, where the most was $0.008"), "held-out", "held-out-3-mente"), "a held-out run failed: an answer was billed past the most its request could cost");
  assert.equal(failureOf(refusalOf({ refused: "the broker admits nothing more this run: x", anomaly: "past its worst" }), "held-out", "held-out-3-mente"), "a held-out run failed: an answer was billed past the most its request could cost");
  assert.equal(failureOf(refusalOf({ refused: "x", anomaly: "toString" }), "held-out", "held-out-3-mente"), "a held-out run failed: Error", "a name no anomaly has was read as one");
  assert.equal(failureOf(null, "held-out", "held-out-3-mente"), "a held-out run failed: Error");
});

test("every line the broker writes to its log is its own words, with what it did not author as JSON, the key out, and a cap", () => {
  const line = logLine(KEY);
  assert.equal(line`plain words, ${3} requests`, "plain words, 3 requests");
  assert.equal(line`at ${"/a b/socket"}`, 'at "/a b/socket"');
  // A value that would end the line and start a workflow command stays on this line.
  const injected = line`failed: ${"x\n::error::y\r"}`;
  assert.equal(injected, 'failed: "x\\n::error::y\\r"');
  assert.ok(!/[\r\n]/.test(injected));
  // The key, wherever it is, is taken out.
  assert.equal(line`${`the key ${KEY}`}`, '"the key [the API key]"');
  // And no line passes LOG_LINE_MAX characters.
  const long = line`${"z".repeat(5000)}`;
  assert.equal(long.length, LOG_LINE_MAX);
  assert.ok(long.endsWith("..."));
});

test("the broker prices what the usage reports, a long Haiku prompt at its higher prices, and the rest at the worst", () => {
  const limits = readLimits(LIMITS_TEXT);
  const haikuPrice = limits.models.get("claude-haiku-5-5") as Price;
  const sonnetPrice = limits.models.get("claude-sonnet-5-5") as Price;
  const plainUsage = { input_tokens: 1000, output_tokens: 1000, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 };
  assert.ok(Math.abs(usageCost(plainUsage, sonnetPrice, limits, 99) - (1000 * 2 + 1000 * 10) / 1e6) < 1e-12);
  // Fields that cost nothing as reported keep the price.
  const benign = { ...plainUsage, cache_creation: { ephemeral_5m_input_tokens: 0, ephemeral_1h_input_tokens: 0 }, server_tool_use: { web_search_requests: 0 }, service_tier: "standard", speed: "standard", inference_geo: "global", other: null };
  assert.equal(usageCost(benign, sonnetPrice, limits, 99), usageCost(plainUsage, sonnetPrice, limits, 99));
  // Anything the broker does not price counts the request at its worst.
  for (const [what, extra] of [
    ["a 1-hour cache write", { cache_creation: { ephemeral_5m_input_tokens: 0, ephemeral_1h_input_tokens: 10 } }],
    ["a web search", { server_tool_use: { web_search_requests: 1 } }],
    ["fast mode", { speed: "fast" }],
    ["inference held in the US", { inference_geo: "us" }],
    ["another tier", { service_tier: "priority" }],
    ["a field the broker does not know", { surcharge_tokens: 5 }],
    ["a cache breakdown with a field more", { cache_creation: { ephemeral_5m_input_tokens: 0, ephemeral_24h_input_tokens: 1 } }],
  ] as [string, object][]) {
    assert.equal(usageCost({ ...plainUsage, ...extra }, sonnetPrice, limits, 99), 99, `the broker priced a usage with ${what}`);
  }
  assert.equal(usageCost("none", sonnetPrice, limits, 99), 99);
  // Haiku 5.5 prices a prompt of more than 100,000 tokens, cache reads and writes included, at five times.
  const short = usageCost({ input_tokens: 50_000, cache_read_input_tokens: 50_000, output_tokens: 10 }, haikuPrice, limits, 99);
  const long = usageCost({ input_tokens: 50_000, cache_read_input_tokens: 50_001, output_tokens: 10 }, haikuPrice, limits, 99);
  assert.ok(Math.abs(short - (50_000 * 0.1 + 50_000 * 0.01 + 10 * 0.5) / 1e6) < 1e-12, String(short));
  assert.ok(Math.abs(long - (5 * (50_000 * 0.1 + 50_001 * 0.01 + 10 * 0.5)) / 1e6) < 1e-12, String(long));
  // And its worst case: a body of more than 100,000 bytes may hold that many tokens.
  const small = "x".repeat(100_000);
  const big = "x".repeat(100_001);
  const rules = { cacheWrite: 1.25, maxTokens: 16_000 };
  assert.ok(Math.abs(brokerWorst(small, 10, haikuPrice, rules, 0) - (100_000 * 0.1 * 1.25 + 10 * 0.5) / 1e6) < 1e-12);
  assert.ok(Math.abs(brokerWorst(big, 10, haikuPrice, rules, 0) - (5 * (100_001 * 0.1 * 1.25 + 10 * 0.5)) / 1e6) < 1e-12);
  // A thinking block counts at maxTokens input tokens, whatever its signature's bytes: the re-check
  // of 4ab15da had a 200-byte signature reserved as 200 tokens and billed at its whole reasoning.
  assert.ok(Math.abs(brokerWorst("x", 10, sonnetPrice, rules, 1) - brokerWorst("x", 10, sonnetPrice, rules, 0) - (16_000 * 2 * 1.25) / 1e6) < 1e-12);
  // And it can take a short Haiku prompt past the long-prompt threshold.
  assert.ok(Math.abs(brokerWorst("x".repeat(90_000), 10, haikuPrice, rules, 1) - (5 * (106_000 * 0.1 * 1.25 + 10 * 0.5)) / 1e6) < 1e-12);
  const blocks = { messages: [{ role: "user", content: [{ type: "text", text: "x" }] }, { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "s" }, { type: "redacted_thinking", data: "d" }, { type: "text", text: "y" }] }, { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "t" }] }] };
  assert.equal(thinkingBlocks(blocks), 3);
  assert.equal(thinkingBlocks({ messages: [{ role: "user", content: [{ type: "thinking" }] }] }), 0);
  assert.equal(thinkingBlocks("none"), 0);
  assert.equal(utf8Length("\u00e9"), 2);
});

test("the broker refuses a body past its cap and a request past its cap in flight, and a line past its length", async () => {
  const limits = { ...readLimits(LIMITS_TEXT), maxBodyBytes: 200, maxInFlight: 2 };
  let release = () => {};
  const held = new Promise<void>((done) => {
    release = done;
  });
  const { broker, posts } = fakeBroker(10, async (body) => {
    await held;
    return { status: 200, retryAfter: "", text: asAsked(body, { usage: { input_tokens: 1, output_tokens: 1 } }) };
  }, limits);
  const big = JSON.parse(await broker.handle(request(1, { ...haiku(), messages: [{ role: "user", content: "x".repeat(300) }] })));
  assert.match(big.refused, /a body of \d+ bytes, past the 200/);
  assert.equal(big.exhausted, undefined);
  const first = broker.handle(request(2, haiku()));
  const second = broker.handle(request(3, haiku()));
  const third = JSON.parse(await broker.handle(request(4, haiku())));
  assert.match(third.refused, /past the 2 the broker sends at once/);
  release();
  await Promise.all([first, second]);
  assert.equal(broker.inFlight, 0);
  assert.equal(JSON.parse(await broker.handle(request(5, haiku()))).status, 200);
  assert.equal(posts.length, 3);
  // A request whose earlier reply holds a thinking block reserves maxTokens input tokens for it: on
  // Opus, $0.08 more, past a budget of $0.05 the same request without it fits within.
  const opus = { model: "claude-opus-5-5", max_tokens: 100, messages: [{ role: "user", content: "x" }, { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "s".repeat(200) }, { type: "text", text: "y" }] }, { role: "user", content: "z" }] };
  const budgeted = fakeBroker(0.05, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { usage: { input_tokens: 1, output_tokens: 1 } }) })).broker;
  assert.equal(JSON.parse(await budgeted.handle(request(1, { ...opus, messages: [{ role: "user", content: "x" }] }))).status, 200);
  assert.match(JSON.parse(await budgeted.handle(request(2, opus))).refused, /past its \$0\.05/);
  // The line reader: lines in order, blank ones skipped, and a line past its length stops it once.
  const lines: string[] = [];
  let stopped = 0;
  const feed = lineReader(10, (l) => lines.push(l), () => stopped++);
  feed("one\n\ntw");
  feed("o\n");
  assert.deepEqual([lines, stopped], [["one", "two"], 0]);
  feed("0123456789A\nlater\n");
  feed("more\n");
  assert.deepEqual([lines, stopped], [["one", "two"], 1]);
  const endless = lineReader(10, (l) => lines.push(l), () => stopped++);
  endless("0123456789");
  endless("A");
  endless("\n");
  assert.equal(stopped, 2, "a line with no end yet, past the length, did not stop the reader");
});

/** A socket address for a test: a named pipe on Windows, a path in a temporary directory elsewhere. */
const socketPath = (dir: string, name: string) => (process.platform === "win32" ? `\\\\.\\pipe\\mente-${name}-${process.pid}-${Date.now()}` : join(dir, `${name}.sock`));

test("the broker serves no more connections at once than broker-limits.json allows", async () => {
  const dir = mkdtempSync(join(tmpdir(), "mente-conns-"));
  const { broker } = fakeBroker(10, (body) => ({ status: 200, retryAfter: "", text: asAsked(body, { usage: { input_tokens: 1, output_tokens: 1 } }) }), { ...readLimits(LIMITS_TEXT), maxConnections: 2 });
  const at = socketPath(dir, "capped");
  const server = await serve(broker, at, (e) => {
    throw e;
  });
  const links: { close(): void }[] = [];
  try {
    const first = await connectBroker(at);
    const second = await connectBroker(at);
    links.push(first, second);
    // The third is closed as it comes, so a request on it finds the broker gone.
    const third = await connectBroker(at).catch((e: Error) => e);
    if (!(third instanceof Error)) {
      links.push(third);
      await assert.rejects(third.post(haiku()), /gone|failed/, "the broker served a connection past its cap");
    }
    // The first two are served.
    for (const link of [first, second]) assert.equal((await link.post(haiku())).status, 200);
  } finally {
    // Every link closed first, so that the server's close does not wait on one a failure left open.
    for (const link of links) link.close();
    await new Promise((done) => server.close(done));
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the harness fails clearly when the broker is not there, goes away, refuses or answers what it cannot read", async () => {
  const dir = mkdtempSync(join(tmpdir(), "mente-broker-"));
  try {
    await assert.rejects(connectBroker(socketPath(dir, "nobody")), /the broker is not there/);
    // A broker that answers by id, out of order; refuses id 3, id 4 past the run's limits, and id 5
    // as another model's answer; answers id 6 with no JSON.
    const at = socketPath(dir, "fake");
    const server = net.createServer((conn) => {
      let pending = "";
      conn.setEncoding("utf8");
      conn.on("data", (chunk: string) => {
        pending += chunk;
        const lines = pending.split("\n");
        pending = lines.pop() ?? "";
        for (const l of lines) {
          const { id } = JSON.parse(l);
          if (id === 1) setTimeout(() => conn.write(`${JSON.stringify({ id, status: 200, retryAfter: "", text: "one" })}\n`), 50);
          if (id === 2) conn.write(`${JSON.stringify({ id, status: 200, retryAfter: "", text: "two" })}\n`);
          if (id === 3) conn.write(`${JSON.stringify({ id, refused: "a model it does not name" })}\n`);
          if (id === 4) conn.write(`${JSON.stringify({ id, refused: "a request past the run's 2", exhausted: true })}\n`);
          if (id === 5) conn.write(`${JSON.stringify({ id, refused: "an answer from the model \"claude-sonnet-5-5\"", anomaly: "another model", billed: true })}\n`);
          if (id === 6) conn.write("garbage\n");
        }
      });
    });
    await new Promise<void>((done) => server.listen(at, done));
    const link = await connectBroker(at);
    const [one, two] = await Promise.all([link.post({}), link.post({})]);
    assert.deepEqual([one.text, two.text], ["one", "two"]);
    await assert.rejects(link.post({}), (e: unknown) => e instanceof BrokerRefused && !e.exhausted && e.anomaly === null && !e.billed);
    await assert.rejects(link.post({}), (e: unknown) => e instanceof BrokerRefused && e.exhausted && e.anomaly === null);
    await assert.rejects(link.post({}), (e: unknown) => e instanceof BrokerRefused && e.anomaly === "another model" && e.billed && !e.exhausted);
    await assert.rejects(link.post({}), /cannot read: garbage/);
    await assert.rejects(link.post({}), BrokerBroken, "a broken link took another request");
    // A broker that goes away while a request waits, and one that never answers.
    const quiet = net.createServer((conn) => setTimeout(() => conn.destroy(), 50));
    const quietAt = socketPath(dir, "quiet");
    await new Promise<void>((done) => quiet.listen(quietAt, done));
    await assert.rejects((await connectBroker(quietAt)).post({}), /the broker is gone/);
    // It reads what it is sent, so that it sees the link close; one that never read would hold its
    // end of the connection open, and the test's process with it.
    const mute = net.createServer((conn) => conn.resume());
    const muteAt = socketPath(dir, "mute");
    await new Promise<void>((done) => mute.listen(muteAt, done));
    const muted = await connectBroker(muteAt, 100);
    await assert.rejects(muted.post({}), /gave no answer/);
    // A closed link fails what still waits, and takes nothing more.
    const waits = muted.post({});
    muted.close();
    await assert.rejects(waits, /closed its link/);
    await assert.rejects(muted.post({}), /closed its link/);
    await Promise.all([server, quiet, mute].map((s) => new Promise((done) => s.close(done))));
    // A failed answer is a network error the caller counts as spent; an error status throws.
    await assert.rejects(send(async () => ({ status: -1, retryAfter: "", text: "the request failed" }), {}), NetworkError);
    await assert.rejects(send(async () => ({ status: 400, retryAfter: "", text: "bad" }), {}), /the API answered 400: bad/);
    const reply = await send(async () => ({ status: 200, retryAfter: "", text: JSON.stringify({ model: "claude-haiku-5-5", content: [{ type: "text", text: "x" }], stop_reason: "end_turn", usage: {} }) }), {});
    assert.deepEqual([reply.model, reply.text, reply.stop_reason], ["claude-haiku-5-5", "x", "end_turn"]);
    await assert.rejects(send(async () => ({ status: 200, retryAfter: "", text: JSON.stringify({ content: [], stop_reason: "end_turn", usage: {} }) }), {}), /without naming the model/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

/** Linux alone has bash, env -i, Unix sockets and /proc as the runners do. */
const LINUX_ONLY = process.platform === "linux" ? false : "the runners' shell, sockets and /proc are Linux's";

test("every shell script of the harness parses", { skip: spawnSync("bash", ["-c", "true"]).status === 0 ? false : "no bash" }, () => {
  // A script the jobs run only on a runner, such as lock-sudo.sh, would otherwise first fail there:
  // an apostrophe in a ${name:?message} inside double quotes opens a quote bash never closes.
  const dir = join(REPOSITORY, "benchmark", "harness");
  const scripts = readdirSync(dir).filter((f) => f.endsWith(".sh"));
  assert.ok(scripts.includes("lock-sudo.sh") && scripts.includes("start-broker.sh"), scripts.join(", "));
  for (const f of scripts) {
    const r = spawnSync("bash", ["-n", join(dir, f)], { encoding: "utf8" });
    assert.equal(r.status, 0, `${f}: ${r.stderr}`);
  }
});

test("fetch-held-out.sh checks the held-out set out at its commit, into a directory it makes, and prints nothing of it", { skip: spawnSync("bash", ["-c", "true"]).status === 0 ? false : "no bash" }, () => {
  // Run 3 of the plan (2026-11-16): actions/checkout printed the held-out commit's subject into this
  // public repository's log. A repository standing in for the held-out one, reached through a `git`
  // first on PATH that hands the real one its URL instead of the held-out repository's, since the
  // script reads no git configuration: the script runs as it is, its URL unchanged.
  const dir = mkdtempSync(join(tmpdir(), "mente-held-out-"));
  const token = "held-out-token-0123456789abcdef";
  try {
    const origin = join(dir, "origin");
    mkdirSync(origin);
    const git = (cwd: string, ...args: string[]) => spawnSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
    git(origin, "init", "-q");
    writeFileSync(join(origin, "secret-task-name.md"), "held out\n");
    git(origin, "add", ".");
    git(origin, "commit", "-q", "-m", "SECRETSUBJECT of a held-out task");
    const commit = git(origin, "rev-parse", "HEAD").stdout.trim();
    const shims = join(dir, "shims");
    mkdirSync(shims);
    writeFileSync(join(shims, "git"), '#!/usr/bin/env bash\nargs=()\nfor a in "$@"; do if [ "$a" = https://github.com/Menteion/mente-bench-heldout ]; then args+=("$ORIGIN_URL"); else args+=("$a"); fi; done\nexec "$REAL_GIT" "${args[@]}"\n', { mode: 0o755 });
    const realGit = spawnSync("bash", ["-c", "command -v git"], { encoding: "utf8" }).stdout.trim();
    const run = (work: string, env: Record<string, string>, before?: () => void, input = `${token}\n`) => {
      mkdirSync(work, { recursive: true });
      before?.();
      // As the step starts it: a cleared environment but PATH, HOME and the commit, the token on its
      // standard input (the re-check of c55cf8f: on env's command line it was readable in /proc).
      const r = spawnSync("bash", ["--noprofile", "--norc", join(REPOSITORY, "benchmark", "harness", "fetch-held-out.sh")], {
        cwd: work,
        env: { PATH: `${shims}${delimiter}${process.env.PATH ?? ""}`, HOME: join(dir, "home"), REAL_GIT: realGit, ORIGIN_URL: pathToFileURL(origin).href, ...env },
        input,
        encoding: "utf8",
        timeout: 60_000,
      });
      // The mask is the runner's to take; every other line is the log's.
      return { status: r.status, printed: `${r.stdout}${r.stderr}`.split("\n").filter((l) => !l.startsWith("::add-mask::")).join("\n") };
    };
    const leaks = (printed: string) => ["SECRETSUBJECT", "secret-task-name", "HEAD is now", token, commit.slice(0, 7)].filter((x) => printed.includes(x));
    const fetched = run(join(dir, "a"), { HELD_OUT_COMMIT: commit });
    assert.equal(fetched.status, 0, fetched.printed);
    assert.deepEqual(leaks(fetched.printed), [], fetched.printed);
    assert.equal(fetched.printed.trim(), "the held-out set is at the commit corpus.lock names");
    assert.equal(git(join(dir, "a", "held-out-set"), "rev-parse", "HEAD").stdout.trim(), commit);
    assert.ok(existsSync(join(dir, "a", "held-out-set", "secret-task-name.md")));
    // The token is in no file of the checkout.
    assert.ok(!readFileSync(join(dir, "a", "held-out-set", ".git", "config"), "utf8").includes("extraheader"));
    // held-out-set there already, as a directory, a file or a link, is refused, and nothing goes
    // where a link points (the review of aafe59b: the checkout landed in the link's target).
    const target = join(dir, "target");
    mkdirSync(target);
    for (const [name, make] of [
      ["a directory", (w: string) => mkdirSync(join(w, "held-out-set"))],
      ["a file", (w: string) => writeFileSync(join(w, "held-out-set"), "x")],
      ["a link", (w: string) => symlinkSync(target, join(w, "held-out-set"), "junction")],
    ] as [string, (w: string) => void][]) {
      const work = join(dir, `there-${name.replace(/ /g, "-")}`);
      const r = run(work, { HELD_OUT_COMMIT: commit }, () => make(work));
      assert.notEqual(r.status, 0, `held-out-set there as ${name} passed`);
      assert.match(r.printed, /held-out-set is already there/, name);
    }
    assert.deepEqual(readdirSync(target), [], "the checkout went where the link pointed");
    // A commit corpus.lock does not name in full, one the repository does not hold, and no token
    // each fail saying only that.
    const short = run(join(dir, "b"), { HELD_OUT_COMMIT: commit.slice(0, 12) });
    assert.notEqual(short.status, 0);
    assert.match(short.printed, /no full commit id/);
    const missing = run(join(dir, "c"), { HELD_OUT_COMMIT: "f".repeat(40) });
    assert.notEqual(missing.status, 0);
    assert.match(missing.printed, /could not be fetched/);
    const none = run(join(dir, "d"), { HELD_OUT_COMMIT: commit }, undefined, "");
    assert.notEqual(none.status, 0);
    assert.match(none.printed, /the held-out token is not set/);
    // A token in the environment alone is not read: the script takes it from its standard input.
    const inEnv = run(join(dir, "e"), { HELD_OUT_TOKEN: token, HELD_OUT_COMMIT: commit }, undefined, "");
    assert.notEqual(inEnv.status, 0);
    assert.match(inEnv.printed, /the held-out token is not set/);
    for (const r of [short, missing, none, inEnv]) assert.deepEqual(leaks(r.printed), [], r.printed);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("start-broker.sh starts the broker alone, its socket the runner's user's, its core limit 0, its environment PATH", { skip: LINUX_ONLY }, async () => {
  const temp = mkdtempSync(join(tmpdir(), "mente-start-"));
  const dir = join(temp, "broker");
  // A tool cache of its own, laid out as setup-node lays out the runner's, with the Node that runs
  // this test where start-broker.sh names Node by its path: no variable of the runner's needed.
  const tools = join(temp, "tools");
  mkdirSync(join(tools, "node", PERMISSION.node, "x64", "bin"), { recursive: true });
  symlinkSync(process.execPath, join(tools, "node", PERMISSION.node, "x64", "bin", "node"));
  const env = { PATH: process.env.PATH ?? "", ANTHROPIC_API_KEY: KEY, BUDGET: "1", RUNNER_TEMP: temp, GITHUB_WORKSPACE: REPOSITORY, RUNNER_TOOL_CACHE: tools };
  try {
    // In a privileged bash, as its step starts it.
    const started = spawnSync("/usr/bin/bash", ["-p", join(REPOSITORY, "benchmark", "harness", "start-broker.sh")], { env, encoding: "utf8", timeout: 60_000 });
    assert.equal(started.status, 0, started.stdout + started.stderr);
    // Its directory, socket, log and pid file are the runner's user's alone.
    assert.equal(statSync(dir).mode & 0o777, 0o700, "the broker's directory");
    for (const f of ["socket", "log", "pid"]) assert.equal(statSync(join(dir, f)).mode & 0o777, 0o600, `the broker's ${f}`);
    const pid = readFileSync(join(dir, "pid"), "utf8").trim();
    // No core file at any size, a hard limit no process of the user can raise again.
    assert.match(readFileSync(`/proc/${pid}/limits`, "utf8"), /Max core file size\s+0\s+0\s/);
    // An environment of PATH alone: no key, and no NODE_OPTIONS that could widen its flags.
    assert.deepEqual(readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").filter(Boolean).map((e) => e.split("=").at(0)), ["PATH"]);
    // It answers on its socket, and sends nothing past the shape.
    const link = await connectBroker(join(dir, "socket"));
    await assert.rejects(link.post({ ...haiku(), speed: "fast" }), /speed/);
    link.close();
    const stopped = spawnSync("bash", [join(REPOSITORY, "benchmark", "harness", "stop-broker.sh")], { env, encoding: "utf8", timeout: 60_000 });
    assert.equal(stopped.status, 0, stopped.stderr);
    assert.ok(![started.stdout, started.stderr, stopped.stdout, readFileSync(join(dir, "log"), "utf8")].some((t) => t.includes(KEY)), "the key in what the broker or its scripts wrote");
  } finally {
    try {
      process.kill(Number(readFileSync(join(dir, "pid"), "utf8").trim()));
    } catch {
      // already stopped
    }
    rmSync(temp, { recursive: true, force: true });
  }
});

test("the sandbox's root half drops only to its caller, takes no path but a directory under its runs root, and starts the command with descriptors 0, 1 and 2", { skip: LINUX_ONLY }, () => {
  const script = join(REPOSITORY, "benchmark", "harness", "sandbox-root.sh");
  const installed = "/usr/local/sbin/mente-sandbox";
  const uid = process.getuid?.() ?? -1;
  if (uid !== 0 && existsSync(installed)) {
    // As the runner's user once lock-sudo.sh has installed it (the harness's fixtures job): sudo
    // runs it, and it runs the command as this user, and nothing else runs.
    const { runs } = readSandboxRoots();
    const dir = mkdtempSync(join(runs, "root-half-"));
    try {
      const sudo = (args: string[]) => spawnSync("sudo", ["-n", installed, ...args], { encoding: "utf8", timeout: 60_000 });
      const ran = sudo([dir, "/usr/bin/id", "-u"]);
      assert.equal(ran.stdout.trim(), String(uid), ran.stderr);
      // The command holds descriptors 0, 1 and 2 alone: the final pass of 05fe2f6 found it holding
      // the root half's own, each on a directory the sandbox covers.
      const fds = sudo([dir, "/bin/sh", "-c", "ls /proc/$$/fd"]);
      assert.deepEqual(fds.stdout.trim().split(/\s+/), ["0", "1", "2"], fds.stderr);
      // sudo closes every descriptor above 2 itself, and lets no caller keep one open into the root
      // half: the rule allows no -C. That the root half closes those it holds is shown where only
      // a call as root can show it, by lock-sudo.sh, which calls the installed root half holding
      // descriptors 7, 8 and 9 before the rule stands, and fails the lock if they reach the command.
      const held = [openSync(REPOSITORY, "r"), openSync(runs, "r"), openSync(SANDBOX_CONF, "r")];
      try {
        const kept = spawnSync("sudo", ["-n", "-C", "10", installed, dir, "/bin/sh", "-c", "ls /proc/$$/fd"], {
          encoding: "utf8",
          timeout: 60_000,
          stdio: ["ignore", "pipe", "pipe", "ignore", "ignore", "ignore", "ignore", ...held],
        });
        assert.notEqual(kept.status, 0, `sudo let its caller keep descriptors open into the root half: ${kept.stdout}`);
        assert.match(kept.stderr, /-C|close/i, kept.stderr);
      } finally {
        for (const fd of held) closeSync(fd);
      }
      // No path but the working directory, and that one under the runs root and the caller's: the
      // final pass of 05fe2f6 had --hide open /etc/shadow as root for the command to read.
      assert.match(sudo(["--hide", "/etc/shadow", dir, "/bin/sh", "-c", "cat <&3"]).stderr, /takes no option/);
      assert.match(sudo(["/etc", "/usr/bin/id", "-u"]).stderr, /does not lie under the runs root/);
      assert.match(sudo([tmpdir(), "/usr/bin/id", "-u"]).stderr, /does not lie under the runs root/);
      assert.match(sudo([runs, "/usr/bin/id", "-u"]).stderr, /does not lie under the runs root/);
      assert.match(sudo([join(dir, "missing"), "/usr/bin/id"]).stderr, /no directory/);
      assert.notEqual(spawnSync("sudo", ["-n", "true"]).status, 0, "sudo runs another command as root");
      assert.notEqual(spawnSync("sudo", ["-n", "cat", "/etc/shadow"]).status, 0, "sudo reads what only root reads");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
    return;
  }
  // A developer's machine: nothing to call it as.
  if (uid !== 0) return;
  // As root, with sudo's variables set by hand (a container): each refusal, before any namespace.
  const dir = mkdtempSync(join(tmpdir(), "mente-root-"));
  try {
    const run = (vars: Record<string, string>, args: string[]) => spawnSync("bash", [script, ...args], { env: { PATH: process.env.PATH ?? "", ...vars }, encoding: "utf8", timeout: 60_000 });
    assert.match(run({}, [dir, "id"]).stderr, /never runs a command as root/);
    assert.match(run({ SUDO_UID: "0", SUDO_GID: "0" }, [dir, "id"]).stderr, /never runs a command as root/);
    assert.match(run({ SUDO_UID: "x", SUDO_GID: "1" }, [dir, "id"]).stderr, /never runs a command as root/);
    assert.match(run({ SUDO_UID: "1000", SUDO_GID: "1000" }, ["--hide", "/etc/shadow", dir, "id"]).stderr, /takes no option/);
    if (!existsSync("/etc/mente-sandbox.conf")) assert.match(run({ SUDO_UID: "1000", SUDO_GID: "1000" }, [dir, "id"]).stderr, /mente-sandbox\.conf is missing/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the broker starts under its pinned flags, reads its key and limits, and refuses beyond them without sending", async () => {
  const dir = mkdtempSync(join(tmpdir(), "mente-broker-"));
  const at = socketPath(dir, "broker");
  const script = join(REPOSITORY, "benchmark", "harness", "broker.ts");
  const child = spawn(process.execPath, [...brokerFlags([script, join(REPOSITORY, "benchmark", "harness", "broker-limits.json")]), script, "--socket", at, "--budget", "1"], {
    cwd: REPOSITORY,
    env: cleared(),
    stdio: ["pipe", "pipe", "pipe"],
  });
  let log = "";
  child.stderr.on("data", (d) => (log += d));
  child.stdout.on("data", (d) => (log += d));
  child.stdin.end(`${KEY}\n`);
  try {
    let link: Awaited<ReturnType<typeof connectBroker>> | undefined;
    for (let k = 0; k < 100 && !link; k++) {
      link = await connectBroker(at).catch(() => undefined);
      if (!link) await new Promise((done) => setTimeout(done, 100));
    }
    assert.ok(link, `the broker did not listen: ${log}`);
    await assert.rejects(link.post({ ...haiku(), model: "gpt-x" }), /does not name/);
    await assert.rejects(link.post(haiku(MAX_TOKENS + 1)), /beyond 1 to/);
    link.close();
    assert.match(log, /the broker listens at/);
    assert.ok(!log.includes(KEY), "the broker wrote its key");
  } finally {
    child.kill();
    rmSync(dir, { recursive: true, force: true });
  }
});
