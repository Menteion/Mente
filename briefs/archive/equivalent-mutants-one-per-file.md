---
id: equivalent-mutants-one-per-file
status: archived
type: chore
awaits: [docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/tests/mutants/**, compiler/mente-cli/src/mir_mutants.rs, compiler/mente-cli/src/records.rs, compiler/mente-cli/tests/records.rs, spec/mir-and-codegen.md, briefs/equivalent-mutants-one-per-file.md, briefs/archive/equivalent-mutants-one-per-file.md]
protectedFiles: [.github/**, CLAUDE.md]
integrity: sha256-2ed033fe596ba869a6432e50a2e176f4822e085fa5fbc1351a908a11300d340a
---

<!-- spec-brief:banner -->
> **Archived 2026-11-16.**
> Each equivalent mutant of the MIR gate is a file of its own, named by the key one function derives from its name; the gate refuses a mutant that gives no key of its own.
> Merged in pull request [#178](https://github.com/Menteion/Mente/pull/178).
> Recorded at commit `5b2c1a5`: 92 files changed, +772 −205.
> The body below describes the tree before execution and is not maintained.
<!-- /spec-brief:banner -->

# Record each equivalent mutant of the MIR gate in a file of its own

## Intent

`tests/mutants/equivalent.txt` holds every survivor of the MIR mutation gate that no input can tell from the original, one line each with its reason. Every change to MIR's lowering that leaves or kills one edits it. Each becomes a file of its own, with the mutant's exact name and its reason in the front matter. The file's key comes from the name by one function in `mir_mutants.rs`, `ge>gt` giving `ge-to-gt`, and a test requires each file's key to equal that function applied to the name in its front matter, so that no key is spelled by hand. `records.rs` declares the kind and drops the file from `SHARED`.

## Negative Scope

- No change to which mutants the gate records, or to their reasons.
- The citation stack-budget's spec-keys change writes into `equivalent.txt`'s comment is kept as it lands on `main`.

## Invariants

- [x] A survivor with no file fails the gate, and a file of a mutant some call kills fails it too, as now.
- [x] The key function is total over the gate's names and gives no two of them one key, by a test over every name the gate makes. The gate itself checks it on every run, so a new name that gives no key, or one another name gives, fails the gate where it appears.
- [x] `node tools/quick.mjs` passes before the push.
