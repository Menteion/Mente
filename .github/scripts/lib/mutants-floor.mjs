// The floor of work for "Mutants in the diff" (.github/workflows/mutants.yml; X6 of the guard
// audit): when a pull request changes code where cargo-mutants finds mutants, the run must test
// them. The count here is independent of cargo-mutants' own diff filter: it takes every mutant
// cargo-mutants lists for the whole tree, under the BASE's .cargo/mutants.toml so that a pull
// request cannot exclude the function it changes, and keeps those whose span holds a line the diff
// adds. A hunk that only deletes lines adds none, so for it the floor takes every mutant of the
// function enclosing the deletion, which the job then runs too (`deleted`). A diff that changes
// only tests, comments or files outside the compiler holds no such mutant, and the floor is 0.
//
//   node .github/scripts/mutants-floor.mjs expected <pr.diff> <all-mutants.json>
//   node .github/scripts/mutants-floor.mjs deleted <pr.diff> <all-mutants.json> <diff-mutants.json>
//   node .github/scripts/mutants-floor.mjs count <mutants.json>
//
// <all-mutants.json> is `cargo mutants --list --json` over the whole tree. `expected` prints how
// many mutants the diff must have tested. `deleted` prints, one a line, an anchored regex for each
// mutant of a function enclosing a deletion-only hunk that <diff-mutants.json>, cargo-mutants' own
// list for the diff, leaves out, for `cargo mutants --re`. `count` prints how many mutants a list
// holds: cargo-mutants prints nothing at all when a diff matches no mutant, which counts as 0. A
// list it cannot read fails, so that a change in cargo-mutants' output never reads as "no mutant".

import fs from "node:fs";

/// What a unified diff changes, by the new file's line numbers: the lines each file gains, and the
/// lines next to a deletion in a hunk that adds nothing (`deletions`), where cargo-mutants'
/// `affected_lines` marks the line before and the line after.
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
    if (hunk && file && hunk.inserts === 0 && hunk.points.length > 0) {
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
}

const holds = (span, lines) => [...lines].some((l) => l >= span.start.line && l <= span.end.line);

/// The function a mutant sits in, as a key, or null for a mutant outside any function.
function functionKey(m) {
  const f = m.function;
  if (!f?.span?.start?.line) return null;
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

/// The mutants of every function that encloses a deletion-only hunk.
export function enclosingDeleted(mutants, deletions) {
  if (!Array.isArray(mutants)) throw new Error("the mutants list is not a JSON array");
  const keys = new Set();
  for (const [k, m] of mutants.entries()) {
    checkMutant(m, k);
    const points = deletions.get(m.file.replace(/\\/g, "/"));
    const key = functionKey(m);
    if (points && key && holds(m.function.span, points)) keys.add(key);
  }
  return mutants.filter((m) => keys.has(functionKey(m)));
}

/// How many mutants a pull request's run must test: those whose span holds an added line, and
/// those of a function that encloses a deletion-only hunk.
export function expected(mutants, diff) {
  const { added, deletions } = parseDiff(diff);
  const names = new Set([...touched(mutants, added), ...enclosingDeleted(mutants, deletions)].map((m) => m.name));
  return names.size;
}

/// A regex that matches exactly one name, in the syntax of Rust's regex crate.
export function exactRegex(name) {
  return `^${name.replace(/[\\.+*?()|[\]{}^$#&\-~]/g, "\\$&")}$`;
}

/// The mutants of functions enclosing a deletion-only hunk that cargo-mutants' list for the diff
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
export function main(args) {
  const [command, ...files] = args;
  const read = (f) => fs.readFileSync(f, "utf8");
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
  if (command === "count" && files.length === 1) return String(readList(read(files[0])).length);
  throw new Error("usage: mutants-floor.mjs expected <pr.diff> <all.json> | deleted <pr.diff> <all.json> <diff.json> | count <list.json>");
}
