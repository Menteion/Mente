// Checks the tests nextest lists against what the repository says of them: the ignored tests are
// exactly those of .config/ignored-tests.txt, and every test .config/nextest.toml gives its longer
// limit is listed once, in its package, and not ignored. The required Rust jobs run it on every
// runner over `cargo-nextest nextest list --run-ignored all --message-format json`, so that a test
// cannot be ignored, in any spelling libtest reads, or stop being one, or be compiled out on a
// platform while it keeps a longer limit, without a change to a file review sees.

/// The tests of a nextest list, as `<package> <test path>` with whether each is ignored.
export function listedTests(list) {
  const out = [];
  for (const suite of Object.values(list["rust-suites"] ?? {})) {
    for (const [name, test] of Object.entries(suite.testcases ?? {})) {
      out.push({ test: `${suite["package-name"]} ${name}`, ignored: test.ignored === true });
    }
  }
  return out;
}

/// The lines of a list file: `<package> <test path>`, comments and blanks aside.
export function readLines(text) {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
}

/// The tests .config/nextest.toml's overrides name, as `<package> <test path>`.
export function slowTests(nextestToml) {
  return [...nextestToml.matchAll(/\(package\(([a-z][a-z0-9_-]*)\) & test\(=([^)]*)\)\)/g)].map((m) => `${m[1]} ${m[2]}`);
}

/// What differs between the list and the repository, one message a difference.
export function problems(list, ignoredText, nextestToml) {
  const found = [];
  const tests = listedTests(list);
  if (tests.length === 0) found.push("nextest listed no test");
  const ignored = new Set(tests.filter((t) => t.ignored).map((t) => t.test));
  const expected = new Set(readLines(ignoredText));
  for (const t of ignored) if (!expected.has(t)) found.push(`${t} is ignored, and .config/ignored-tests.txt does not list it`);
  for (const t of expected) if (!ignored.has(t)) found.push(`${t} is listed in .config/ignored-tests.txt, and nextest does not list it as ignored`);
  for (const slow of slowTests(nextestToml)) {
    const matches = tests.filter((t) => t.test === slow);
    if (matches.length !== 1) found.push(`${slow}, given a longer limit in .config/nextest.toml, is listed ${matches.length} times, not once`);
    else if (matches[0].ignored) found.push(`${slow}, given a longer limit in .config/nextest.toml, is ignored`);
  }
  return found;
}

export function main(args, read) {
  const [command, listFile] = args;
  let found;
  if (command === "nextest" && listFile) {
    found = problems(JSON.parse(read(listFile)), read(".config/ignored-tests.txt"), read(".config/nextest.toml"));
    if (found.length === 0) console.log("The ignored tests are those .config/ignored-tests.txt lists, and every test given a longer limit runs.");
  } else if (command === "doctests" && listFile) {
    found = doctestProblems(read(listFile));
    if (found.length === 0) console.log("No library holds a doc example rustdoc would compile.");
  } else {
    throw new Error("usage: node .github/scripts/test-list.mjs nextest <nextest list --message-format json> | doctests <cargo test --doc -- --list>");
  }
  for (const p of found) console.log(`::error::${p}`);
  if (found.length > 0) process.exitCode = 1;
}

/// Why the doctests' list is not empty, from `cargo test --doc -- --list`: each library's header,
/// `Doc-tests <crate>`, is followed by its count, which must be `0 tests, 0 benchmarks`. nextest
/// runs no doctests, and `cargo test --doc` gives none a limit of its own, so a library holds no
/// doc example rustdoc would compile, in whatever spelling of a doc comment or attribute; such an
/// example belongs in a test. A list with no library, or a header without its count, fails too.
export function doctestProblems(text) {
  const lines = String(text).split(/\r?\n/).map((l) => l.trim());
  const found = [];
  let libraries = 0;
  for (const [k, line] of lines.entries()) {
    const header = /^Doc-tests (\S+)$/.exec(line);
    if (!header) continue;
    libraries++;
    const count = lines.slice(k + 1).find((l) => /^\d+ tests?, \d+ benchmarks?$/.test(l) || /^Doc-tests /.test(l));
    if (!count || count.startsWith("Doc-tests ")) found.push(`${header[1]}: rustdoc listed no count of its doctests`);
    else if (count !== "0 tests, 0 benchmarks") found.push(`${header[1]}: rustdoc lists doctests (${count}): make each a test, or mark its block \`text\` or \`ignore\``);
  }
  if (libraries === 0) found.push("rustdoc listed the doctests of no library");
  return found;
}
