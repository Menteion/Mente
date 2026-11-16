// The tests a Node run ran, for `mn evidence` (compiler/mente-cli/src/evidence.rs), which checks
// the guards of the escaped bugs against them. A reporter of Node's test runner:
//
//   node --test --test-reporter=spec --test-reporter-destination=stdout \
//     --test-reporter=./tools/evidence-reporter.mjs --test-reporter-destination=<file> <files>
//
// It writes one JSON object a finished test, through JSON.stringify, so that no name can break the
// format: {"name", "suite", "file", "skipped", "todo"}, `suite` being the names of the suites and
// tests around it, outermost first. A suite itself is no test and is not written. The format is
// ours and is read by the one JSON reader of compiler/mente-cli/src/json.rs; the file
// tools/evidence-reporter.jsonl holds what this writes for the events of its test, and the Rust
// reader's test reads the same file.

/// The reporter: Node hands it the run's events, in the order the tests are defined in each file.
export default async function* evidenceReporter(source) {
  // For each file, the name of the test or suite started at each depth, so far.
  const started = new Map();
  for await (const event of source) {
    const { type, data } = event;
    if (type === "test:start") {
      const names = started.get(data.file) ?? [];
      names.length = data.nesting;
      names.push(data.name);
      started.set(data.file, names);
    } else if ((type === "test:pass" || type === "test:fail") && data.details?.type === "test") {
      const names = started.get(data.file) ?? [];
      yield `${JSON.stringify({
        name: data.name,
        suite: names.slice(0, data.nesting),
        file: data.file ?? "",
        skipped: Boolean(data.skip),
        todo: Boolean(data.todo),
      })}\n`;
    }
  }
}
