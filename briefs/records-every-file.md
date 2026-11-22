---
id: records-every-file
status: deferred
trigger: when pressure-report, which also changes records.rs, test-isolation's rewrite of mutants.yml (#202) and the quick checks' report of their tree (#203) have merged
type: chore
awaits: [docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/src/records.rs, compiler/mente-cli/tests/records.rs, tools/quick.mjs, tools/quick.test.mjs, .github/workflows/ci.yml, .github/workflows/mutants.yml, .github/workflows/mutants-full.yml, .github/gate-commands.txt, .github/scripts/policy/pins.mjs, .gitignore, CLAUDE.md, docs/design-review.md, docs/escapes/undeclared-record-directory.md, docs/escapes/undeclared-record-other-markup.md, briefs/goldens-one-per-program.md, briefs/records-every-file.md, briefs/archive/records-every-file.md]
protectedFiles: [spec/**, compiler/mente-cli/tests/gates.rs, compiler/mente-cli/tests/lints.rs]
---

# Hold every document and every file of a kind to the records, from git's list

## Intent

The records' closed world (`records::undeclared`) reads Markdown files alone. A directory of records in another markup, or with no extension, stands undeclared and its names go unread: `docs/adr/0001-use-keys.markdown` or `docs/decisions/0001.txt` (the re-review of #175, low 1).

#175 tried a walk of the disk over every document, and CI showed why that cannot hold. The Mutants job writes `deleted-re.txt` into its checkout before its tests, and the walk read it as a document nothing declares. A walk of the disk cannot tell a tool's output from a record, so the closure reads git's list of the tree's files instead (option (a), agreed with the manager):

- **In CI**, a step writes the list right after the checkout and its toolchain, before any step writes into the tree: `git ls-files -z --cached --others --exclude-standard`, into a file at the checkout's root that `.gitignore` keeps out of its own list.
  - The tests read that file in their sandbox, whose `.git` is empty.
  - A missing or empty list fails, with no fallback. CI is told apart by GitHub's `CI` variable, which the sandbox lets through.
  - The step stands in every job that runs the records' tests: ci.yml's Rust and Coverage jobs, mutants.yml and mutants-full.yml. `.github/gate-commands.txt` pins it in the required ones.
- **Elsewhere**, `tools/quick.mjs` writes the same list into the same file at the start of its run, so that one place runs git and the tests read its output. No test runs git, so no program that a repository's git configuration names (`core.fsmonitor`, say) runs inside a test, the principle `pressure-report` holds. A test run outside quick that finds no list fails, saying to run quick; one that finds an old list reads the files as quick last listed them, keeping those that still exist.

Over that list:
- every document-like file (Markdown in each usual spelling, plain text, reStructuredText, AsciiDoc and Org in any case, or a file with no extension; a dotfile is none) belongs to exactly one declaration;
- every file inside a declared record directory, of any extension, is a record of its kind, by the rule `records::entries` reads the directory with, written once for both.

Code, fixtures of code, benchmark runs and configuration in other formats stay out.

## Negative Scope

- No change to the kinds, the key rule or what a record holds.
- No hook and no change to `.claude/settings.json`.
- `entries` still reads a kind's directory from the disk for the readers of records, `mn escapes` among them.

## Invariants

- [ ] A planted `docs/adr/0001-use-keys.markdown` and `docs/decisions/0001.txt` are each refused by the real declarations.
- [ ] A stray file of any extension in a kind's directory is refused by the list, as `entries` refuses it.
- [ ] In CI, a job that runs the records' tests without the step fails them, saying which step to add.
- [ ] The list holds a known tracked file and nothing `.gitignore` excludes.
- [ ] No test runs git: off CI, the list is the one `tools/quick.mjs` wrote.
- [ ] The `.github` changes go to Norman.
- [ ] `node tools/quick.mjs` passes before the push.
