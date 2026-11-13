// The floor of work for "Mutants in the diff" (.github/workflows/mutants.yml; X6 of the guard
// audit): when a pull request changes code where cargo-mutants finds mutants, the run must test
// them, every one. The expected set here is independent of cargo-mutants' own diff filter: it takes
// every mutant cargo-mutants lists for the whole tree, under the BASE's .cargo/mutants.toml so that
// a pull request cannot exclude what it changes, and keeps those whose span holds a line the diff
// adds, and every mutant of each function that encloses a deleted line, which the job runs too
// (`deleted`). Each name of that set must be in the run's lists, or the floor fails (`missing`). A
// diff that changes only tests, comments or files outside the compiler holds no such mutant. An
// exclusion added to .cargo/mutants.toml therefore lands in a pull request of its own, before the
// change to the function it excludes.
//
//   node .github/scripts/mutants-floor.mjs expected <pr.diff> <base-all.json>
//   node .github/scripts/mutants-floor.mjs deleted <pr.diff> <all.json> <diff.json>
//   node .github/scripts/mutants-floor.mjs missing <pr.diff> <base-all.json> <diff.json> <deleted.json>
//   node .github/scripts/mutants-floor.mjs count <list.json>
//
// <all.json> is `cargo mutants --list --json` over the whole tree, <base-all.json> the same under
// the base's settings, <diff.json> cargo-mutants' list for the diff, <deleted.json> its list for
// the `deleted` regexes. `expected` prints how many mutants the diff must have tested; `deleted`
// prints, one a line, an anchored regex for each mutant around a deletion that <diff.json> leaves
// out, for `cargo mutants --re`; `missing` prints, one a line, each expected name neither list
// holds; `count` prints how many mutants a list holds: cargo-mutants prints nothing at all when a
// diff matches no mutant, which counts as 0. A list it cannot read fails, so that a change in
// cargo-mutants' output never reads as "no mutant".

import fs from "node:fs";

/// What a unified diff changes, by the new file's line numbers: the lines each file gains, and the
/// lines next to each deleted line (`deletions`), where cargo-mutants' `affected_lines` marks the line
/// before and the line after.
export function parseDiff(diff) {
  const added = new Map();
  const deletions = new Map();
  const add = (map, file, line) => {
    if (!map.has(file)) map.set(file, new Set());
    map.get(file).add(line);
  };
  let file = null;
  let hunk = null;
  const closeHunk = () => {
    if (hunk && file && hunk.points.length > 0) {
      for (const p of hunk.points) add(deletions, file, p);
    }
    hunk = null;
  };
  for (const text of diff.split("\n")) {
    if (text.startsWith("diff --git ")) {
      closeHunk();
      file = null;
      continue;
    }
    if (!hunk) {
      const target = /^\+\+\+ (?:b\/(.+)|\/dev\/null)$/.exec(text);
      if (target) {
        file = target[1] ?? null;
        continue;
      }
      if (text.startsWith("--- ")) continue;
    }
    const header = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    if (header) {
      closeHunk();
      hunk = { line: Number(header[1]), inserts: 0, points: [], prevRemoved: false };
      continue;
    }
    if (!hunk || !file) continue;
    const kind = text[0];
    if (kind === "+" || kind === " ") {
      if (hunk.prevRemoved) {
        hunk.points.push(hunk.line);
        hunk.prevRemoved = false;
      }
      if (kind === "+") {
        hunk.inserts++;
        add(added, file, hunk.line);
      }
      hunk.line++;
    } else if (kind === "-") {
      hunk.prevRemoved = true;
      if (hunk.line > 1) hunk.points.push(hunk.line - 1);
    } else if (kind !== "\\") {
      closeHunk();
    }
  }
  closeHunk();
  return { added, deletions };
}

function checkMutant(m, k) {
  const start = m?.span?.start?.line;
  const end = m?.span?.end?.line;
  if (typeof m?.file !== "string" || typeof m?.name !== "string" || !Number.isInteger(start) || !Number.isInteger(end)) {
    throw new Error(`mutant ${k} has no file, name and span this script reads: ${JSON.stringify(m).slice(0, 200)}`);
  }
  // A function named without its span means cargo-mutants' output changed, not a mutant outside
  // any function, which has no function at all.
  const f = m.function;
  if (f !== null && f !== undefined && (!Number.isInteger(f.span?.start?.line) || !Number.isInteger(f.span?.end?.line))) {
    throw new Error(`mutant ${JSON.stringify(m.name)} names a function without its span: cargo-mutants' output changed`);
  }
}

const holds = (span, lines) => [...lines].some((l) => l >= span.start.line && l <= span.end.line);

const inFunction = (m) => Number.isInteger(m.function?.span?.start?.line) && Number.isInteger(m.function?.span?.end?.line);

/// What a deleted line takes in with a mutant: its whole function, by a key for it, or, for a
/// mutant outside any function (a constant's initializer, say), the mutant alone, by its span. None
/// is dropped: each is weighed by one span or the other.
function deletionKey(m) {
  if (!inFunction(m)) return `mutant\n${m.name}`;
  const f = m.function;
  return `${m.file}\n${f.function_name}\n${f.span.start.line}\n${f.span.end.line}`;
}

/// The mutants whose span holds a line the diff adds.
export function touched(mutants, added) {
  if (!Array.isArray(mutants)) throw new Error("the mutants list is not a JSON array");
  return mutants.filter((m, k) => {
    checkMutant(m, k);
    const lines = added.get(m.file.replace(/\\/g, "/"));
    return lines ? holds(m.span, lines) : false;
  });
}

/// The mutants of every function that encloses a deleted line.
export function enclosingDeleted(mutants, deletions) {
  if (!Array.isArray(mutants)) throw new Error("the mutants list is not a JSON array");
  const keys = new Set();
  for (const [k, m] of mutants.entries()) {
    checkMutant(m, k);
    const points = deletions.get(m.file.replace(/\\/g, "/"));
    if (!points) continue;
    if (holds(inFunction(m) ? m.function.span : m.span, points)) keys.add(deletionKey(m));
  }
  return mutants.filter((m) => deletions.has(m.file.replace(/\\/g, "/")) && keys.has(deletionKey(m)));
}

/// How many mutants a pull request's run must test: those whose span holds an added line, and
/// those of a function that encloses a deleted line.
export function expected(mutants, diff) {
  const { added, deletions } = parseDiff(diff);
  const names = new Set([...touched(mutants, added), ...enclosingDeleted(mutants, deletions)].map((m) => m.name));
  return names.size;
}

/// A regex that matches exactly one name, in the syntax of Rust's regex crate.
export function exactRegex(name) {
  return `^${name.replace(/[\\.+*?()|[\]{}^$#&\-~]/g, "\\$&")}$`;
}

/// Each name the base's settings expect that neither of the run's lists holds.
export function missing(baseMutants, diff, inDiff, deleted) {
  const ran = new Set([...inDiff, ...deleted].map((m) => m.name));
  const { added, deletions } = parseDiff(diff);
  const want = new Set([...touched(baseMutants, added), ...enclosingDeleted(baseMutants, deletions)].map((m) => m.name));
  return [...want].filter((n) => !ran.has(n)).sort();
}

/// The mutants of functions enclosing a deleted line that cargo-mutants' list for the diff
/// leaves out, as regexes for `--re`.
export function deletedRegexes(mutants, diff, inDiff) {
  const listed = new Set(inDiff.map((m) => m.name));
  const { deletions } = parseDiff(diff);
  return [...new Set(enclosingDeleted(mutants, deletions).map((m) => m.name))].filter((n) => !listed.has(n)).map(exactRegex);
}

/// The mutants of a `--list --json` output; an empty output is no mutant.
export function readList(text) {
  if (text.trim() === "") return [];
  const list = JSON.parse(text);
  if (!Array.isArray(list)) throw new Error("the mutants list is not a JSON array");
  list.forEach(checkMutant);
  return list;
}

/// The CLI: returns what it prints.
/// What nextest prints for a test that failed, in its output and so in a mutant's log: FAIL, a
/// timeout, a leak that fails, or a signal or an abort.
const FAILED_TEST = /^\s+(FAIL|TIMEOUT|LEAK-FAIL|SIG[A-Z]+|ABORT) \[/m;

/// Why a run's outcomes do not show the work they claim, one message a mutant: a mutant counts as
/// caught only when its tests ran and one failed, nextest exiting 100 with a failing test in the
/// log, and as unviable only when its build failed with an error of the compiler. A stub in place of
/// nextest or cargo, which exits at once, would otherwise make every mutant caught or unviable,
/// and the floor count them all as tested. Times cannot tell: a caught mutant's tests took 0.15 s
/// in the runs of #120. `readLog` reads a log by the path the outcome names.
export function evidence(outcomes, readLog) {
  const found = [];
  const strip = (text) => String(text).replace(/\x1b\[[0-9;]*m/g, "");
  for (const o of outcomes.outcomes ?? []) {
    const name = o.scenario?.Mutant?.name;
    if (!name || !["CaughtMutant", "Unviable"].includes(o.summary)) continue;
    const phase = (p) => (o.phase_results ?? []).find((r) => r.phase === p);
    const log = strip(readLog(o.log_path));
    if (o.summary === "CaughtMutant") {
      const test = phase("Test");
      if (test?.process_status?.Failure !== 100) found.push(`${name}: caught, but its tests did not end as nextest ends a failed run (${JSON.stringify(test?.process_status ?? null)})`);
      else if (!FAILED_TEST.test(log)) found.push(`${name}: caught, but no test failed in its log`);
    } else {
      const build = phase("Build");
      if (!build || build.process_status === "Success" || phase("Test")) found.push(`${name}: unviable, but its build did not fail`);
      else if (!/^error(\[E\d+\])?:/m.test(log)) found.push(`${name}: unviable, but no error of the compiler is in its log`);
    }
  }
  return found;
}

export function main(args) {
  const [command, ...files] = args;
  const read = (f) => fs.readFileSync(f, "utf8");
  if (command === "evidence" && files.length > 0) {
    // Each output directory of the run; one cargo-mutants did not write (no deleted lines) holds
    // nothing to check.
    const found = [];
    for (const dir of files) {
      if (!fs.existsSync(`${dir}/outcomes.json`)) continue;
      found.push(...evidence(JSON.parse(read(`${dir}/outcomes.json`)), (log) => read(`${dir}/${log}`)));
    }
    return found.join("\n");
  }
  if (command === "expected" && files.length === 2) {
    const mutants = readList(read(files[1]));
    if (mutants.length === 0) throw new Error("cargo-mutants lists no mutant in the whole tree");
    return String(expected(mutants, read(files[0])));
  }
  if (command === "deleted" && files.length === 3) {
    const mutants = readList(read(files[1]));
    if (mutants.length === 0) throw new Error("cargo-mutants lists no mutant in the whole tree");
    return deletedRegexes(mutants, read(files[0]), readList(read(files[2]))).join("\n");
  }
  if (command === "missing" && files.length === 4) {
    const base = readList(read(files[1]));
    if (base.length === 0) throw new Error("cargo-mutants lists no mutant in the whole tree");
    return missing(base, read(files[0]), readList(read(files[2])), readList(read(files[3]))).join("\n");
  }
  if (command === "count" && files.length === 1) return String(readList(read(files[0])).length);
  throw new Error("usage: mutants-floor.mjs evidence <mutants.out>... | expected <pr.diff> <base-all.json> | deleted <pr.diff> <all.json> <diff.json> | missing <pr.diff> <base-all.json> <diff.json> <deleted.json> | count <list.json>");
}
