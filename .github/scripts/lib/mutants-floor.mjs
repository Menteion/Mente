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
//
//   node .github/scripts/mutants-floor.mjs plan <diff.json> <deleted.json> <seconds> <dir>
//   node .github/scripts/mutants-floor.mjs assigned <shard.json> <list.json | mutants.out>
//
// `plan` deals the run's mutants, those of <diff.json> and those of <deleted.json> it does not
// hold, to as many shards as <seconds>, the time the tests took once without a mutant, needs
// (planShards); it writes each shard's names and exact regexes into <dir>, and prints the plan's
// outputs for the workflow. `assigned` prints each name a shard was dealt that a list cargo-mutants
// made for it, or its run's outcomes, hold other than as often, and each they hold that it was
// not dealt.
//
//   node .github/scripts/mutants-floor.mjs evidence <mutants.out>...
//
// `evidence` prints each mutant whose outcome its log does not show: caught without nextest's exit
// for a failed run, a failed test and nextest's own line, before the tests start, that it hands
// them to the sandbox; unviable without a failed build whose every error, read from rustc's JSON
// diagnostics in the log, has a span inside the mutant's own change, the lines and columns its
// diff changed. What it stops, and
// what it does not: it catches a test tool that is missing or broken, and a stub
// left in its place that exits at once. It does not stop code of the change that sets out to
// defeat it, which runs with the job's rights: a stub that prints a failing test and exits as
// nextest does, logs or outcomes rewritten after the run, or this script rewritten before it runs.
// Against that, the mutation runs' tests run in a sandbox, on Linux, where every mutation run is:
// cargo-mutants and nextest write the outcomes and the logs outside it, where no test can
// (.github/scripts/test-sandbox.sh; docs/design-review.md, untrusted-test-isolation).

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
    // A file git wrote as binary has no lines here, so its mutants would count for none: the job
    // reads the diff with --text, and a diff that still holds one is refused, not read as empty.
    if (/^Binary files .* differ$/.test(text) || text === "GIT binary patch") {
      throw new Error(`the diff holds a file git wrote as binary (${text}), whose changed lines the floor cannot count: read it with git diff --text --no-ext-diff --no-textconv`);
    }
    if (!hunk) {
      // A path git quotes, for a character outside plain ASCII or a quote, would be read as no
      // file and its lines counted for none: refused, as a binary file is.
      if (/^(\+\+\+|---) "/.test(text)) throw new Error(`the diff names a file git quoted (${text}), whose changed lines the floor would not count: git quotes a path only for a quote, a backslash or a control character, which no file may hold`);
      const target = /^\+\+\+ (?:b\/(.+)|\/dev\/null)$/.exec(text);
      if (target) {
        // git ends a path that holds a space with a tab.
        file = target[1]?.replace(/\t$/, "") ?? null;
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

// ------------------------------------------------------------------------- the plan of the shards

/// The most shards a run takes (the Manager, 2026-11-17): beyond sixteen, PR 158's slowest shard
/// gained at most six minutes, and each shard costs about seven runner-minutes of setup out of the
/// organisation's twenty concurrent jobs.
export const MAX_SHARDS = 16;
/// The time the plan gives one shard's mutants, in minutes: half the six hours GitHub allows a job,
/// so that a shard's setup and the estimate's error stay well inside the limit (the Manager,
/// 2026-11-22: a shard of PR 191, one of four, was cancelled at six hours with 184 mutants tested).
export const SHARD_MINUTES = 180;
/// The time a mutant's own build may take, beside its tests, in seconds: PR 191's 795 mutants built
/// in 2 s at the median and 7 s at the most, each rebuilding only what its change reaches.
export const BUILD_SECONDS = 30;
/// The reasons a plan gives: shards made, nothing to mutate, or more time than MAX_SHARDS shards of
/// SHARD_MINUTES hold, which fails "Mutants in the diff" with that reason rather than a shard at
/// GitHub's limit.
export const PLAN_REASONS = ["sharded", "no-mutants", "too-large"];

/// The run's mutants: cargo-mutants' list for the diff, then each mutant around a deleted line
/// that list does not hold, in cargo-mutants' order.
export function runList(inDiff, deleted) {
  const listed = new Set(inDiff.map((m) => m.name));
  return [...inDiff, ...deleted.filter((m) => !listed.has(m.name))];
}

/// How two lists of names differ, each name counted as often as it stands, since cargo-mutants does
/// not promise its names unique: one message a name, none when they hold the same.
export function nameProblems(assigned, ran) {
  const count = new Map();
  for (const n of assigned) count.set(n, (count.get(n) ?? 0) + 1);
  for (const n of ran) count.set(n, (count.get(n) ?? 0) - 1);
  return [...count]
    .filter(([, c]) => c !== 0)
    .map(([n, c]) => (c > 0 ? `${n}: dealt ${c} more time(s) than it ran` : `${n}: ran ${-c} more time(s) than it was dealt`))
    .sort();
}

/// The shards of a run, sized from the baseline. `seconds`, the time every test took once without a
/// mutant, built beforehand, with BUILD_SECONDS for its build, stands for each mutant: the time of
/// one that no test catches, which runs every test, as the baseline did, and fails the run; one a
/// test catches stops at that test, and an unviable one at its build. The run takes as many shards
/// as that estimate needs at SHARD_MINUTES each, and none above MAX_SHARDS, nor more than it has
/// names: the plan then refuses it, since one name's mutants alone would outlast a shard's time
/// and a shard would stand empty (the review of 560ccd9). The mutants are dealt round-robin in the list's order, all
/// of one name to one shard, since a shard runs its share by an exact regex of each name: a slice
/// of a list ordered by file put the slowest files in one shard (PR 158's run 43865236938: 226
/// minutes against 40; PR 191's run 44959802577: 353 against 162).
export function planShards(mutants, seconds) {
  if (!Number.isInteger(seconds) || seconds <= 0) throw new Error(`the baseline's time is no whole number of seconds above 0: ${seconds}`);
  if (mutants.length === 0) return { reason: "no-mutants", needed: 0, shards: [] };
  const needed = Math.ceil((mutants.length * (seconds + BUILD_SECONDS)) / (SHARD_MINUTES * 60));
  if (needed > MAX_SHARDS) return { reason: "too-large", needed, shards: [] };
  const byName = new Map();
  for (const m of mutants) byName.set(m.name, [...(byName.get(m.name) ?? []), m]);
  if (needed > byName.size) return { reason: "too-large", needed, shards: [] };
  const shards = Array.from({ length: needed }, () => []);
  [...byName.values()].forEach((group, k) => shards[k % needed].push(...group));
  // Every mutant of the run lies in exactly one shard.
  const problems = nameProblems(
    mutants.map((m) => m.name),
    shards.flat().map((m) => m.name),
  );
  if (problems.length > 0) throw new Error(`the shards do not hold the run's mutants, each once: ${problems.join("; ")}`);
  return { reason: "sharded", needed, shards };
}

/// The names of the mutants a cargo-mutants run tested, from its outcomes.
function testedNames(outcomes) {
  if (!Array.isArray(outcomes?.outcomes)) throw new Error("the outcomes hold no list of outcomes");
  return outcomes.outcomes
    .filter((o) => o.scenario !== "Baseline")
    .map((o) => {
      const name = o.scenario?.Mutant?.name;
      if (typeof name !== "string") throw new Error(`an outcome names no mutant: ${JSON.stringify(o.scenario).slice(0, 200)}`);
      return name;
    });
}

/// The names a shard was dealt, from its file: a JSON array of strings.
function dealtNames(text) {
  const names = JSON.parse(text);
  if (!Array.isArray(names) || names.some((n) => typeof n !== "string")) throw new Error("a shard's file is no JSON array of names");
  return names;
}

/// The CLI: returns what it prints.
/// What nextest prints for a test that failed, in its output and so in a mutant's log: FAIL, a
/// timeout, a leak that fails, or a signal or an abort.
const FAILED_TEST = /^\s+(FAIL|TIMEOUT|LEAK-FAIL|SIG[A-Z]+|ABORT) \[/m;

/// The sandbox's one program, as setup installs it and test-sandbox.sh names it cargo's runner,
/// `test-sandbox run` (.github/scripts/test-sandbox.rs). The policy holds the script, the program
/// and this to the one path.
export const SANDBOX_PROGRAM = "/opt/mente-test-sandbox/test-sandbox";
/// What nextest prints before a run whose test executables it hands to the sandbox
/// (.github/scripts/test-sandbox.sh): its own line, for each mutant, in that mutant's log, naming
/// that program exactly. The count of sandboxed commands at the step's end cannot tell one mutant's
/// tests from another's. A test's own output lands in the same log, so the line counts only where
/// nextest alone writes: before its line that starts the tests, and so before any failed test.
const SANDBOX_RUNNER = new RegExp(`^\\s*info: (?:for the \\w+ platform, )?using target runner \`${SANDBOX_PROGRAM.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")} run\``, "m");
/// nextest's line that starts the tests, after which a test's output may appear.
const TESTS_START = /^\s*Starting \d+ tests? across /m;

/// Each error rustc reported in a log, from the JSON diagnostics cargo forwards there (each run of
/// mutants passes `--cargo-arg=--cargo-message-format=json`, which the policy requires): one
/// `compiler-message` line each, as rustc structures it, never its rendering for people. Each error
/// keeps its primary spans and the spans of its own children, the notes and helps rustc attaches to
/// it; a separate note, such as "erroneous constant encountered", is a message of its own, no
/// error's (the review of 75e5eeb: a rendering cut at the left, shifted by a label, or read across a
/// blank line, put an error where it was not). rustc's summary, "aborting due to ...", is none. A
/// line the reading cannot parse counts as an error no change can claim.
export function compilerErrors(log) {
  const out = [];
  for (const line of String(log).split("\n")) {
    // Every line that holds a compiler message anywhere counts: one that does not start with it, or
    // does not parse whole, is an error no change can claim. cargo-mutants writes a build's stdout
    // and stderr into one log, and a long JSON line can come in two writes, with another line's text
    // before it (the review of 47f423d: `   Compiling{"reason":"compiler-message",...}` went unread).
    if (!/"reason"\s*:\s*"compiler-message"/.test(line)) continue;
    let m;
    try {
      if (!line.startsWith('{"reason":"compiler-message"')) throw new Error("not at the start of its line");
      m = JSON.parse(line).message;
    } catch {
      out.push({ text: "a compiler message the reading cannot parse", primary: [], children: [] });
      continue;
    }
    if (m?.level !== "error") continue;
    const spans = Array.isArray(m.spans) ? m.spans : [];
    if (spans.length === 0 && /^aborting due to/.test(String(m.message))) continue;
    out.push({
      text: `error${m.code?.code ? `[${m.code.code}]` : ""}: ${m.message}`,
      primary: spans.filter((s) => s?.is_primary),
      children: (Array.isArray(m.children) ? m.children : []).flatMap((c) => (Array.isArray(c?.spans) ? c.spans : [])),
    });
  }
  return out;
}

/// The places a span stands for in the source: the span, and the call site of each macro it was
/// expanded from, outward, so that an error inside `assert!` is read where `assert!` is called.
export function spanSites(span) {
  const out = [];
  for (let s = span, k = 0; s && k < 64; s = s.expansion?.span, k++) out.push(s);
  return out;
}

/// A span of rustc's as its first and last places, its columns 1-based and its end exclusive.
const spanEnds = (span) => [
  [span.line_start, span.column_start],
  [span.line_end, Math.max(span.column_end - 1, span.column_start)],
];
const notAfter = (a, b) => a[0] < b[0] || (a[0] === b[0] && a[1] <= b[1]);

/// Whether a span meets a region of changedRegions, its ends included.
export function spanMeets(span, region) {
  const [from, to] = spanEnds(span);
  return notAfter(from, [region.to.line, region.to.column]) && notAfter([region.from.line, region.from.column], to);
}

/// Whether a span lies wholly inside a region of changedRegions.
export function spanWithin(span, region) {
  const [from, to] = spanEnds(span);
  return notAfter([region.from.line, region.from.column], from) && notAfter(to, [region.to.line, region.to.column]);
}

/// What a mutant's own diff changed in the mutated file, one region a hunk's run of changed
/// lines: from the first column where its first added line differs from the first line it
/// replaced, to the last column where its last added line differs from the last line it replaced,
/// each `{ line, column }` in the new file. A run that only adds lines changed all of them; one
/// that only removes lines changed none. The ends are found whatever the run's line counts, so that
/// no unchanged text the mutant joined to a changed line counts as changed (the re-check of
/// da58409: two lines merged into one, the second ending in a constant assertion of the change's).
export function changedRegions(diff) {
  const out = [];
  let line = 0;
  let removed = [];
  let added = [];
  const flush = () => {
    if (added.length > 0) {
      const first = added[0];
      const last = added[added.length - 1];
      if (removed.length === 0) out.push({ from: { line: first.at, column: 1 }, to: { line: last.at, column: last.text.length + 1 } });
      else {
        const a = removed[0];
        const z = removed[removed.length - 1];
        let p = 0;
        while (p < a.length && p < first.text.length && a[p] === first.text[p]) p++;
        // On a single changed line the suffix may not reach back into the prefix.
        const room = added.length === 1 && removed.length === 1 ? Math.min(a.length, last.text.length) - p : added.length === 1 ? last.text.length - p : last.text.length;
        let s = 0;
        while (s < room && s < z.length && z[z.length - 1 - s] === last.text[last.text.length - 1 - s]) s++;
        const toColumn = Math.max(last.text.length - s, 1);
        out.push({ from: { line: first.at, column: p + 1 }, to: { line: last.at, column: added.length === 1 ? Math.max(toColumn, p + 1) : toColumn } });
      }
    }
    removed = [];
    added = [];
  };
  for (const l of String(diff).split("\n")) {
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(l);
    if (hunk) {
      flush();
      line = Number(hunk[1]);
    } else if (line === 0) continue;
    else if (l.startsWith("-")) removed.push(l.slice(1));
    else if (l.startsWith("+")) {
      added.push({ at: line, text: l.slice(1) });
      line++;
    } else {
      flush();
      if (l.startsWith(" ")) line++;
    }
  }
  flush();
  return out;
}

/// Why a run's outcomes do not show the work they claim, one message a mutant: a mutant counts as
/// caught only when its tests ran in the sandbox and one failed, nextest exiting 100 with a failing
/// test in the log after its line that names the sandbox as its runner, and as unviable only when
/// its build failed with errors of the compiler that each lie inside the mutant's own change, the
/// lines and columns its diff changed: an error elsewhere is the change's, not the mutant's (the
/// review of 41a54b6: a `const _: () = assert!(...)` over `include_str!` of the mutated file failed
/// every mutant of it with a real E0080, and the floor counted them all as tested). A stub in place of
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
      else {
        const runner = log.search(SANDBOX_RUNNER);
        const start = log.search(TESTS_START);
        const before = runner >= 0 && runner < log.search(FAILED_TEST) && (start < 0 || runner < start);
        if (!before) found.push(`${name}: caught, but its log does not show nextest handing its tests to the sandbox before they started`);
      }
    } else {
      const build = phase("Build");
      const errors = compilerErrors(log);
      const file = o.scenario.Mutant.file;
      const regions = changedRegions(o.diff_path ? strip(readLog(o.diff_path)) : "");
      // An error is the mutant's when one of its primary spans, or a span of one of its own
      // children, meets the mutant's change, at the span or at a call site of a macro it came from.
      // A span itself meets the change by touching it; the call site of a macro it came from only
      // by lying wholly inside it: a macro called around the change could hold the change's own
      // check, whose error would then reach the change through the call (the review of 47f423d:
      // `checked!(a - b)` with a constant assertion inside `checked!`).
      const sameFile = (x) => String(x.file_name).replace(/\\/g, "/") === file;
      const meets = (s) => spanSites(s).some((x, k) => sameFile(x) && regions.some((r) => (k === 0 ? spanMeets(x, r) : spanWithin(x, r))));
      const inside = (e) => e.primary.some(meets) || e.children.some(meets);
      const outside = errors.find((e) => !inside(e));
      if (!build || build.process_status === "Success" || phase("Test")) found.push(`${name}: unviable, but its build did not fail`);
      else if (errors.length === 0) found.push(`${name}: unviable, but no error of the compiler is in its log, as JSON diagnostics`);
      else if (outside) {
        const at = outside.primary[0] ? spanSites(outside.primary[0]).map((s) => `${s.file_name}:${s.line_start}:${s.column_start}`).join(" <= ") : "no location";
        found.push(`${name}: unviable, but an error of the compiler lies outside the mutant's own change: ${outside.text} at ${at}`);
      }
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
  if (command === "plan" && files.length === 4) {
    const [diffList, deletedList, seconds, dir] = files;
    if (!/^[1-9][0-9]*$/.test(seconds)) throw new Error(`the baseline's time is no whole number of seconds above 0: '${seconds}'`);
    const run = runList(readList(read(diffList)), readList(read(deletedList)));
    const plan = planShards(run, Number(seconds));
    fs.mkdirSync(dir, { recursive: true });
    for (const [k, shard] of plan.shards.entries()) {
      const names = shard.map((m) => m.name);
      fs.writeFileSync(`${dir}/shard-${k}.json`, `${JSON.stringify(names)}\n`);
      fs.writeFileSync(`${dir}/shard-${k}.re`, `${[...new Set(names)].map(exactRegex).join("\n")}\n`);
    }
    // The partition as the shards will read it: the directory holds the shards' files and nothing
    // else, which a shard could read as its own; each shard's names, read once more, hold every
    // mutant of the run in exactly one; and each shard's regexes are those of its names.
    const onDisk = fs.readdirSync(dir).sort();
    const want = plan.shards.flatMap((_, k) => [`shard-${k}.json`, `shard-${k}.re`]).sort();
    if (onDisk.join("\n") !== want.join("\n")) throw new Error(`${dir} holds ${onDisk.join(", ")}, not the files of ${plan.shards.length} shards`);
    const dealt = plan.shards.map((_, k) => dealtNames(read(`${dir}/shard-${k}.json`)));
    const written = nameProblems(
      plan.reason === "sharded" ? run.map((m) => m.name) : [],
      dealt.flat(),
    );
    if (written.length > 0) throw new Error(`the shards written do not hold the run's mutants, each once: ${written.join("; ")}`);
    for (const [k, names] of dealt.entries()) {
      if (names.length === 0) throw new Error(`shard ${k} was dealt no mutant`);
      const regexes = read(`${dir}/shard-${k}.re`).split("\n").filter((l) => l !== "");
      if (regexes.join("\n") !== [...new Set(names)].map(exactRegex).join("\n")) throw new Error(`shard ${k}'s regexes are not those of its names`);
    }
    const count = plan.reason === "sharded" ? plan.needed : 0;
    const said = {
      sharded: `${run.length} mutants at ${seconds} s of tests and ${BUILD_SECONDS} s of build each need ${count} shard(s) of ${SHARD_MINUTES} minutes at most`,
      "no-mutants": "the diff changes no line where a mutant lies",
      "too-large": `${run.length} mutants at ${seconds} s of tests and ${BUILD_SECONDS} s of build each need ${plan.needed} shards of ${SHARD_MINUTES} minutes, ${plan.needed > MAX_SHARDS ? `more than the ${MAX_SHARDS} a run takes` : "more than the run has names, so one name's mutants alone outlast a shard's time"}: split the pull request`,
    }[plan.reason];
    fs.writeFileSync(`${dir}/summary.md`, `Mutants plan: ${said}.\n`);
    return [`reason=${plan.reason}`, `count=${count}`, `shards=${JSON.stringify(plan.shards.map((_, k) => k))}`, `needed=${plan.needed}`].join("\n");
  }
  if (command === "assigned" && files.length === 2) {
    // Against a list cargo-mutants made for the shard, or against the outcomes of its run.
    const ran = fs.statSync(files[1]).isDirectory() ? testedNames(JSON.parse(read(`${files[1]}/outcomes.json`))) : readList(read(files[1])).map((m) => m.name);
    return nameProblems(dealtNames(read(files[0])), ran).join("\n");
  }
  throw new Error(
    "usage: mutants-floor.mjs evidence <mutants.out>... | expected <pr.diff> <base-all.json> | deleted <pr.diff> <all.json> <diff.json> | missing <pr.diff> <base-all.json> <diff.json> <deleted.json> | count <list.json> | plan <diff.json> <deleted.json> <seconds> <dir> | assigned <shard.json> <list.json | mutants.out>",
  );
}
