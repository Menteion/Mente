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
  const [listFile] = args;
  if (!listFile) throw new Error("usage: node .github/scripts/test-list.mjs <nextest list --message-format json>");
  const found = problems(JSON.parse(read(listFile)), read(".config/ignored-tests.txt"), read(".config/nextest.toml"));
  for (const p of found) console.log(`::error::${p}`);
  if (found.length > 0) process.exitCode = 1;
  else console.log("The ignored tests are those .config/ignored-tests.txt lists, and every test given a longer limit runs.");
}
