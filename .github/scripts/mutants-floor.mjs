// The floor of work for "Mutants in the diff" (.github/workflows/mutants.yml; X6 of the guard
// audit): when a pull request changes a line where cargo-mutants finds a mutant, the run must test
// at least one. The count here is independent of cargo-mutants' own filter: it takes every mutant
// cargo-mutants lists for the whole tree, without the diff, and keeps those whose span holds a line
// the diff adds. A diff that changes only tests, comments or files outside the compiler holds no
// such line, and the floor is then 0.
//
//   node .github/scripts/mutants-floor.mjs touched <pr.diff> <all-mutants.json>
//   node .github/scripts/mutants-floor.mjs count <mutants.json>
//
// <all-mutants.json> is `cargo mutants --list --json` over the whole tree; `touched` prints how
// many of its mutants the diff's added lines hold. `count` prints how many mutants a list holds:
// cargo-mutants prints nothing at all when a diff changes no Rust file, which counts as 0. A list
// it cannot read fails, so that a change in cargo-mutants' output never reads as "no mutant".

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/// The lines each file gains in a unified diff, by the new file's line numbers.
export function addedLines(diff) {
  const added = new Map();
  let file = null;
  let line = 0;
  for (const text of diff.split("\n")) {
    if (text.startsWith("diff --git ")) {
      file = null;
      continue;
    }
    const target = /^\+\+\+ (?:b\/(.+)|\/dev\/null)$/.exec(text);
    if (target) {
      file = target[1] ?? null;
      continue;
    }
    if (text.startsWith("--- ")) continue;
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    if (hunk) {
      line = Number(hunk[1]);
      continue;
    }
    if (!file || line === 0) continue;
    if (text.startsWith("+")) {
      if (!added.has(file)) added.set(file, new Set());
      added.get(file).add(line);
      line++;
    } else if (text.startsWith(" ")) {
      line++;
    }
  }
  return added;
}

/// The mutants whose span holds a line the diff adds.
export function touched(mutants, added) {
  if (!Array.isArray(mutants)) throw new Error("the mutants list is not a JSON array");
  return mutants.filter((m, k) => {
    const start = m?.span?.start?.line;
    const end = m?.span?.end?.line;
    if (typeof m?.file !== "string" || !Number.isInteger(start) || !Number.isInteger(end)) {
      throw new Error(`mutant ${k} has no file and span this script reads: ${JSON.stringify(m).slice(0, 200)}`);
    }
    const lines = added.get(m.file.replace(/\\/g, "/"));
    if (!lines) return false;
    for (const l of lines) if (l >= start && l <= end) return true;
    return false;
  });
}

/// The mutants of a `--list --json` output; an empty output is no mutant.
export function readList(text) {
  if (text.trim() === "") return [];
  const list = JSON.parse(text);
  if (!Array.isArray(list)) throw new Error("the mutants list is not a JSON array");
  return list;
}

function main() {
  const [command, ...files] = process.argv.slice(2);
  if (command === "touched" && files.length === 2) {
    const mutants = readList(fs.readFileSync(files[1], "utf8"));
    if (mutants.length === 0) throw new Error("cargo-mutants lists no mutant in the whole tree");
    console.log(touched(mutants, addedLines(fs.readFileSync(files[0], "utf8"))).length);
  } else if (command === "count" && files.length === 1) {
    console.log(readList(fs.readFileSync(files[0], "utf8")).length);
  } else {
    throw new Error("usage: mutants-floor.mjs touched <pr.diff> <all-mutants.json> | count <mutants.json>");
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
  }
}
