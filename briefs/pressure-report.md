---
id: pressure-report
status: active
type: feature
awaits: [docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/src/pressure.rs, compiler/mente-cli/tests/pressure.rs, compiler/mente-cli/src/lib.rs, compiler/mente-cli/src/main.rs, compiler/mente-cli/src/records.rs, briefs/pressure-report.md, briefs/archive/pressure-report.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**]
---

# Report which files the most merged pull requests change

## Intent

`mn pressure` reads `main`'s history from `git log` on its standard input and ranks the files by how many merged pull requests changed them, each with what it is among the records. A file that many changes in flight touch at once is seen, and a record-holding file it ranks high is decided in `records.rs`: made a kind, or listed with the reason it stays one file. It is a report, never a gate.

## Negative Scope

- No gate: the report fails nothing but a log of another form or an argument it does not take.
- No git of its own, so no program a repository's git configuration names.
- No change to a kind of record beyond the files its first run decides.

## Invariants

- [ ] Each rule of the report and of the command has a test seen to fail without it.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] `git log --first-parent --diff-merges=first-parent --name-only --format=%x00%cs%x09%s main | mn pressure` prints the ranking, with `--since` and `--top`.
- [ ] Each file the first run ranks high that holds records is a kind or is listed in `records::SHARED` with its reason.
