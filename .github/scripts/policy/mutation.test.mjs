// The workflow policy: mutation testing (README.md in this directory). cargo-mutants' settings, the
// sources it reads, and the floor of work of "Mutants in the diff".

import { after, test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { CJK_RANGES, badSubject, baseChangedSincePush, cjkLines, problems, pullRequestOf } from "../lib/commits.mjs";
import { BUILD_SECONDS, MAX_SHARDS, PLAN_REASONS, SANDBOX_PROGRAM, SHARD_MINUTES, deletedRegexes, evidence, exactRegex, expected, missing, nameProblems, parseDiff, planShards, readList, runList, touched } from "../lib/mutants-floor.mjs";
import { read, strings, tracked, workspace } from "./repo.mjs";
import { steps, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 4;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of mutation.test.mjs ran: a skip or a filter left the rest out`);
});
/// The crate whose functions alone start threads (clippy.toml's disallowed-methods), and which
/// alone does not inherit the workspace's lints.
const THREADS_CRATE = "mente-threads";
/// What clippy.toml bans outside it: the core count, every safe way std starts a thread, and the
/// ways a test could hide from its check the variables the check reads.
const THREAD_BANS = [
  "std::thread::available_parallelism",
  "std::thread::Scope::spawn",
  "std::thread::spawn",
  "std::thread::Builder::spawn",
  "std::thread::Builder::spawn_scoped",
  "std::env::set_var",
  "std::env::remove_var",
  "std::process::Command::env_clear",
];

/// .cargo/mutants.toml, as the few forms the policy reads: a bare key at the start of a line, then
/// `true`, `false`, one string, or a list of strings over one line or several, with comments. A
/// string is literal ('...') or holds no escape ("..."). Anything else is an error rather than a
/// line skipped: a quoted or dotted key, a table, an escape, which could each spell a key or a
/// value otherwise to cargo-mutants than to this.
export function mutantsSettings(text, file = ".cargo/mutants.toml") {
  const settings = new Map();
  let k = 0;
  const fail = (why) => {
    throw new Error(`${file}:${text.slice(0, k).split("\n").length}: ${why}`);
  };
  // A carriage return ends a line only before a line feed; alone, TOML refuses it or reads it as a
  // line's end, where this reader would read a space.
  k = text.search(/\r(?!\n)/);
  if (k >= 0) fail("a carriage return that does not end its line");
  k = 0;
  const skip = (lines) => {
    for (;;) {
      const c = text[k];
      if (c === " " || c === "\t" || c === "\r" || (lines && c === "\n")) k++;
      else if (c === "#") while (k < text.length && text[k] !== "\n") k++;
      else return;
    }
  };
  const string = () => {
    const quote = text[k];
    const end = text.indexOf(quote, k + 1);
    const line = text.indexOf("\n", k + 1);
    if (end < 0 || (line >= 0 && line < end)) fail("a string that does not close on its line");
    const body = text.slice(k + 1, end);
    if (quote === '"' && body.includes("\\")) fail("a string with an escape, which could spell anything");
    k = end + 1;
    return body;
  };
  for (;;) {
    skip(true);
    if (k >= text.length) break;
    if (k > 0 && text[k - 1] !== "\n") fail("a key that does not start its line");
    const key = /^[a-z][a-z0-9_]*(?= = )/.exec(text.slice(k, k + 100))?.[0];
    if (!key) fail("not a bare key and its value");
    if (settings.has(key)) fail(`${key} twice`);
    k += key.length + 3;
    let value;
    if (text[k] === "[") {
      k++;
      value = [];
      for (;;) {
        skip(true);
        if (text[k] === "]") {
          k++;
          break;
        }
        if (text[k] !== "'" && text[k] !== '"') fail(`${key} holds something other than strings`);
        value.push(string());
        skip(true);
        if (text[k] === ",") k++;
        else if (text[k] !== "]") fail(`${key}: a list that does not go on with , or end with ]`);
      }
    } else if (text[k] === "'" || text[k] === '"') {
      value = string();
    } else {
      const word = /^(true|false)(?![\w.-])/.exec(text.slice(k, k + 8))?.[1];
      if (!word) fail(`${key} is neither true, false, a string nor a list of strings`);
      value = word === "true";
      k += word.length;
    }
    skip(false);
    if (k < text.length && text[k] !== "\n") fail(`more after the value of ${key}`);
    settings.set(key, value);
  }
  return settings;
}
/// What .cargo/mutants.toml may set, each key with what its value must be. cargo-mutants reads the
/// file of the change in every mutation run, outside the tests' sandbox, and other keys of it hand
/// cargo or nextest whatever arguments they hold (`additional_cargo_args`,
/// `additional_cargo_test_args`): a wrapper around rustc, a runner, or another nextest
/// configuration, whose setup scripts are no test executables and so never reach the sandbox.
/// These four choose the tests and the mutants, and name no command, no argument and no file. To
/// give cargo-mutants another setting, add its key here with its reason, where review sees it.
const MUTANTS_KEYS = new Map([
  // Every mutant runs against the whole workspace's tests.
  ["test_workspace", (v) => v === true],
  // Through nextest, whose limits and whose runner, the sandbox, the policy holds.
  ["test_tool", (v) => v === "nextest"],
  // The files and the mutants left out, each with its reason in the file.
  ["exclude_globs", strings],
  ["exclude_re", strings],
]);

/// Why a .cargo/mutants.toml could take a command of the change past the sandbox, or run the
/// mutants' tests otherwise than through nextest over the whole workspace.
export function mutantsSettingsProblems(text) {
  let settings;
  try {
    settings = mutantsSettings(text);
  } catch (e) {
    return [e.message];
  }
  const found = [];
  for (const [key, value] of settings) {
    const allowed = MUTANTS_KEYS.get(key);
    if (!allowed) found.push(`.cargo/mutants.toml sets ${key}, which MUTANTS_KEYS in .github/scripts/policy/mutation.test.mjs does not hold: cargo-mutants reads this file outside the tests' sandbox, and a key of it can hand cargo or nextest a wrapper, a runner or another configuration`);
    else if (!allowed(value)) found.push(`.cargo/mutants.toml: ${key} = ${JSON.stringify(value)} is not a value the policy holds for it`);
  }
  for (const key of ["test_workspace", "test_tool"]) {
    if (!settings.has(key)) found.push(`.cargo/mutants.toml does not set ${key}, on which the mutation runs' limits and sandbox stand`);
  }
  return found;
}

check("the mutation runs' settings name no command, no argument for cargo and no other file", () => {
  assert.deepEqual(mutantsSettingsProblems(read(".cargo/mutants.toml")), []);
  const base = 'test_workspace = true\ntest_tool = "nextest"\n';
  assert.deepEqual(mutantsSettingsProblems(`# c\n${base}exclude_globs = ["spikes/**"] # why\nexclude_re = [\n  # one\n  'a, [b] # \\(c\\)$',\n  "d",\n]\n`), []);
  assert.deepEqual(mutantsSettings("exclude_re = [\n  'a, [b] # \\(c\\)$', # why\n  \"d\"\n]\n").get("exclude_re"), ["a, [b] # \\(c\\)$", "d"]);
  // The plants of the review of pull request 137, and the other keys and spellings that name a
  // command, an argument or a file.
  for (const plant of [
    'additional_cargo_test_args = ["--config-file", "compiler/mente-cli/tests/nextest-plant.toml"]\n',
    "additional_cargo_args = [\"--config\", \"build.rustc-wrapper='w.sh'\"]\n",
    'additional_cargo_args = ["--config", "target.x86_64-unknown-linux-gnu.runner=\'env\'"]\n',
    '"additional_cargo_args" = ["--all-features"]\n',
    "'additional_cargo_args' = ['--all-features']\n",
    '"\\u0061dditional_cargo_args" = ["--all-features"]\n',
    'output = "/tmp/elsewhere"\n',
    'profile = "plant"\n',
    'features = ["plant"]\n',
    "all_features = true\n",
    "timeout_multiplier = 0.1\n",
    "minimum_test_timeout = 0\n",
    'error_values = ["x"]\n',
    'examine_globs = ["none/**"]\n',
    "skip_calls = ['f']\n",
    "[package]\n",
    "tool.test = true\n",
    'exclude_re = ["a\\\\u0062"]\n',
    'exclude_globs = ["a", 1]\n',
    'exclude_globs = "spikes/**"\n',
    "exclude_globs = ['a'] 'b'\n",
    "  exclude_globs = ['a']\nexclude_globs = ['b']\n",
    // L-4 of the second review of 4e896ca: a carriage return alone, which this reader took for a
    // space and TOML refuses or reads as a line's end.
    "exclude_globs = ['a']\rexclude_re = ['b']\n",
    "exclude_globs = ['a'] # c\rexclude_re = ['b']\n",
  ]) {
    assert.ok(mutantsSettingsProblems(`${base}${plant}`).length > 0, `the settings' reader lets through: ${plant}`);
  }
  for (const text of ['test_workspace = false\ntest_tool = "nextest"\n', 'test_workspace = true\ntest_tool = "cargo"\n', "test_workspace = true\n", 'test_tool = "nextest"\n', `${base}${base}`]) {
    assert.ok(mutantsSettingsProblems(text).length > 0, `the settings' reader lets through: ${text}`);
  }
});

/// The attributes .gitattributes may set: line ends, and the language and documentation GitHub
/// counts a file as. Every other is refused: `linguist-generated`, which folds a file's diff away on
/// GitHub's review page; and above all those that hide a change from a diff, which the Mutants
/// floor reads (H-1 of the second review of 4e896ca: `compiler/** -diff` made git write "Binary
/// files differ", and cargo-mutants read that as a diff with no line): `-diff`, `diff=`, `binary`,
/// a macro, and the filters, merge drivers and export rules that make the tree another than the
/// commit's.
const GIT_ATTRIBUTES = [/^text$/, /^text=auto$/, /^eol=lf$/, /^linguist-language=[A-Za-z][\w+#-]*$/, /^linguist-documentation=true$/];
export function gitattributesProblems(text) {
  const found = [];
  for (const [k, raw] of String(text).split("\n").entries()) {
    const line = raw.replace(/\r$/, "");
    const where = `.gitattributes:${k + 1}`;
    if (line.includes("\r")) found.push(`${where}: a carriage return inside a line`);
    if (line.trim() === "" || line.trim().startsWith("#")) continue;
    if (/^\s/.test(line)) found.push(`${where}: a line that does not start with its pattern`);
    const [pattern, ...attributes] = line.trim().split(/\s+/);
    if (/^["[!]/.test(pattern)) found.push(`${where}: a quoted pattern, a macro or a negation: ${pattern}`);
    if (attributes.length === 0) found.push(`${where}: a pattern with no attribute`);
    for (const a of attributes) if (!GIT_ATTRIBUTES.some((re) => re.test(a))) found.push(`${where}: ${a}, an attribute GIT_ATTRIBUTES in .github/scripts/policy/mutation.test.mjs does not allow; -diff, diff=, binary, filter=, merge= and export-ignore hide a change from a diff or make the tree another than the commit's`);
  }
  return found;
}
/// A Rust source as its tokens read, for a rule over them: each comment, nested block comments
/// too, made one space, and each string, raw string or character literal made `""`. One pass, so
/// that no text makes a rule over it slow, and no `"/*"` in a string hides the code after it.
export function rustTokensText(text) {
  const out = [];
  const n = text.length;
  const raw = /[bc]?r(#*)"/y;
  let k = 0;
  let from = 0;
  const cut = (to, put) => {
    out.push(text.slice(from, k), put);
    k = to;
    from = to;
  };
  while (k < n) {
    const c = text[k];
    raw.lastIndex = k;
    const rawMatch = /[bcr]/.test(c) && !/\w/.test(text[k - 1] ?? "") ? raw.exec(text) : null;
    if (c === "/" && text[k + 1] === "/") {
      const end = text.indexOf("\n", k);
      cut(end < 0 ? n : end, " ");
    } else if (c === "/" && text[k + 1] === "*") {
      let depth = 0;
      let j = k;
      while (j < n) {
        if (text[j] === "/" && text[j + 1] === "*") {
          depth++;
          j += 2;
        } else if (text[j] === "*" && text[j + 1] === "/") {
          depth--;
          j += 2;
          if (depth === 0) break;
        } else j++;
      }
      cut(Math.min(j, n), " ");
    } else if (rawMatch) {
      const close = `"${rawMatch[1]}`;
      const end = text.indexOf(close, k + rawMatch[0].length);
      cut(end < 0 ? n : end + close.length, '""');
    } else if (c === '"') {
      let j = k + 1;
      while (j < n && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      cut(Math.min(j + 1, n), '""');
    } else if (c === "'" && text[k + 1] === "\\") {
      let j = k + 3;
      while (j < n && text[j] !== "'" && text[j] !== "\n") j++;
      cut(Math.min(j + 1, n), '""');
    } else if (c === "'" && k + 1 < n && text[k + 1] !== "\n") {
      const width = text.codePointAt(k + 1) > 0xffff ? 2 : 1;
      if (text[k + 1 + width] === "'") cut(k + 2 + width, '""');
      else k++;
    } else k++;
  }
  out.push(text.slice(from));
  return out.join("");
}
/// Rust's whitespace beyond ASCII (Pattern_White_Space): NEL, the left-to-right and right-to-left
/// marks, and the line and paragraph separators. Rust and the parser cargo-mutants reads with take
/// each for a space between tokens; JavaScript's \s knows only the last two. None may stand in a
/// Rust source or in a file under compiler/ or spikes/.
const RUST_SPACE_BEYOND_ASCII = new RegExp(`[${String.fromCharCode(0x85, 0x200e, 0x200f, 0x2028, 0x2029)}]`);
/// `mutants::skip` as cargo-mutants 27.1.0 reads it, in its tokens: path segments `mutants` and
/// `skip`, a leading `::` or not, alone or under a cfg_attr, with any space, line or comment
/// between them, and raw identifiers. cargo-mutants has no option that disregards it, so the rule
/// is here. It takes a function out of every mutation run, the floor included.
const RUST_SPACE = `\\s${String.fromCharCode(0x85, 0x200e, 0x200f)}`;
const MUTANTS_SKIP = new RegExp(`\\b(?:r#)?mutants[${RUST_SPACE}]*::[${RUST_SPACE}]*(?:r#)?skip\\b`);
const namesMutantsSkip = (text) => MUTANTS_SKIP.test(rustTokensText(text));
/// The attributes of a Rust source, each the text between `#[` or `#![` and its closing bracket, in
/// the source's tokens (so no bracket in a string or a comment counts), in one pass.
export function rustAttributes(text) {
  const code = rustTokensText(text);
  const found = [];
  const open = /#\s*!?\s*\[/g;
  for (let m = open.exec(code); m; m = open.exec(code)) {
    let depth = 0;
    let k = m.index + m[0].length - 1;
    for (; k < code.length; k++) {
      if (code[k] === "[") depth++;
      else if (code[k] === "]" && --depth === 0) break;
    }
    found.push(code.slice(m.index + m[0].length, k));
    open.lastIndex = Math.min(k + 1, code.length);
    if (k >= code.length) break;
  }
  return found;
}
/// A `path = ...` in an attribute, alone or under a cfg_attr: the module it names is read from a
/// file of any name, wherever it lies (cargo-mutants 27.1.0 refuses only a leading `/`), so a
/// function put there escapes every rule that reads the sources by their names.
const PATH_ATTRIBUTE = /(?<![\w#])(?:r#)?path\s*=/;
const readsModuleFromPath = (text) => rustAttributes(text).some((a) => PATH_ATTRIBUTE.test(a));
/// An attribute whose path ends in `test` with segments before it (`rust_analyzer::test`,
/// `miri::test`, `::test`): cargo-mutants 27.1.0 skips any item whose attribute path ends in `test`
/// (`attr_is_test`), as it skips `mutants::skip`, and clippy and rustfmt let some of them through (the
/// fourth review of PR 137: `#[rust_analyzer::test]` took 13 listed mutants to 0). The bare `#[test]`
/// marks a test, which no non-test code can call.
const TEST_SUFFIX = /::\s*(?:r#)?test(?!\w)(?!\s*::)/;
const skipsAsTest = (text) => rustAttributes(text).some((a) => TEST_SUFFIX.test(a));

check("git reads every change as text, and no source names mutants::skip, an attribute that ends in ::test, or a module path of its own", () => {
  assert.deepEqual(gitattributesProblems(read(".gitattributes")), []);
  const nested = [...tracked()].filter((f) => f !== ".gitattributes" && path.posix.basename(f) === ".gitattributes");
  assert.deepEqual(nested, [], "a .gitattributes below the root, which the policy does not read");
  // Every Rust source of the repository, and every file under compiler/ and spikes/ whatever its
  // name: with no #[path], cargo-mutants reads modules only from .rs files under a crate's src/,
  // and a file of another name is read as text here, comments and all.
  const problems = [];
  for (const f of [...tracked()].filter((x) => x.endsWith(".rs") || x.startsWith("compiler/") || x.startsWith("spikes/"))) {
    const text = read(f);
    if (RUST_SPACE_BEYOND_ASCII.test(text)) problems.push(`${f}: a space of Rust's beyond ASCII (U+0085, U+200E, U+200F, U+2028 or U+2029)`);
    if (f.endsWith(".rs") ? namesMutantsSkip(text) : MUTANTS_SKIP.test(text)) problems.push(`${f}: mutants::skip, which takes a function out of every mutation run, the Mutants floor's included`);
    if (f.endsWith(".rs") && readsModuleFromPath(text)) problems.push(`${f}: a #[path] attribute, whose module could be a file of any name the rules here do not read`);
    if (f.endsWith(".rs") && skipsAsTest(text)) problems.push(`${f}: an attribute whose path ends in ::test, which cargo-mutants reads as a test and skips`);
  }
  assert.deepEqual(problems, []);
  // The plants of the third review of 980965e, H1: a module from a .inc file, and the spaces Rust
  // reads that JavaScript's \s does not.
  for (const plant of ['#[path = "plant.inc"] mod plant;', '#[cfg_attr(all(), path = "x.rs")]\nmod plant;', '#[ path="../../docs/x.rs" ]\nmod plant;', "#[r#path = \"x\"] mod plant;", '#[cfg_attr(miri, path = "a.inc")] pub mod plant;']) {
    assert.ok(readsModuleFromPath(`fn f() {}\n${plant}\n`), `the #[path] rule lets through: ${plant}`);
  }
  for (const fine of ["let path = 1;", "fn f(path: &str) {}", '#[doc = "path = x"]\nfn f() {}', '#[serde(rename = "path")]\nstruct S;', "// #[path = \"x.inc\"]\nmod m;", "#[derive(Debug)]\nstruct Path { path: u8 }"]) {
    assert.ok(!readsModuleFromPath(fine), `the #[path] rule refuses: ${fine}`);
  }
  // The fourth review's plants: attributes cargo-mutants reads as a test, and so skips.
  for (const plant of ["#[rust_analyzer::test]", "#[miri::test]", "#[rustfmt::test]", "#[::test]", "#[a::b::test]", "#[tokio :: test]", "#[cfg_attr(miri, rust_analyzer::test)]", "#[rust_analyzer /* c */ :: test]"]) {
    assert.ok(skipsAsTest(`${plant}\npub fn f() -> u8 { 1 }\n`), `the ::test rule lets through: ${plant}`);
  }
  for (const fine of ["#[test]\nfn t() {}", "#[cfg(test)]\nmod tests {}", '#[serde(rename = "a::test")]\nstruct S;', "#[test_case::x]\nfn f() {}", "#[a::testing]\nfn f() {}", "#[a::test::b]\nfn f() {}", "// #[rust_analyzer::test]\nfn f() {}"]) {
    assert.ok(!skipsAsTest(fine), `the ::test rule refuses: ${fine}`);
  }
  // cargo-mutants also skips an `unsafe fn`: no member may write one, since each inherits the
  // workspace's lints and the workspace forbids unsafe code.
  assert.match(read("Cargo.toml"), /^\[workspace\.lints\.rust\]\n(?:#.*\n)*unsafe_code = "forbid"$/m, "the workspace no longer forbids unsafe code, whose functions cargo-mutants skips");
  const lintTables = (text, prefix) => {
    const tables = [];
    let current = null;
    for (const raw of text.split(/\r?\n/)) {
      const line = raw.replace(/\s+#.*$/, "").trim();
      if (line === "" || line.startsWith("#")) continue;
      // An array of tables, `[[bin]]` say, ends a lint table as a table does.
      if (/^\[\[[^\]]+\]\]$/.test(line)) {
        current = null;
        continue;
      }
      const header = /^\[([^\]]+)\]$/.exec(line);
      if (header) {
        current = header[1].startsWith(prefix) ? [header[1].slice(prefix.length), {}] : null;
        if (current) tables.push(current);
        continue;
      }
      const entry = /^([a-z][a-z0-9_-]*) = (.+)$/.exec(line);
      if (current) {
        assert.ok(entry, `a line of [${prefix}${current[0]}] the policy does not read: ${raw}`);
        current[1][entry[1]] = entry[2];
      }
    }
    return tables;
  };
  // Each member's lints, read as tables, not matched as text: a `"""` string could hold a decoy.
  for (const [name, { dir }] of workspace()) {
    const manifest = read(`${dir}/Cargo.toml`);
    assert.ok(!/"""|'''/.test(manifest), `${dir}/Cargo.toml holds a multi-line string, which the policy does not read`);
    if (name === THREADS_CRATE) continue;
    assert.deepEqual(lintTables(manifest, "lints"), [["", { workspace: "true" }]], `${name} does not inherit the workspace's lints, and only them, which forbid unsafe code and clippy's disallowed_methods`);
  }
  // The one crate that may start threads holds the workspace's lints as its own, but for
  // disallowed_methods, which it denies where the workspace forbids it.
  const workspaceLints = lintTables(read("Cargo.toml"), "workspace.lints.");
  const threadsLints = lintTables(read(`compiler/${THREADS_CRATE}/Cargo.toml`), "lints.");
  const expected = workspaceLints.map(([group, keys]) => [group, group === "clippy" ? { ...keys, disallowed_methods: '"deny"' } : keys]);
  assert.equal(Object.fromEntries(workspaceLints).clippy?.disallowed_methods, '"forbid"', "the workspace no longer forbids clippy's disallowed_methods, which keeps threads to mente-threads");
  assert.deepEqual(threadsLints, expected, `${THREADS_CRATE}'s lints are not the workspace's with disallowed_methods denied`);
  // The methods clippy.toml bans outside mente-threads: every way std starts a thread, and the
  // core count. A ban removed, which no #[expect] in mente-threads may notice, fails here.
  const clippy = read("clippy.toml").split(/\r?\n/).filter((l) => !/^\s*#/.test(l)).join("\n");
  const bans = clippy.slice(clippy.indexOf("disallowed-methods = ["), clippy.indexOf("]", clippy.indexOf("disallowed-methods = [") + 22));
  assert.deepEqual([...bans.matchAll(/path = "([^"]+)"/g)].map((m) => m[1]).sort(), [...THREAD_BANS].sort(), "clippy.toml's disallowed-methods are not THREAD_BANS");
  for (const space of [0x85, 0x200e, 0x200f, 0x2028, 0x2029].map((c) => String.fromCharCode(c))) {
    assert.ok(RUST_SPACE_BEYOND_ASCII.test(`#[cfg_attr(miri, mutants${space}::skip)]`));
    assert.ok(namesMutantsSkip(`#[rustfmt::skip]\n#[cfg_attr(miri, mutants${space}::${space}skip)] fn f() {}`), `the mutants::skip rule lets through U+${space.codePointAt(0).toString(16)}`);
  }
  // The plants of the second review of 4e896ca, H-1, and the other spellings.
  for (const plant of ["compiler/** -diff", "*.rs binary", "*.rs linguist-generated=true", "*.rs linguist-generated", "*.rs linguist-vendored", "*.rs diff=plain", "*.rs -text -diff", "*.rs !diff", "*.rs filter=lfs", "*.rs merge=ours", "* export-ignore", "[attr]hide -diff", "*.rs hide", '"compiler/a b.rs" text', "*.rs working-tree-encoding=UTF-16", "*.rs text eol=crlf", "  *.rs -diff", "*.rs", "*.rs text\r-diff"]) {
    assert.ok(gitattributesProblems(`* text=auto eol=lf\n${plant}\n`).length > 0, `the .gitattributes rule lets through: ${JSON.stringify(plant)}`);
  }
  assert.deepEqual(gitattributesProblems("# c\n\n* text=auto eol=lf\r\n*.mn text eol=lf linguist-language=Rust\nspec/*.md linguist-documentation=true\n"), []);
  for (const plant of [
    "#[mutants::skip]",
    "#[mutants :: skip]",
    "#[cfg_attr(test, mutants::skip)]",
    "#[mutants/* c */::/* d */skip]",
    "#[mutants/* a /* nested */ b */::skip]",
    "#[mutants // c\n::skip]",
    "#[::mutants::skip]",
    "#[r#mutants::r#skip]",
    // A string that opens a comment, or closes one, hides nothing after it.
    'const A: &str = "/*";\n#[mutants::skip]\nconst B: &str = "*/";',
    "const A: char = '\"';\n#[mutants::skip]\nconst B: &str = \"\";",
    'const A: &str = r#"/*"#;\n#[mutants::skip]',
    "fn f<'a>(x: &'a str) {}\n#[mutants::skip]",
  ]) {
    assert.ok(namesMutantsSkip(`fn f() {}\n${plant}\nfn g() {}\n`), `the mutants::skip rule lets through: ${plant}`);
  }
  for (const fine of ["mir_mutants::command(&root)", "mutants_skip", "mutants::skipped_count", "// #[mutants::skip]", "/* #[mutants::skip] */", 'const S: &str = "#[mutants::skip]";', 'const S: &str = r##"#[mutants::skip]"##;']) {
    assert.ok(!namesMutantsSkip(fine), `the mutants::skip rule refuses: ${fine}`);
  }
  // In time linear in the text, whatever it holds (CodeQL found the first pattern backtracking on
  // many repetitions of "*//*").
  const started = Date.now();
  for (const bulk of [`mutants${"*//*".repeat(50000)}`, `mutants ${"/* ".repeat(50000)}`, `${"mutants ".repeat(50000)}::`, `"${"\\\\".repeat(50000)}`, "'".repeat(100000)]) namesMutantsSkip(bulk);
  for (const bulk of ["#[".repeat(100000), `#[${"[".repeat(100000)}`, `${"#[a] ".repeat(50000)}`]) readsModuleFromPath(bulk);
  assert.ok(Date.now() - started < 2000, `the mutants::skip and #[path] rules took ${Date.now() - started} ms over 1 MB of text`);
});

// ------------------------------------------------------------------------------ floors of work

check("the Mutants floor counts what the diff changes, as cargo-mutants marks it", () => {
  // H-1 of the second review of 4e896ca: a file git writes as binary is refused, not read as a
  // file with no changed line, and the job reads the diff as text whatever .gitattributes says.
  for (const binary of ["Binary files a/compiler/a/src/lib.rs and b/compiler/a/src/lib.rs differ", "GIT binary patch"]) {
    assert.throws(() => parseDiff(`diff --git a/compiler/a/src/lib.rs b/compiler/a/src/lib.rs\nindex 1..2 100644\n${binary}\n`), /git wrote as binary/, binary);
  }
  // The third review of 980965e: a path git quotes is refused, and one with a space keyed without
  // the tab git ends it with.
  assert.throws(() => parseDiff('diff --git "a/compiler/a/src/\\303\\251.rs" "b/compiler/a/src/\\303\\251.rs"\n--- "a/compiler/a/src/\\303\\251.rs"\n+++ "b/compiler/a/src/\\303\\251.rs"\n@@ -1,1 +1,2 @@\n x\n+y\n'), /git quoted/);
  assert.deepEqual([...parseDiff("diff --git a/compiler/a/src/a b.rs b/compiler/a/src/a b.rs\n--- a/compiler/a/src/a b.rs\t\n+++ b/compiler/a/src/a b.rs\t\n@@ -1,1 +1,2 @@\n x\n+y\n").added.keys()], ["compiler/a/src/a b.rs"]);
  const mutantsDoc = workflows().find((w) => w.file === "mutants.yml").doc;
  const mutantsJob = ["plan", "shard"].flatMap((id) => steps(mutantsDoc.jobs[id])).map((s) => String(s.run ?? "")).join("\n");
  // With quotePath off, git quotes only a path holding a quote, a backslash or a control character,
  // which the floor refuses; a path in any other script stays as it is (the fourth review).
  assert.match(mutantsJob, /^\s*git -c core\.quotePath=false diff --text --no-ext-diff --no-textconv --no-renames "origin\/\$\{GITHUB_BASE_REF\}\.\.\.HEAD" > pr\.diff$/m, "the Mutants job reads the diff in a form .gitattributes or the path's characters can change");
  assert.doesNotMatch(mutantsJob.replace(/git (?:-c core\.quotePath=false )?(diff|show) --(text --no-ext-diff --)?no-textconv /g, ""), /\bgit (?:-c \S+ )*(diff|show|log -p)\b/, "the Mutants job reads a diff or a file without --no-textconv");
  const diff = [
    "diff --git a/compiler/a/src/lib.rs b/compiler/a/src/lib.rs",
    "index 1..2 100644",
    "--- a/compiler/a/src/lib.rs",
    "+++ b/compiler/a/src/lib.rs",
    "@@ -10,3 +10,4 @@ fn f() {",
    " context 10",
    "-removed",
    "+added 11",
    "+added 12",
    " context 13",
    "@@ -40,3 +41,2 @@ fn g() {",
    " context 41",
    "-only removed",
    " context 42",
    "diff --git a/gone.rs b/gone.rs",
    "--- a/gone.rs",
    "+++ /dev/null",
    "@@ -1 +0,0 @@",
    "-x",
    "",
  ].join("\n");
  const { added, deletions } = parseDiff(diff);
  assert.deepEqual([...added.keys()], ["compiler/a/src/lib.rs"]);
  assert.deepEqual([...added.get("compiler/a/src/lib.rs")], [11, 12]);
  assert.deepEqual([...deletions.get("compiler/a/src/lib.rs")], [10, 11, 41, 42], "every hunk that deletes a line, the mixed one too");
  const fnf = { function_name: "f", span: { start: { line: 5 }, end: { line: 20 } } };
  const fng = { function_name: "g", span: { start: { line: 38 }, end: { line: 50 } } };
  const mutant = (name, start, end, fn, file = "compiler/a/src/lib.rs") => ({ name, file, function: fn, span: { start: { line: start, column: 1 }, end: { line: end, column: 9 } } });
  const mutants = [
    mutant("f body", 5, 20, fnf),
    mutant("f at 12", 12, 12, fnf),
    mutant("f at 15", 15, 15, fnf),
    mutant("g at 39", 39, 39, fng),
    mutant("g at 48", 48, 48, fng),
    mutant("elsewhere", 1, 99, null, "compiler/b/src/lib.rs"),
  ];
  assert.deepEqual(touched(mutants, added).map((m) => m.name), ["f body", "f at 12"]);
  assert.equal(expected(mutants, diff), 5, "every mutant of f and g, each of which loses a line");
  assert.deepEqual(deletedRegexes(mutants, diff, [mutants[0], mutants[1], mutants[3]]), ["^f at 15$", "^g at 48$"]);
  // Every expected name must be in the run's lists. A pull request that changes f and g and
  // excludes f in its own .cargo/mutants.toml lists none of f's, under the base's settings
  // expected all the same, and fails.
  const named = (...names) => mutants.filter((m) => names.includes(m.name));
  assert.deepEqual(missing(mutants, diff, named("f body", "f at 12", "g at 39"), named("f at 15", "g at 48")), []);
  assert.deepEqual(missing(mutants, diff, named("g at 39"), named("g at 48")), ["f at 12", "f at 15", "f body"]);
  assert.deepEqual(missing(mutants, diff, named("f body", "f at 12", "f at 15", "g at 39"), []), ["g at 48"], "one left out is enough");
  assert.equal(exactRegex("a.rs:1:2: replace <impl X for Y>::f -> bool with true (x|y)"), "^a\\.rs:1:2: replace <impl X for Y>::f \\-> bool with true \\(x\\|y\\)$");
  assert.throws(() => touched([{ file: "x", span: {} }], added), /no file, name and span/);
  // A mutant outside any function, in a file that loses lines, is weighed by its own span: taken
  // when a deleted line touches it, as main.rs's constant `1 << ...` was on this pull request, and
  // left when none does. One in a file the diff leaves alone ("elsewhere") does not matter.
  const loose = [...mutants, mutant("const at 41", 41, 41, null), mutant("const at 30", 30, 30, null)];
  assert.equal(expected(loose, diff), 6, "g's and f's five, and the constant a deleted line touches");
  assert.deepEqual(missing(loose, diff, loose.filter((m) => m.name !== "const at 41"), []), ["const at 41"]);
  assert.deepEqual(readList(""), []);
  assert.throws(() => readList("{}"), /not a JSON array/);
  const spanless = { ...mutant("f at 12", 12, 12, fnf), function: { function_name: "f" } };
  assert.throws(() => readList(JSON.stringify([spanless])), /names a function without its span/);
  assert.equal(readList(JSON.stringify([mutant("const", 3, 3, null)])).length, 1, "no function at all is a mutant outside any");
  // The evidence of each outcome, as cargo-mutants 27 writes it (the shapes of #120's run): a
  // caught mutant's tests ended as nextest ends a failed run, a test failing in its log; an
  // unviable one's build failed with an error of the compiler. A stub that exits at once shows
  // neither.
  const outcome = (name, summary, phases, log, diff = "", file = "compiler/x/src/lib.rs") => ({ scenario: { Mutant: { name, file } }, summary, log_path: `log/${name}.log`, diff_path: `diff/${name}.diff`, phase_results: phases, log, diff });
  // A mutant's diff as cargo-mutants writes it: line 12 of lib.rs, `0` replaced by
  // `Default::default()`, columns 9 to 26 of the new line.
  const DIFF = "--- compiler/x/src/lib.rs\n+++ replace f -> Foo with Default::default()\n@@ -11,3 +11,3 @@\n fn f() -> Foo {\n-        0\n+        Default::default()\n }\n";
  // An error as cargo forwards rustc's JSON diagnostic into a mutant's build log: its primary span
  // (line, first column, last column exclusive), and the spans of its children, as rustc
  // structures them; a span may come from a macro, whose call site is its expansion.
  const span = (line, from, to, file = "compiler/x/src/lib.rs", expansion = undefined) => ({ file_name: file, line_start: line, column_start: from, line_end: line, column_end: to, is_primary: true, ...(expansion ? { expansion: { span: expansion, macro_decl_name: "assert!" } } : {}) });
  const diagnostic = (code, spans, children = [], level = "error") => `${JSON.stringify({ reason: "compiler-message", message: { message: `a diagnostic ${code}`, code: code ? { code } : null, level, spans, children: children.map((s) => ({ message: "note", level: "note", spans: [s], children: [] })) } })}\n`;
  const aborting = `${JSON.stringify({ reason: "compiler-message", message: { message: "aborting due to 1 previous error", code: null, level: "error", spans: [], children: [] } })}\n`;
  const build = (status) => ({ phase: "Build", duration: 2.8, process_status: status });
  const test = (status) => ({ phase: "Test", duration: 0.15, process_status: status });
  const FAILED = "\x1b[1;31m        FAIL\x1b[0m [   0.004s] mente-syntax tests::x\n";
  // nextest's own line, in each mutant's log, that it hands the test executables to the sandbox's
  // program (the form of the logs of pull requests 139 and 140, coloured as nextest writes it, with
  // the runner of pull request 179).
  const SANDBOXED = "\x1b[1minfo\x1b[0m: using target runner `\x1b[1m/opt/mente-test-sandbox/test-sandbox run\x1b[0m` defined by environment variable `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER`\n";
  const real = [
    outcome("caught", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `*** cargo nextest run\n${SANDBOXED}${FAILED}*** result: Failure(100)\n`),
    outcome("timeout", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `${SANDBOXED}        TIMEOUT [  40.104s] mente-cli::bin/mn x\n`),
    outcome("unviable", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}${aborting}`, DIFF),
    outcome("unviable, inside lines it rewrote whole", "Unviable", [build({ Failure: 101 })], diagnostic("E0308", [span(13, 3, 9)]), "@@ -12,2 +12,3 @@\n-    a\n-    b\n+    c\n+    d\n+    e\n"),
    outcome("unviable, from a macro called in the change", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(62, 9, 73, "/rustc/x/library/core/src/panic.rs", span(12, 9, 27))]), DIFF),
    outcome("unviable, a constant's error with its own note in the change", "Unviable", [build({ Failure: 101 })], diagnostic("E0080", [span(40, 15, 18)], [span(12, 9, 27)]), DIFF),
    outcome("missed", "MissedMutant", [build("Success"), test("Success")], ""),
    { scenario: "Baseline", summary: "Success", phase_results: [], log_path: "log/baseline.log", log: "" },
  ];
  const logs = (list) => (p) => list.find((o) => o.log_path === p)?.log ?? list.find((o) => o.diff_path === p)?.diff ?? "";
  assert.deepEqual(evidence({ outcomes: real }, logs(real)), []);
  const stubbed = [
    outcome("caught by a stub", "CaughtMutant", [build("Success"), test({ Failure: 1 })], "*** cargo nextest run\n*** result: Failure(1)\n"),
    outcome("caught, a failing line but not nextest's exit", "CaughtMutant", [build("Success"), test({ Failure: 1 })], FAILED),
    outcome("caught, no test failed", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `${SANDBOXED}error: no tests to run\n`),
    outcome("caught, by tests that ran outside the sandbox", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `*** cargo nextest run\n${FAILED}*** result: Failure(100)\n`),
    outcome("caught, by tests another runner took", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `info: using target runner \`env\` defined by \`--config\`\n${FAILED}`),
    // A runner named like the sandbox's, by another path, and the shell's wrapper the program
    // replaced: neither is the program setup installed.
    outcome("caught, by a runner of the program's name elsewhere", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `info: using target runner \`/tmp/opt/mente-test-sandbox/test-sandbox run\` defined by x\n${FAILED}`),
    outcome("caught, by the shell's old wrapper", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `info: using target runner \`/usr/bin/bash /opt/mente-test-sandbox/test-sandbox.sh run\` defined by x\n${FAILED}`),
    outcome("caught, the runner's line printed by a test", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `    Starting 3 tests across 1 binary\n${SANDBOXED}${FAILED}`),
    outcome("caught, the runner's line after the failure", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `${FAILED}${SANDBOXED}`),
    outcome("unviable, built", "Unviable", [build("Success")], diagnostic("E0277", [span(12, 9, 27)]), DIFF),
    outcome("unviable, no error", "Unviable", [build({ Failure: 101 })], "*** result: Failure(101)\n", DIFF),
    // An error only as rendered for people, with no JSON diagnostic: the runs pass the flag that
    // forwards them, and a log without one shows no error the reading can place.
    outcome("unviable, only a rendering", "Unviable", [build({ Failure: 101 })], "error[E0277]: the trait bound `Foo: Default` is not satisfied\n  --> compiler/x/src/lib.rs:12:9\n", DIFF),
    // The reviews of 41a54b6 and da58409: an error of the change's own, a constant assertion over the
    // mutated file's text, fails every mutant of it; and errors beside the change, on its line, in
    // another file, after one inside, with no location, or with no diff to place it.
    outcome("unviable, by a const assert elsewhere", "Unviable", [build({ Failure: 101 })], diagnostic("E0080", [span(62, 9, 73, "/rustc/x/library/core/src/panic.rs", span(40, 15, 22))]), DIFF),
    outcome("unviable, beside the change on its line", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(12, 30, 34)]), DIFF),
    outcome("unviable, in another file", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(12, 9, 27, "compiler/x/src/other.rs")]), DIFF),
    outcome("unviable, one error inside and one outside", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}${diagnostic("E0080", [span(40, 15, 22)])}`, DIFF),
    outcome("unviable, an error with no location", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}${diagnostic(null, [])}`, DIFF),
    outcome("unviable, an error the reading cannot parse", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}{"reason":"compiler-message","message":{\n`, DIFF),
    outcome("unviable, no diff", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(12, 9, 27)])),
    outcome("unviable, a note outside the change", "Unviable", [build({ Failure: 101 })], diagnostic("E0080", [span(40, 15, 18)], [span(30, 9, 27)]), DIFF),
    // The review of 47f423d: an error outside the change, after another line's text on its line, as
    // a log interleaves a build's two streams; it counts, as an error no change can claim.
    outcome("unviable, an error after another line's text", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}   Compiling${diagnostic("E0080", [span(40, 15, 22)])}`, DIFF),
    // A note of its own, not the error's, in the change: the review of 75e5eeb.
    outcome("unviable, a separate note in the change", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0080", [span(3, 64, 70)])}${diagnostic(null, [span(12, 9, 27)], [], "note")}`, DIFF),
  ];
  assert.deepEqual(evidence({ outcomes: stubbed }, logs(stubbed)).map((p) => p.split(":")[0]), stubbed.map((o) => o.scenario.Mutant.name));
  // And the review of 75e5eeb's reproductions, from rustc 1.97.1's own diagnostics over the mutated
  // files (.github/scripts/fixtures/): a long merged line whose assertion is the change's, refused;
  // `x -= y` indented 32 spaces, the mutant's own, accepted; and a constant that fails once the
  // mutant's marker is in the file, whose separate note lies at the mutant's line, refused. And the
  // review of 47f423d's: the change's own assertion inside a macro called around the change,
  // refused; and an error on the mutant's own tokens through three macros, accepted.
  // And end to end, on the tool's own output: one real cargo-mutants 27.1.0 run over a tiny crate,
  // its outcomes, logs and diffs as it wrote them. One mutant fails to build by its own error and
  // counts; four fail by the file's own assertion, which fails once the marker is in it, and are
  // refused (the review of 47f423d).
  const e2e = ".github/scripts/fixtures/cargo-mutants-27.1.0";
  const ran = JSON.parse(read(`${e2e}/crate.json`));
  const refused = evidence(JSON.parse(read(`${e2e}/outcomes.json`)), (p) => read(`${e2e}/${p}`)).map((p) => p.slice(0, p.indexOf(": unviable")));
  assert.deepEqual(refused.sort(), [...ran.expect.refused].sort(), "the floor reads a real cargo-mutants run otherwise than its fixture expects");
  const fixtures = JSON.parse(read(".github/scripts/fixtures/unviable-rustc-1.97.1.json"));
  assert.deepEqual(Object.keys(fixtures.cases), ["merged-assertion", "operator-indented", "erroneous-constant", "macro-check", "macro-three-levels"]);
  for (const [id, c] of Object.entries(fixtures.cases)) {
    const one = [outcome(id, "Unviable", [build({ Failure: 101 })], `*** cargo nextest run --no-run\n${c.log}*** result: Failure(101)\n`, c.diff, c.file)];
    assert.equal(evidence({ outcomes: one }, logs(one)).length, c.expect === "refused" ? 1 : 0, `${id}, from real rustc output, is not ${c.expect}: ${c.why}`);
  }
  assert.deepEqual(evidence({ outcomes: stubbed }, logs(stubbed)).map((p) => p.split(":")[0]), stubbed.map((o) => o.scenario.Mutant.name));
});

check("the plan deals every mutant of the run to one shard, as many shards as the baseline's time needs, and each shard runs its share", () => {
  const m = (name) => ({ name, file: "compiler/a/src/lib.rs", function: null, span: { start: { line: 1, column: 1 }, end: { line: 1, column: 2 } } });
  // The run: the diff's list, then each mutant around a deleted line it does not hold.
  assert.deepEqual(runList([m("a"), m("b")], [m("b"), m("c"), m("c")]).map((x) => x.name), ["a", "b", "c", "c"]);
  // Nothing to mutate.
  assert.deepEqual(planShards([], 186), { reason: "no-mutants", needed: 0, shards: [] });
  // As many shards as the estimate needs: each mutant the baseline's tests and BUILD_SECONDS.
  const fit = Math.floor((SHARD_MINUTES * 60) / (100 + BUILD_SECONDS));
  const names = (n) => Array.from({ length: n }, (_, k) => m(`m${k}`));
  assert.equal(planShards(names(fit), 100).needed, 1, "a shard's worth of mutants takes one shard");
  assert.equal(planShards(names(fit + 1), 100).needed, 2, "one more takes a second");
  assert.equal(planShards(names(1), 100).needed, 1);
  // Dealt round-robin in the list's order, all of a name to one shard: a slice by file put the
  // slowest files in one shard.
  const dealt = planShards(names(10), Math.ceil((SHARD_MINUTES * 60 * 3) / 10) - BUILD_SECONDS);
  assert.equal(dealt.reason, "sharded");
  assert.deepEqual(dealt.shards.map((s) => s.map((x) => x.name)), [["m0", "m3", "m6", "m9"], ["m1", "m4", "m7"], ["m2", "m5", "m8"]]);
  const twins = planShards([m("x"), m("y"), m("x"), m("z")], Math.ceil((SHARD_MINUTES * 60 * 2) / 4) - BUILD_SECONDS);
  assert.deepEqual(twins.shards.map((s) => s.map((x) => x.name)), [["x", "x", "z"], ["y"]], "two mutants of one name in two shards, each listing both by its regex");
  // More shards than names would leave one empty, since one name's mutants alone outlast a
  // shard's time: refused (the review of 560ccd9).
  assert.deepEqual(planShards([m("x"), m("x"), m("x")], 4000), { reason: "too-large", needed: 2, shards: [] }, "three mutants of one name over two shards");
  assert.deepEqual(planShards([m("a")], SHARD_MINUTES * 60), { reason: "too-large", needed: 2, shards: [] }, "one mutant longer than a shard's time");
  // More than MAX_SHARDS refuse the run, with the shards it would need; at MAX_SHARDS it runs.
  const most = Math.floor((MAX_SHARDS * SHARD_MINUTES * 60) / (100 + BUILD_SECONDS));
  assert.equal(planShards(names(most), 100).needed, MAX_SHARDS);
  assert.deepEqual(planShards(names(most + 1), 100), { reason: "too-large", needed: MAX_SHARDS + 1, shards: [] });
  for (const seconds of [0, -1, 1.5, Number.NaN, "100"]) assert.throws(() => planShards(names(3), seconds), /whole number of seconds/, String(seconds));
  assert.deepEqual(PLAN_REASONS, ["sharded", "no-mutants", "too-large"], "the reasons the aggregate's script knows");
  // A shard's names against those it ran, each counted as often as it stands.
  assert.deepEqual(nameProblems(["a", "b", "b"], ["b", "a", "b"]), []);
  assert.deepEqual(nameProblems(["a", "b"], ["a"]), ["b: dealt 1 more time(s) than it ran"]);
  assert.deepEqual(nameProblems(["a"], ["a", "c"]), ["c: ran 1 more time(s) than it was dealt"]);
  assert.deepEqual(nameProblems(["a", "a"], ["a"]), ["a: dealt 1 more time(s) than it ran"], "a twin dealt twice and run once");
  // The workflow keeps to the plan: no run of cargo-mutants slices the list itself, the shards'
  // time limit holds twice the plan's time for each, and each shard reads the files the plan wrote.
  const doc = workflows().find((w) => w.file === "mutants.yml").doc;
  const scripts = Object.values(doc.jobs).flatMap((j) => steps(j)).map((s) => String(s.run ?? "")).join("\n");
  assert.doesNotMatch(scripts, /--shard\b/, "a run of cargo-mutants in mutants.yml takes --shard, beside the plan's own dealing");
  assert.ok(doc.jobs.shard["timeout-minutes"] >= 2 * SHARD_MINUTES, "a shard's time limit is less than twice the plan's time for it");
  assert.ok(doc.jobs.shard.strategy?.matrix?.shard === "${{ fromJSON(needs.plan.outputs.shards) }}", "the shards are not those the plan made");
  const planStep = steps(doc.jobs.plan).find((s) => s.id === "plan");
  assert.match(String(planStep?.run), /mutants-floor\.mjs plan diff-mutants\.json deleted-mutants\.json "\$\(cat baseline-seconds\.txt\)" plan >> "\$GITHUB_OUTPUT"\n/);
  const upload = steps(doc.jobs.plan).find((s) => String(s.uses ?? "").startsWith("actions/upload-artifact@"));
  const download = steps(doc.jobs.shard).find((s) => String(s.uses ?? "").startsWith("actions/download-artifact@"));
  assert.equal(upload?.with?.name, download?.with?.name, "the shards download another artifact than the plan keeps");
  assert.ok(String(upload.with.path).split("\n").includes("plan") && download.with.path === "mutants-plan", "the shards read the plan's files elsewhere than it keeps them");
  const shardScripts = steps(doc.jobs.shard).map((s) => String(s.run ?? "")).join("\n");
  assert.equal(shardScripts.split('< "mutants-plan/plan/shard-$SHARD.re"').length, 3, "the shard's list and its run do not both read its regexes");
});
