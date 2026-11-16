// The tests a Node run ran, for `mn evidence` (compiler/mente-cli/src/evidence.rs), which checks
// the guards of the escaped bugs against them. A reporter of Node's test runner:
//
//   node --test --test-reporter=spec --test-reporter-destination=stdout \
//     --test-reporter=./tools/evidence-reporter.mjs --test-reporter-destination=<file> <files>
//
// It writes one JSON object a test that ran, through JSON.stringify, so that no name can break the
// format: {"name", "suite", "file", "skipped", "todo"}, `suite` being the names of the suites and
// tests around it, outermost first, and `file` the file that declares it. A test ran when it passed,
// or failed by its own body; one that timed out, or that a hook or its parent cancelled, did not
// run to its end and is not written, nor is a suite, which is no test. The suites around a test are
// found by the test's own parent, `parentId`, which Node gives each event with its `testId` within
// its entry file, never by the file that declares it, since a helper file may declare tests inside
// another file's suite. The format is ours and is read by the one JSON reader of
// compiler/mente-cli/src/json.rs; the file tools/evidence-reporter.jsonl holds what this writes for
// the events of its test, and the Rust reader's test reads the same file.

/// The only failure of a test's own body: Node's `failureType` for an assertion or an error thrown
/// there. Every other one, `testTimeoutFailure`, `cancelledByParent`, `hookFailed`,
/// `subtestsFailed` and the rest, says the body did not run to its end.
const OWN_FAILURE = "testCodeFailure";

/// The reporter: Node hands it the run's events, each file's in the order the tests are defined.
export default async function* evidenceReporter(source) {
  // Each test and suite started, by its entry file and its id there: its name and its parent's id.
  const started = new Map();
  const ids = (data) => {
    if (!Number.isInteger(data.testId) || !Number.isInteger(data.parentId)) {
      throw new Error(`${data.file}: "${data.name}" has no testId or parentId, which this reporter reads its suites from (Node 24.22.1 gives both)`);
    }
    return `${data.entryFile ?? data.file}\n`;
  };
  for await (const event of source) {
    const { type, data } = event;
    if (type === "test:start") {
      started.set(ids(data) + data.testId, { name: data.name, parent: data.parentId });
    } else if ((type === "test:pass" || type === "test:fail") && data.details?.type === "test") {
      const entry = ids(data);
      if (type === "test:fail" && data.details.error?.failureType !== OWN_FAILURE) continue;
      const suite = [];
      for (let id = data.parentId; id !== 0; ) {
        const parent = started.get(entry + id);
        if (parent === undefined) throw new Error(`${data.file}: the parent of "${data.name}" was never started`);
        if (suite.length === started.size) throw new Error(`${data.file}: the parents of "${data.name}" make a loop`);
        suite.unshift(parent.name);
        id = parent.parent;
      }
      yield `${JSON.stringify({
        name: data.name,
        suite,
        file: data.file ?? "",
        skipped: data.skip !== undefined,
        todo: data.todo !== undefined,
      })}\n`;
    }
  }
}
