// Tests of check.mjs: each copies the public corpus to a temporary directory,
// breaks it in one way, and requires the checker to reject it with the right
// message, so that no check of the corpus is one that cannot fail.
// Run with `npm run test:benchmark`.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));

function copy() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-benchmark-"));
  for (const f of ["README.md", "check.mjs", "reference.mjs", "corpus.lock"]) fs.copyFileSync(path.join(HERE, f), path.join(dir, f));
  fs.cpSync(path.join(HERE, "tasks"), path.join(dir, "tasks"), { recursive: true });
  return dir;
}

function edit(file, from, to) {
  const s = fs.readFileSync(file, "utf8");
  assert.equal(s.split(from).length, 2, `${path.basename(file)} should hold ${from} once`);
  fs.writeFileSync(file, s.replace(from, to));
}

function check(dir) {
  const r = spawnSync(process.execPath, [path.join(dir, "check.mjs")], { encoding: "utf8" });
  fs.rmSync(dir, { recursive: true, force: true });
  return { ok: r.status === 0, out: r.stdout + r.stderr };
}

function rejects(mutate, message) {
  const dir = copy();
  mutate(dir);
  const r = check(dir);
  assert.equal(r.ok, false, `the checker accepted a broken corpus:\n${r.out}`);
  assert.match(r.out, message);
}

test("the corpus as it stands passes", () => {
  const r = check(copy());
  assert.equal(r.ok, true, r.out);
});

test("a wrong expected value is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "gcd.md"), "gcd(12, 18) == 6", "gcd(12, 18) == 7"), /the reference returns 6/);
});

test("an argument outside its type is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "reverse-bits.md"), "reverse_bits(1) ==", "reverse_bits(256) =="), /is outside u8/);
});

test("a result outside its refinement is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "percent.md"), "percent(5, 5) == 100", "percent(5, 5) == 101"), /above the refinement/);
});

test("a mut argument written without mut is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "reverse-in-place.md"), "reverse_in_place(mut [1, 2, 3, 4, 5, 6, 7, 8, 9, 10])", "reverse_in_place([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])"), /must be marked mut/);
});

test("a wrong recorded call is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "retry-get.md"), "then Clock.sleep_ms calls [100, 200, 400, 800, 1600]", "then Clock.sleep_ms calls [100, 200, 400, 800]"), /Clock.sleep_ms calls/);
});

test("a core mark that disagrees with the signature is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "clamp.md"), "core: false", "core: true"), /marked core: true/);
});

test("a task without a reference solution is rejected", () => {
  rejects((d) => edit(path.join(d, "reference.mjs"), "  gcd(_, a, b) {", "  gcd_renamed(_, a, b) {"), /no reference solution for gcd/);
});

test("a missing section is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "isqrt.md"), "## Examples", "## Example"), /missing section ## Examples/);
});

test("a corpus that does not match the composition table is rejected", () => {
  rejects((d) => fs.rmSync(path.join(d, "tasks", "step-up.md")), /composition: the adversarial set has 1 tasks for M1/);
  rejects((d) => fs.rmSync(path.join(d, "tasks", "gcd.md")), /reference.mjs has a solution for gcd, which is not a task/);
  rejects((d) => edit(path.join(d, "README.md"), "| M7 | 3 | 1 | 1 |", "| M7 | 3 | 1 | 2 |"), /composition: the adversarial set/);
});

test("a changed task that keeps its old digest is rejected", () => {
  rejects((d) => edit(path.join(d, "tasks", "gcd.md"), "Return the greatest common divisor", "Return the largest common divisor"), /corpus.lock does not match the tasks \(changed or new: gcd/);
});
