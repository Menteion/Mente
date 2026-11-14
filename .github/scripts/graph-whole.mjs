// Fails when spec-graph read nothing in a file its patterns name.
//
// spec-graph leaves a file out of the graph, all of it, when another file already has its id, and
// does not read a file over its size limit. Either way nothing in the file is checked, and the
// check still passes: the line that says so is no finding, so `strict` does not raise it and
// `--max-warnings` does not count it. `docs/language-card.md` went unchecked that way, under the
// id of the register's entry `language-card`, until it was given an id of its own
// (docs/design-review.md, guard-per-problem).
//
//   spec-graph check --format json | node .github/scripts/graph-whole.mjs
//
// It reads the JSON report on standard input, and fails too when the report lacks the fields it
// reads, so that a change of the report's shape cannot leave this check passing on nothing.
import { readFileSync } from "node:fs";

const fail = (lines) => {
  for (const line of lines) console.error(line);
  process.exit(1);
};

let report;
try {
  report = JSON.parse(readFileSync(0, "utf8"));
} catch (error) {
  fail([`spec-graph's report on standard input is not JSON: ${error.message}`]);
}

const duplicates = report?.summary?.duplicateIdFiles;
const skipped = report?.skipped;
if (!Number.isInteger(duplicates) || !Array.isArray(skipped) || !Array.isArray(report.problems)) {
  fail(["spec-graph's JSON report has no summary.duplicateIdFiles, no skipped list or no problems list, which this check reads"]);
}

const unread = [
  ...report.problems.filter((p) => /left out of the graph/.test(p.message ?? "")).map((p) => `${p.file}:${p.line}: ${p.message}`),
  ...skipped.map((s) => `${s.path}: not read (${s.reason}, ${s.bytes} bytes)`),
];
if (duplicates > 0 || skipped.length > 0) {
  fail([
    `spec-graph read nothing in ${duplicates + skipped.length} file(s) its patterns name:`,
    ...unread.map((line) => `  ${line}`),
    "Give a file left out for a duplicate id an `id:` of its own in its front matter.",
  ]);
}
console.log("spec-graph read every file its patterns name");
