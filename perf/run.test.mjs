// Tests of perf/run.mjs's statistics, its reading of `perf stat` output and its regression
// gate. They measure nothing: node --test perf/run.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  KERNELS,
  INSTRUCTION_TOLERANCE,
  TIME_TOLERANCE,
  gap,
  gate,
  median,
  missingBase,
  parsePerfCsv,
  summarize,
} from "./lib.mjs";

test("a kernel the base built nothing for fails the instruction gate, unless it is new", () => {
  const failed = missingBase({ error: "E0109 at 3:5" }, { requireInstructions: true });
  assert.equal(failed.pass, false);
  assert.match(failed.reasons[0], /built no executable \(E0109/);
  assert.equal(missingBase(undefined, { requireInstructions: true }).pass, false);
  const fresh = missingBase({ error: "no such file" }, { requireInstructions: true, isNew: true });
  assert.equal(fresh.pass, true);
  assert.match(fresh.warnings[0], /new kernel/);
  const advisory = missingBase({ error: "x" }, { requireInstructions: false });
  assert.equal(advisory.pass, true);
  assert.equal(advisory.warnings.length, 1);
});

test("the CLI runs main whatever path starts it", () => {
  const cli = fs.readFileSync(path.join(HERE, "run.mjs"), "utf8");
  assert.doesNotMatch(cli, /process\.argv\[1\]|import\.meta\.url/);
  assert.match(cli, /^\s*main\(process\.argv\.slice\(2\)\);/m);
});

const HERE = path.dirname(fileURLToPath(import.meta.url));

test("median takes the middle value, or the mean of the two middle values", () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 3, 2]), 2.5);
  assert.ok(Number.isNaN(median([])));
});

test("summarize gives the spread as the range over the median", () => {
  const s = summarize([100, 90, 110, 105, 95]);
  assert.equal(s.median, 100);
  assert.equal(s.min, 90);
  assert.equal(s.max, 110);
  assert.equal(s.spread, 0.2);
});

test("parsePerfCsv reads counted events, units, modifiers and PMU prefixes", () => {
  const csv = [
    "# started on Tue Nov 10 10:00:00 2026",
    "",
    "1020304050,ns,duration_time,1020304050,100.00,,",
    "1012.50,msec,task-clock,1012500000,100.00,0.992,CPUs utilized",
    "3000000000,,cycles:u,1012500000,100.00,2.951,GHz",
    "6000000000,,cpu_core/instructions/,1012500000,100.00,2.00,insn per cycle",
    "<not supported>,,cache-misses,0,100.00,,",
    "<not counted>,,branch-misses,0,0.00,,",
  ].join("\n");
  const c = parsePerfCsv(csv);
  assert.equal(c.duration_time, 1020304050);
  assert.equal(c["task-clock"], 1012500000);
  assert.equal(c.cycles, 3000000000);
  assert.equal(c.instructions, 6000000000);
  assert.equal(c["cache-misses"], null);
  assert.equal(c["branch-misses"], null);
});

function measured(medianNs, instructions, sha256 = "a") {
  return {
    time: summarize([medianNs * 0.99, medianNs, medianNs * 1.01]),
    counters: instructions === undefined ? {} : { instructions },
    sha256,
  };
}

test("the gate passes a change within the tolerances", () => {
  const v = gate(measured(100e6, 1e9, "a"), measured(100e6 * (1 + TIME_TOLERANCE / 2), 1e9, "b"));
  assert.equal(v.pass, true);
  assert.equal(v.identical, false);
});

test("the gate reports a change slower than the base beyond the time tolerance, and passes it", () => {
  const v = gate(measured(100e6, 1e9, "a"), measured(100e6 * (1 + TIME_TOLERANCE * 1.5), 1e9, "b"));
  assert.equal(v.pass, true);
  assert.deepEqual(v.reasons, []);
  assert.match(v.warnings[0], /median time .* reported only/);
  const within = gate(measured(100e6, 1e9, "a"), measured(100e6 * (1 + TIME_TOLERANCE / 2), 1e9, "b"));
  assert.deepEqual(within.warnings, []);
});

test("the gate fails a run that must count instructions and counted none", () => {
  const uncounted = gate(measured(100e6, undefined, "a"), measured(100e6, undefined, "b"), {}, { requireInstructions: true });
  assert.equal(uncounted.pass, false);
  assert.match(uncounted.reasons[0], /counted no instructions/);
  const identical = gate(measured(100e6, undefined, "same"), measured(100e6, undefined, "same"), {}, { requireInstructions: true });
  assert.equal(identical.pass, false);
  const counted = gate(measured(100e6, 1e9, "a"), measured(100e6, 1e9, "b"), {}, { requireInstructions: true });
  assert.equal(counted.pass, true);
  const halfCounted = gate(measured(100e6, 1e9, "a"), measured(100e6, undefined, "b"), {}, { requireInstructions: true });
  assert.equal(halfCounted.pass, false);
});

test("the gate fails a change that executes more instructions beyond their tolerance", () => {
  const more = 1e9 * (1 + INSTRUCTION_TOLERANCE * 2);
  const v = gate(measured(100e6, 1e9, "a"), measured(100e6, more, "b"));
  assert.equal(v.pass, false);
  assert.match(v.reasons[0], /instructions/);
});

test("the gate judges time alone where perf counts no instructions", () => {
  const v = gate(measured(100e6, undefined, "a"), measured(101e6, undefined, "b"));
  assert.equal(v.pass, true);
  assert.equal(v.instructionRatio, null);
});

test("the gate always passes identical executables, and still reports their ratio", () => {
  const v = gate(measured(100e6, 1e9, "same"), measured(150e6, 1e9, "same"));
  assert.equal(v.pass, true);
  assert.equal(v.identical, true);
  assert.equal(v.timeRatio, 1.5);
});

test("the gap is Mente's median over the fastest of C, C++ and Rust", () => {
  const g = gap({
    mente: measured(300e6),
    c: measured(120e6),
    cpp: measured(100e6),
    rust: measured(150e6),
    "rust-checked": measured(50e6),
  });
  assert.equal(g.ratio, 3);
  assert.equal(g.fastest, "cpp");
});

test("every kernel exists in Mente, C, C++ and Rust", () => {
  for (const k of KERNELS) {
    for (const ext of ["mn", "c", "cpp", "rs"]) {
      assert.ok(fs.existsSync(path.join(HERE, "kernels", `${k.name}.${ext}`)), `${k.name}.${ext}`);
    }
  }
  const files = fs.readdirSync(path.join(HERE, "kernels"));
  for (const f of files) {
    const name = f.replace(/\.[^.]+$/, "");
    assert.ok(KERNELS.some((k) => k.name === name), `${f} belongs to no kernel in KERNELS`);
  }
});
