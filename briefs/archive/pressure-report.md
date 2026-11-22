---
id: pressure-report
status: archived
type: feature
awaits: [docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/src/pressure.rs, compiler/mente-cli/tests/pressure.rs, compiler/mente-cli/src/lib.rs, compiler/mente-cli/src/main.rs, compiler/mente-cli/src/records.rs, briefs/pressure-report.md, briefs/archive/pressure-report.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**]
integrity: sha256-91754e949f7c36f9bdd70534a73eed840f367406dce7b228ad9b6321836a14e4
---

<!-- spec-brief:banner -->
> **Archived 2026-11-21.**
> mn pressure ranks the files by how many merged pull requests changed them, each with its place among the records, and refuses a log that is empty, shallow or of another form.
> Merged in pull request [#176](https://github.com/Menteion/Mente/pull/176).
> Recorded at commit `592241c`.
> The body below describes the tree before execution and is not maintained.
<!-- /spec-brief:banner -->

# Report which files the most merged pull requests change

## Intent

`mn pressure` reads `main`'s history from `git log` on its standard input and ranks the files by how many merged pull requests changed them, each with what it is among the records. A file that many changes in flight touch at once is seen, and a record-holding file it ranks high is decided in `records.rs`: made a kind, or listed with the reason it stays one file. It is a report, never a gate.

## Negative Scope

- No gate: the report fails nothing but a log of another form or an argument it does not take.
- No git of its own, so no program a repository's git configuration names.
- No change to a kind of record beyond the files its first run decides.

## Invariants

- [x] Each rule of the report and of the command has a test seen to fail without it.
- [x] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [x] `git log --first-parent --diff-merges=first-parent --name-only --format=%x00%cs%x09%s main | mn pressure` prints the ranking, with `--since` and `--top`.
- [ ] Each file the first run ranks high that holds records is a kind or is listed in `records::SHARED` with its reason.

  **Delegated** to Norman, with the manager's recommendation to defer: `docs/STATUS.md` and `ROADMAP.md` are listed in `SHARED`, since spec-graph reads each from one file; `compiler/mente-cli/src/catalogue.rs`, the error codes with their explanations, stays one file for now, since a count of the pull requests that touch a file is no count of their conflicts, and edits to different entries of a code file merge cleanly. It is revisited if the pressure report and real rebases show it conflicting.
