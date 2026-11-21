// The test of tools/evidence-reporter.mjs. It hands the reporter the events Node's test runner
// emits, as plain objects in the order Node 24.22.1 emits them, and holds what it writes equal to
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
const HELPER = "/w/.github/scripts/policy-helpers.mjs";
const SITE = "/w/site/site.test.mjs";

/// The events of the tests of one entry file: each with its id and its parent's within that file,
/// as Node gives them, and the file that declares it, the entry file unless a helper declares it.
const run = (entry) => ({
  start: (id, parent, nesting, name, file = entry) => ({
    type: "test:start",
    data: { entryFile: entry, file, testId: id, parentId: parent, nesting, name },
  }),
  end: (id, parent, nesting, name, kind, extra = {}, file = entry) => {
    const { failed, ...rest } = extra;
    const details = { type: kind, duration_ms: 1, ...(failed ? { error: { failureType: failed } } : {}) };
    return {
      type: failed ? "test:fail" : "test:pass",
      data: { entryFile: entry, file, testId: id, parentId: parent, nesting, name, details, ...rest },
    };
  },
});
const P = run(POLICY);
const S = run(SITE);

/// The events of a run over two files whose ids start alike: tests at the top, skipped ones and
/// ones left to do, with a reason and with an empty one, a name with every character a markup could
/// break on, a suite holding a test, a test a helper file declares and a deeper suite, a test that
/// fails by its own body, one that times out, one its suite's hook cancels, a test with a subtest
/// and one whose subtest fails; one file's events among the other's, completions out of order, and
/// a file's own wrapper, which carries no entry file.
const EVENTS = [
  P.start(1, 0, 0, "a plain check"),
  P.end(1, 0, 0, "a plain check", "test"),
  P.start(2, 0, 0, 'a check with > < & " \' and <!-- a comment -->'),
  S.start(1, 0, 0, "a suite"),
  P.end(2, 0, 0, 'a check with > < & " \' and <!-- a comment -->', "test"),
  P.start(3, 0, 0, "a skipped check"),
  P.end(3, 0, 0, "a skipped check", "test", { skip: "later" }),
  P.start(4, 0, 0, "a check left to do"),
  P.end(4, 0, 0, "a check left to do", "test", { todo: true }),
  P.start(5, 0, 0, "a check skipped with no reason"),
  P.end(5, 0, 0, "a check skipped with no reason", "test", { skip: "" }),
  P.start(6, 0, 0, "a check left to do with no reason"),
  P.end(6, 0, 0, "a check left to do with no reason", "test", { todo: "" }),
  { type: "test:complete", data: { entryFile: POLICY, file: HELPER, testId: 8, parentId: 7, nesting: 1, name: "a check from a helper", details: { type: "test" } } },
  P.start(7, 0, 0, "a policy suite"),
  P.start(8, 7, 1, "a check from a helper", HELPER),
  P.end(8, 7, 1, "a check from a helper", "test", {}, HELPER),
  P.start(9, 7, 1, "a nested check"),
  P.end(9, 7, 1, "a nested check", "test"),
  P.end(7, 0, 0, "a policy suite", "suite"),
  P.start(10, 0, 0, "a check that timed out"),
  P.end(10, 0, 0, "a check that timed out", "test", { failed: "testTimeoutFailure" }),
  P.start(11, 0, 0, "a suite whose hook failed"),
  P.start(12, 11, 1, "a check its hook cancelled"),
  P.end(12, 11, 1, "a check its hook cancelled", "test", { failed: "cancelledByParent" }),
  P.end(11, 0, 0, "a suite whose hook failed", "suite", { failed: "hookFailed" }),
  S.start(2, 1, 1, "a test inside"),
  S.end(2, 1, 1, "a test inside", "test"),
  S.start(3, 1, 1, "an inner suite"),
  S.start(4, 3, 2, "a deeper test"),
  S.end(4, 3, 2, "a deeper test", "test", { failed: "testCodeFailure" }),
  S.end(3, 1, 1, "an inner suite", "suite", { failed: "subtestsFailed" }),
  S.end(1, 0, 0, "a suite", "suite", { failed: "subtestsFailed" }),
  S.start(5, 0, 0, "a parent"),
  S.start(6, 5, 1, "a subtest"),
  S.end(6, 5, 1, "a subtest", "test"),
  S.end(5, 0, 0, "a parent", "test"),
  S.start(7, 0, 0, "a parent whose subtest fails"),
  S.start(8, 7, 1, "a failing subtest"),
  S.end(8, 7, 1, "a failing subtest", "test", { failed: "testCodeFailure" }),
  S.end(7, 0, 0, "a parent whose subtest fails", "test", { failed: "subtestsFailed" }),
  { type: "test:diagnostic", data: { file: SITE, nesting: 0, message: "<!-- not a test -->" } },
  { type: "test:stdout", data: { file: SITE, message: "{\"name\": \"not a test\"}\n" } },
  { type: "test:complete", data: { file: SITE, testId: 1, parentId: 0, nesting: 0, name: "site.test.mjs", details: { type: "test", error: { failureType: "subtestsFailed" } } } },
];

async function* source(events) {
  for (const event of events) yield event;
}

async function written(events) {
  let out = "";
  for await (const chunk of reporter(source(events))) out += chunk;
  return out;
}

test("the evidence reporter writes each test that ran as one JSON line, with its suites, file and skip, as the Rust reader reads it", async () => {
  const out = await written(EVENTS);
  assert.equal(out, fs.readFileSync(path.join(HERE, "evidence-reporter.jsonl"), "utf8"));
  for (const line of out.trimEnd().split("\n")) {
    assert.deepEqual(Object.keys(JSON.parse(line)), ["name", "suite", "file", "skipped", "todo"]);
  }
});

test("the evidence reporter writes nothing for a suite, a diagnostic, a test's own output, or a test that did not run to its end", async () => {
  const ran = (e) => (e.type === "test:pass" || (e.type === "test:fail" && e.data.details.error.failureType === "testCodeFailure")) && e.data.details.type === "test";
  assert.equal(await written(EVENTS.filter((e) => !ran(e))), "");
});

test("without test isolation Node gives no entry file, and the reporter reads each test's suites by its id in the one process", async () => {
  // `node --test --test-isolation=none`, as Node 24.22.1 runs it: every file in one process, ids
  // unique across the run, and no `entryFile` on any event (the review of a136bf0).
  const bare = (event) => {
    delete event.data.entryFile;
    return event;
  };
  const N = run(undefined);
  const events = [
    N.start(1, 0, 0, "a policy suite", POLICY),
    N.start(2, 1, 1, "a check from a helper", HELPER),
    N.end(2, 1, 1, "a check from a helper", "test", {}, HELPER),
    N.end(1, 0, 0, "a policy suite", "suite", {}, POLICY),
    N.start(3, 0, 0, "a site test", SITE),
    N.end(3, 0, 0, "a site test", "test", {}, SITE),
  ].map(bare);
  const line = (name, suite, file) => `${JSON.stringify({ name, suite, file, skipped: false, todo: false })}\n`;
  assert.equal(await written(events), line("a check from a helper", ["a policy suite"], HELPER) + line("a site test", [], SITE));
});

test("the evidence reporter refuses a test whose parent never started, parents that loop, and an event with no ids", async () => {
  for (const [events, why] of [
    [[P.start(2, 1, 1, "an orphan"), P.end(2, 1, 1, "an orphan", "test")], /the parent of "an orphan" was never started/],
    [[S.start(2, 1, 1, "a test of another file's suite"), P.start(1, 0, 0, "a suite"), S.end(2, 1, 1, "a test of another file's suite", "test")], /was never started/],
    [[P.start(1, 1, 0, "its own parent"), P.end(1, 1, 0, "its own parent", "test")], /make a loop/],
    [[{ type: "test:start", data: { file: POLICY, nesting: 0, name: "an old event" } }], /has no testId or parentId/],
    [[{ type: "test:pass", data: { file: POLICY, nesting: 0, name: "an old event", details: { type: "test" } } }], /has no testId or parentId/],
  ]) {
    await assert.rejects(written(events), why);
  }
});
