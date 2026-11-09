// Tests of the benchmark harness's task reader, signature mapping and prompts, with no network.

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import {
  costOf,
  extractProgram,
  menteSignature,
  menteType,
  parseTask,
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
