// The test of tools/evidence-reporter.mjs. It hands the reporter the events Node's test runner
// emits, as plain objects in the order Node emits them, and holds what it writes equal to
// tools/evidence-reporter.jsonl, which the Rust reader's test (compiler/mente-cli/tests/evidence.rs)
// reads too, so that the writer and the reader agree on one format. It starts no process.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import reporter from "./evidence-reporter.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const POLICY = "/w/.github/scripts/policy.test.mjs";
const SITE = "/w/site/site.test.mjs";

const start = (file, nesting, name) => ({ type: "test:start", data: { file, nesting, name } });
const end = (file, nesting, name, kind, extra = {}) => ({
  type: extra.failed ? "test:fail" : "test:pass",
  data: { file, nesting, name, details: { type: kind, duration_ms: 1 }, ...extra },
});

/// The events of a run over two files: tests at the top, a skipped one and one left to do, a name
/// with every character a markup could break on, a suite holding a test and a deeper suite, a test
/// that fails, and a test with a subtest; one file's events among the other's.
const EVENTS = [
  start(POLICY, 0, "a plain check"),
  end(POLICY, 0, "a plain check", "test"),
  start(POLICY, 0, 'a check with > < & " \' and <!-- a comment -->'),
  start(SITE, 0, "a suite"),
  end(POLICY, 0, 'a check with > < & " \' and <!-- a comment -->', "test"),
  start(POLICY, 0, "a skipped check"),
  end(POLICY, 0, "a skipped check", "test", { skip: "later" }),
  start(POLICY, 0, "a check left to do"),
  end(POLICY, 0, "a check left to do", "test", { todo: true }),
  start(POLICY, 0, "a policy suite"),
  start(POLICY, 1, "a nested check"),
  end(POLICY, 1, "a nested check", "test"),
  end(POLICY, 0, "a policy suite", "suite"),
  start(SITE, 1, "a test inside"),
  end(SITE, 1, "a test inside", "test"),
  start(SITE, 1, "an inner suite"),
  start(SITE, 2, "a deeper test"),
  end(SITE, 2, "a deeper test", "test", { failed: true }),
  end(SITE, 1, "an inner suite", "suite"),
  end(SITE, 0, "a suite", "suite"),
  start(SITE, 0, "a parent"),
  start(SITE, 1, "a subtest"),
  end(SITE, 1, "a subtest", "test"),
  end(SITE, 0, "a parent", "test"),
  { type: "test:diagnostic", data: { file: SITE, nesting: 0, message: "<!-- not a test -->" } },
  { type: "test:stdout", data: { file: SITE, message: "{\"name\": \"not a test\"}\n" } },
];

async function* source(events) {
  for (const event of events) yield event;
}

test("the evidence reporter writes each finished test as one JSON line, with its suites, file and skip, as the Rust reader reads it", async () => {
  let written = "";
  for await (const chunk of reporter(source(EVENTS))) written += chunk;
  assert.equal(written, fs.readFileSync(path.join(HERE, "evidence-reporter.jsonl"), "utf8"));
  for (const line of written.trimEnd().split("\n")) {
    assert.deepEqual(Object.keys(JSON.parse(line)), ["name", "suite", "file", "skipped", "todo"]);
  }
});

test("the evidence reporter writes nothing for a suite, a diagnostic or a test's own output", async () => {
  let written = "";
  const finished = (e) => (e.type === "test:pass" || e.type === "test:fail") && e.data.details.type === "test";
  const quiet = EVENTS.filter((e) => !finished(e));
  for await (const chunk of reporter(source(quiet))) written += chunk;
  assert.equal(written, "");
});
