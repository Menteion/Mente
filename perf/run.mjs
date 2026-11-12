// Builds and measures the tight-loop kernels of perf/kernels: each kernel in Mente, C, C++ and
// Rust, the checksums compared, wall time over several rounds, and hardware counters through
// `perf stat` where the platform gives them. With a base compiler it also measures the base's
// Mente code beside the change's and fails when the change executes more instructions than the
// base beyond the tolerance below; a slower wall time is reported, never failed (perf/README.md,
// and "Measurement protocols" in ROADMAP.md). With --require-instructions, a kernel whose
// instructions perf did not count fails, so that a gate without counters cannot pass by measuring
// nothing.
//
// Usage:
//   node perf/run.mjs --mn <mn> [--base-mn <mn>] [--require-instructions] [--out <results.json>]
//                     [--work <dir>] [--rounds <n>] [--only <kernel,...>] [--scale <factor>]
//
// Runs on Linux. The compilers are clang-18, clang++-18 and the rustc on PATH, unless PERF_CC,
// PERF_CXX or PERF_RUSTC name others. Benchmarks run on GitHub Actions only, never on a laptop.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));

/// Each kernel, and the work count its function takes: repetitions, or for `collatz` the limit.
/// Chosen so that the fastest language's run lasts about a tenth of a second or more, except
/// `bsearch`, whose count keeps Mente's run within the workflow's time (perf/README.md).
export const KERNELS = [
  { name: "sum", n: 300000 },
  { name: "dot", n: 200000 },
  { name: "prefix", n: 150000 },
  { name: "matmul", n: 15000 },
  { name: "sieve", n: 600 },
  { name: "bsearch", n: 400 },
  { name: "popcount", n: 35000 },
  { name: "isort", n: 25000 },
  { name: "collatz", n: 1200000 },
];

/// The regression gate: a median instruction count of the change's Mente code more than
/// INSTRUCTION_TOLERANCE above the base's fails; a median wall time more than TIME_TOLERANCE above
/// it is reported and does not fail, since shared runners' times vary from run to run (wall time
/// is advisory; docs/design-review.md, guard-per-problem). perf/README.md gives the measurements
/// behind both figures.
export const TIME_TOLERANCE = 0.05;
export const INSTRUCTION_TOLERANCE = 0.01;

/// The events asked of `perf stat`. `duration_time` is the wall time of the measured process
/// alone, without perf's own start-up; the others are reported when the platform counts them.
export const EVENTS = [
  "duration_time",
  "task-clock",
  "cycles",
  "instructions",
  "branches",
  "branch-misses",
  "cache-references",
  "cache-misses",
];

/// The languages each kernel is built in, in the order they are reported. `mente` is the
/// change's compiler, `mente-base` the base's. `rust-checked` is Rust with overflow checks on,
/// reported beside the others and never counted in the gap.
export const REFERENCES = ["c", "cpp", "rust"];

// ------------------------------------------------------------------ statistics and parsing

export function median(values) {
  const v = [...values].sort((a, b) => a - b);
  if (v.length === 0) return NaN;
  const mid = Math.floor(v.length / 2);
  return v.length % 2 === 1 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
}

/// The median, minimum and maximum of a series, and its spread: (max - min) / median.
export function summarize(values) {
  const m = median(values);
  const min = Math.min(...values);
  const max = Math.max(...values);
  return { median: m, min, max, spread: m > 0 ? (max - min) / m : NaN };
}

/// Reads the CSV `perf stat -x,` writes: one counter per line, value first and event name
/// third. Returns `{ event: value }` for counted events and `{ event: null }` for events perf
/// marks `<not supported>` or `<not counted>`. Modifiers such as `:u` and PMU prefixes such as
/// `cpu_core/cycles/` are stripped from the name. Times are in nanoseconds.
export function parsePerfCsv(text) {
  const counters = {};
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() === "" || line.startsWith("#")) continue;
    const fields = line.split(",");
    if (fields.length < 3) continue;
    let name = fields[2].trim();
    const pmu = /^[\w-]+\/([\w-]+)\/?$/.exec(name);
    if (pmu) name = pmu[1];
    name = name.replace(/:[a-zA-Z]+$/, "");
    const raw = fields[0].trim();
    if (raw.startsWith("<")) {
      counters[name] = null;
      continue;
    }
    let value = Number(raw);
    if (!Number.isFinite(value)) continue;
    const unit = fields[1].trim();
    if (unit === "msec") value *= 1e6;
    else if (unit === "usec") value *= 1e3;
    else if (unit === "sec" || unit === "s") value *= 1e9;
    counters[name] = value;
  }
  return counters;
}

/// The gate's verdict on one kernel, from the base's and the change's measurements: `reasons` fail
/// it, `warnings` are reported. Identical executables cannot regress, so they always pass; their
/// ratios still show the noise. With `requireInstructions`, uncounted instructions fail even them.
export function gate(base, change, tolerances = {}, { requireInstructions = false } = {}) {
  const time = tolerances.time ?? TIME_TOLERANCE;
  const instructions = tolerances.instructions ?? INSTRUCTION_TOLERANCE;
  const timeRatio = change.time.median / base.time.median;
  const bi = base.counters?.instructions;
  const ci = change.counters?.instructions;
  const instructionRatio = bi && ci ? ci / bi : null;
  const identical = Boolean(change.sha256 && change.sha256 === base.sha256);
  const reasons = [];
  const warnings = [];
  if (requireInstructions && instructionRatio === null) {
    reasons.push("perf counted no instructions, which this gate requires");
  }
  if (!identical) {
    if (timeRatio > 1 + time) {
      warnings.push(`median time ${pct(timeRatio - 1)} above the base's (tolerance ${pct(time)}, reported only)`);
    }
    if (instructionRatio !== null && instructionRatio > 1 + instructions) {
      reasons.push(
        `instructions ${pct(instructionRatio - 1)} above the base's (tolerance ${pct(instructions)})`,
      );
    }
  }
  return { identical, timeRatio, instructionRatio, pass: reasons.length === 0, reasons, warnings };
}

/// Mente's median time over the fastest of C, C++ and Rust.
export function gap(results) {
  const refs = REFERENCES.map((l) => results[l]?.time.median).filter((t) => Number.isFinite(t));
  if (!results.mente || refs.length === 0) return null;
  const fastest = Math.min(...refs);
  const lang = REFERENCES.find((l) => results[l]?.time.median === fastest);
  return { ratio: results.mente.time.median / fastest, fastest: lang };
}

export function pct(x) {
  return `${(x * 100).toFixed(1)}%`;
}

// ------------------------------------------------------------------------ the measurement

function run(cmd, args, options = {}) {
  const r = spawnSync(cmd, args, { encoding: "utf8", maxBuffer: 1 << 26, ...options });
  if (r.error) throw new Error(`${cmd}: ${r.error.message}`);
  return r;
}

function must(cmd, args, options) {
  const r = run(cmd, args, options);
  if (r.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} exited with ${r.status}:\n${r.stdout}${r.stderr}`);
  }
  return r.stdout;
}

function firstLine(text) {
  return (text || "").split(/\r?\n/).find((l) => l.trim() !== "") ?? "";
}

function machine() {
  let cpu = os.cpus()[0]?.model ?? "unknown";
  try {
    const lscpu = must("lscpu", []);
    const model = /^Model name:\s*(.+)$/m.exec(lscpu);
    if (model) cpu = model[1].trim();
  } catch {
    // os.cpus() is enough.
  }
  let osName = `${os.type()} ${os.release()}`;
  try {
    const release = fs.readFileSync("/etc/os-release", "utf8");
    const pretty = /^PRETTY_NAME="?([^"\n]+)"?$/m.exec(release);
    if (pretty) osName = `${pretty[1]}, kernel ${os.release()}`;
  } catch {
    // Not Linux, or no os-release.
  }
  return { cpu, arch: os.arch(), cores: os.cpus().length, os: osName };
}

/// What `perf stat` gives on this machine: whether it runs at all, the paranoia level, and
/// which of EVENTS it counts.
function probePerf() {
  const result = { available: false, paranoid: null, events: {}, version: null };
  try {
    result.paranoid = fs.readFileSync("/proc/sys/kernel/perf_event_paranoid", "utf8").trim();
  } catch {
    // Not Linux.
  }
  let v;
  try {
    v = run("perf", ["--version"]);
  } catch (e) {
    result.error = e.message;
    return result;
  }
  if (v.status !== 0) {
    result.error = firstLine(v.stderr) || firstLine(v.stdout);
    return result;
  }
  result.version = firstLine(v.stdout);
  const out = path.join(os.tmpdir(), `perf-probe-${process.pid}.csv`);
  const r = run("perf", ["stat", "-x", ",", "-o", out, "-e", EVENTS.join(","), "--", "true"]);
  if (r.status !== 0) {
    result.error = firstLine(r.stderr);
    return result;
  }
  const counters = parsePerfCsv(fs.readFileSync(out, "utf8"));
  fs.rmSync(out, { force: true });
  result.available = "duration_time" in counters && counters.duration_time !== null;
  for (const e of EVENTS) result.events[e] = e in counters && counters[e] !== null;
  return result;
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

/// Builds one kernel in every language into `work/<language>/<kernel>`. A Mente build the
/// base's compiler cannot make is left out, with the reason.
function build(kernel, tools, work, n) {
  const src = (ext) => path.join(HERE, "kernels", `${kernel}.${ext}`);
  const out = (lang) => {
    fs.mkdirSync(path.join(work, lang), { recursive: true });
    return path.join(work, lang, kernel);
  };
  const built = {};
  // Both Mente compilers write the executable at one path, then it is moved, so that nothing
  // in it differs between them but the code.
  const scratch = path.join(work, "scratch", kernel);
  fs.mkdirSync(path.dirname(scratch), { recursive: true });
  for (const [lang, mn] of [
    ["mente", tools.mn],
    ["mente-base", tools.baseMn],
  ]) {
    if (!mn) continue;
    const r = run(mn, ["build", src("mn"), "--call", `${kernel}(${n})`, "-o", scratch]);
    if (r.status !== 0) {
      if (lang === "mente") throw new Error(`mn build ${kernel}:\n${r.stdout}${r.stderr}`);
      built[lang] = { error: firstLine(r.stderr) || firstLine(r.stdout) };
      continue;
    }
    fs.renameSync(scratch, out(lang));
    for (const extra of [".o", ".obj", ".pdb"]) fs.rmSync(scratch + extra, { force: true });
    built[lang] = { exe: out(lang), args: [] };
  }
  must(tools.cc, ["-O2", "-std=c17", "-o", out("c"), src("c")]);
  built.c = { exe: out("c"), args: [String(n)] };
  must(tools.cxx, ["-O2", "-std=c++17", "-o", out("cpp"), src("cpp")]);
  built.cpp = { exe: out("cpp"), args: [String(n)] };
  must(tools.rustc, ["--edition", "2021", "-C", "opt-level=2", "-o", out("rust"), src("rs")]);
  built.rust = { exe: out("rust"), args: [String(n)] };
  must(tools.rustc, [
    "--edition",
    "2021",
    "-C",
    "opt-level=2",
    "-C",
    "overflow-checks=on",
    "-o",
    out("rust-checked"),
    src("rs"),
  ]);
  built["rust-checked"] = { exe: out("rust-checked"), args: [String(n)] };
  return built;
}

/// One timed run: its checksum, its wall time in nanoseconds, and its counters when perf runs.
function measureOnce(binary, perf) {
  if (perf.available) {
    const out = path.join(os.tmpdir(), `perf-run-${process.pid}.csv`);
    const r = run("perf", [
      "stat",
      "-x",
      ",",
      "-o",
      out,
      "-e",
      EVENTS.join(","),
      "--",
      binary.exe,
      ...binary.args,
    ]);
    if (r.status !== 0) throw new Error(`${binary.exe} exited with ${r.status}: ${r.stderr}`);
    const counters = parsePerfCsv(fs.readFileSync(out, "utf8"));
    fs.rmSync(out, { force: true });
    return { checksum: r.stdout.trim(), ns: counters.duration_time, counters };
  }
  const start = process.hrtime.bigint();
  const r = run(binary.exe, binary.args);
  const ns = Number(process.hrtime.bigint() - start);
  if (r.status !== 0) throw new Error(`${binary.exe} exited with ${r.status}: ${r.stderr}`);
  return { checksum: r.stdout.trim(), ns, counters: {} };
}

/// Measures one kernel's executables over `rounds` rounds. Every round runs each executable
/// once, starting one further along the list each time, so that drift on the machine falls on
/// every language alike. The first, untimed run checks that every checksum agrees.
function measure(kernel, built, perf, rounds) {
  const langs = Object.keys(built).filter((l) => built[l].exe);
  const checksums = {};
  for (const l of langs) checksums[l] = measureOnce(built[l], { available: false }).checksum;
  const distinct = new Set(Object.values(checksums));
  if (distinct.size !== 1) {
    throw new Error(`${kernel}: the checksums differ: ${JSON.stringify(checksums)}`);
  }
  const samples = Object.fromEntries(langs.map((l) => [l, []]));
  for (let round = 0; round < rounds; round++) {
    for (let i = 0; i < langs.length; i++) {
      const l = langs[(i + round) % langs.length];
      samples[l].push(measureOnce(built[l], perf));
    }
  }
  const results = {};
  for (const l of langs) {
    const counters = {};
    for (const e of EVENTS) {
      const values = samples[l].map((s) => s.counters[e]).filter((v) => typeof v === "number");
      if (values.length === samples[l].length && values.length > 0) counters[e] = median(values);
    }
    results[l] = {
      time: summarize(samples[l].map((s) => s.ns)),
      counters,
      sha256: sha256(built[l].exe),
    };
  }
  for (const l of Object.keys(built)) {
    if (built[l].error) results[l] = { error: built[l].error };
  }
  return { checksum: [...distinct][0], results };
}

// ---------------------------------------------------------------------------- the report

function ms(ns) {
  return (ns / 1e6).toFixed(ns < 1e8 ? 1 : 0);
}

function cell(r) {
  if (!r) return "";
  if (r.error) return "not built";
  return `${ms(r.time.median)} ms ±${(r.time.spread * 50).toFixed(1)}%`;
}

function big(v) {
  if (v === undefined) return "";
  if (v >= 1e9) return `${(v / 1e9).toFixed(2)} G`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(1)} M`;
  if (v >= 1e3) return `${(v / 1e3).toFixed(1)} k`;
  return String(Math.round(v));
}

const LABELS = {
  mente: "Mente",
  "mente-base": "Mente (base)",
  c: "C",
  cpp: "C++",
  rust: "Rust",
  "rust-checked": "Rust, overflow checks",
};

export function report(results) {
  const lines = [];
  const m = results.machine;
  lines.push(`### Tight-loop kernels: ${m.arch}, ${m.cpu}`);
  lines.push("");
  lines.push(`${m.os}; ${m.cores} cores. ${results.toolchains.join("; ")}.`);
  lines.push("");
  const p = results.perf;
  const counted = Object.keys(p.events).filter((e) => p.events[e]);
  lines.push(
    p.available
      ? `perf: ${p.version}, perf_event_paranoid ${p.paranoid}; counted: ${counted.join(", ")}; not counted: ${Object.keys(p.events).filter((e) => !p.events[e]).join(", ") || "none"}.`
      : `perf: unavailable (${p.error || p.version || "not installed"}), perf_event_paranoid ${p.paranoid}; wall time from the parent process.`,
  );
  lines.push("");
  const hasBase = results.kernels.some((k) => k.results["mente-base"]);
  const head = ["Kernel", "Mente", "C", "C++", "Rust", "Gap", "Rust, overflow checks"];
  if (hasBase) head.push("Mente (base)", "Change / base");
  lines.push(`| ${head.join(" | ")} |`);
  lines.push(`|${head.map(() => " :--- ").join("|")}|`);
  for (const k of results.kernels) {
    const r = k.results;
    const g = k.gap ? `${k.gap.ratio.toFixed(2)}× ${LABELS[k.gap.fastest]}` : "";
    const row = [k.name, cell(r.mente), cell(r.c), cell(r.cpp), cell(r.rust), g, cell(r["rust-checked"])];
    if (hasBase) {
      const v = k.gate;
      row.push(
        cell(r["mente-base"]),
        v
          ? `${v.timeRatio.toFixed(3)}${v.instructionRatio ? `, instr. ${v.instructionRatio.toFixed(4)}` : ""}${v.identical ? " (identical code)" : ""}${v.pass ? "" : " **FAIL**"}${v.warnings?.length ? " (slower, reported)" : ""}`
          : "",
      );
    }
    lines.push(`| ${row.join(" | ")} |`);
  }
  lines.push("");
  lines.push(
    "Time: the median of the rounds, ± half the range over the median. Gap: Mente's median over the fastest of C, C++ and Rust.",
  );
  if (counted.some((e) => e !== "duration_time" && e !== "task-clock")) {
    lines.push("");
    lines.push("| Kernel | Language | Instructions | IPC | Branch misses | Cache misses |");
    lines.push("| :--- | :--- | ---: | ---: | ---: | ---: |");
    for (const k of results.kernels) {
      for (const l of Object.keys(LABELS)) {
        const r = k.results[l];
        if (!r || r.error) continue;
        const c = r.counters;
        const ipc = c.instructions && c.cycles ? (c.instructions / c.cycles).toFixed(2) : "";
        const bm =
          c["branch-misses"] !== undefined && c.branches
            ? `${big(c["branch-misses"])} (${pct(c["branch-misses"] / c.branches)})`
            : big(c["branch-misses"]);
        const cm =
          c["cache-misses"] !== undefined && c["cache-references"]
            ? `${big(c["cache-misses"])} (${pct(c["cache-misses"] / c["cache-references"])})`
            : big(c["cache-misses"]);
        lines.push(`| ${k.name} | ${LABELS[l]} | ${big(c.instructions)} | ${ipc} | ${bm} | ${cm} |`);
      }
    }
  }
  if (hasBase) {
    lines.push("");
    const failed = results.kernels.filter((k) => k.gate && !k.gate.pass);
    const slower = results.kernels.filter((k) => k.gate?.warnings?.length);
    lines.push(
      failed.length === 0
        ? `Gate: no kernel executed more than ${pct(INSTRUCTION_TOLERANCE)} more instructions than the base${results.requireInstructions ? "" : " where they were counted"}.`
        : `Gate: FAILED. ${failed.map((k) => `${k.name}: ${k.gate.reasons.join("; ")}`).join(". ")}.`,
    );
    if (slower.length > 0) {
      lines.push("");
      lines.push(`Wall time, reported only: ${slower.map((k) => `${k.name}: ${k.gate.warnings.join("; ")}`).join(". ")}.`);
    }
  }
  return lines.join("\n") + "\n";
}

// ------------------------------------------------------------------------------- the main

function parseArgs(argv) {
  const args = { rounds: 11, scale: 1 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => argv[++i];
    if (a === "--mn") args.mn = path.resolve(next());
    else if (a === "--base-mn") args.baseMn = path.resolve(next());
    else if (a === "--out") args.out = path.resolve(next());
    else if (a === "--work") args.work = path.resolve(next());
    else if (a === "--rounds") args.rounds = Number(next());
    else if (a === "--only") args.only = next().split(",");
    else if (a === "--scale") args.scale = Number(next());
    else if (a === "--require-instructions") args.requireInstructions = true;
    else throw new Error(`unknown argument ${a}`);
  }
  if (!args.mn) throw new Error("usage: node perf/run.mjs --mn <mn> [--base-mn <mn>] [--out <json>]");
  if (args.requireInstructions && !args.baseMn) throw new Error("--require-instructions needs --base-mn");
  return args;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const tools = {
    mn: args.mn,
    baseMn: args.baseMn,
    cc: process.env.PERF_CC || "clang-18",
    cxx: process.env.PERF_CXX || "clang++-18",
    rustc: process.env.PERF_RUSTC || "rustc",
  };
  const work = args.work || fs.mkdtempSync(path.join(os.tmpdir(), "mente-perf-"));
  const rustc = must(tools.rustc, ["-vV"]);
  const toolchains = [
    `${tools.cc}: ${firstLine(must(tools.cc, ["--version"]))}`,
    `${tools.cxx}: ${firstLine(must(tools.cxx, ["--version"]))}`,
    `rustc ${/^release: (.+)$/m.exec(rustc)?.[1]} (LLVM ${/^LLVM version: (.+)$/m.exec(rustc)?.[1]})`,
    "C and C++ at -O2, Rust at -C opt-level=2, Mente through Cranelift's speed level, all for the baseline CPU",
  ];
  const perf = probePerf();
  const results = {
    date: new Date().toISOString(),
    machine: machine(),
    toolchains,
    perf,
    rounds: args.rounds,
    tolerances: { time: TIME_TOLERANCE, instructions: INSTRUCTION_TOLERANCE },
    requireInstructions: Boolean(args.requireInstructions),
    kernels: [],
  };
  for (const k of KERNELS) {
    if (args.only && !args.only.includes(k.name)) continue;
    const n = Math.max(1, Math.round(k.n * args.scale));
    process.stderr.write(`${k.name}(${n}): building\n`);
    const built = build(k.name, tools, work, n);
    process.stderr.write(`${k.name}(${n}): measuring ${args.rounds} rounds\n`);
    const m = measure(k.name, built, perf, args.rounds);
    const entry = { name: k.name, n, checksum: m.checksum, results: m.results, gap: gap(m.results) };
    if (m.results["mente-base"]?.time) {
      entry.gate = gate(m.results["mente-base"], m.results.mente, {}, { requireInstructions: args.requireInstructions });
    }
    results.kernels.push(entry);
  }
  const text = report(results);
  process.stdout.write(text);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  if (args.out) fs.writeFileSync(args.out, JSON.stringify(results, null, 2) + "\n");
  if (results.kernels.some((k) => k.gate && !k.gate.pass)) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    process.stderr.write(`error: ${e.message}\n`);
    process.exit(2);
  }
}
