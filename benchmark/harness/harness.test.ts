// Tests of the benchmark harness's task reader, signature mapping and prompts, with no network.

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
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
