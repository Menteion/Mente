---
id: gimli-object-with-cranelift
status: active
type: chore
awaits: [docs/design-review.md#dependency-approval]
affectedFiles: [compiler/mente-codegen/Cargo.toml, compiler/mente-codegen/src/**, Cargo.lock, .config/approved-crates.txt, compiler/mente-cli/tests/gates.rs, .github/dependabot.yml, briefs/gimli-object-with-cranelift.md, briefs/archive/gimli-object-with-cranelift.md]
protectedFiles: [CLAUDE.md, spec/**]
---

# Move gimli and object only with cranelift

## Intent

cranelift-codegen and cranelift-object use gimli and object in their own signatures (`gimli::write::Address`, `FrameDescriptionEntry`, `object::write::SymbolId`). Dependabot's pull request 164 bumped gimli and object alone, so two versions of each sat in one graph and every Rust job failed with E0308. When this round is done, they cannot diverge from cranelift's, by structure:
- mente-codegen reaches them only through `cranelift_codegen::gimli` and `cranelift_object::object`.
- A direct dependency stays only to turn on a feature cranelift leaves off. It sits under a name no code imports, at exactly cranelift's version.
- A test refuses a lock that holds either crate in any version but cranelift's.
- Dependabot moves them in one group with cranelift.

## Negative Scope

- No crate is added to `Cargo.lock`, none is upgraded, and no feature is turned off that the object and DWARF writers use.
- No change to the code generated, the object files written or their tests.
- The dependency gate's other readings stay as they are. Reading the crate graph structurally, for every crate-graph rule, is a round of its own.

## Invariants

- [x] mente-codegen names gimli and object only through cranelift's re-exports, and its manifest has no `gimli` and no `object` key.
- [x] The one direct dependency left, renamed `object-features`, turns on `coff` alone, at `=0.40.0`, the version cranelift-object takes.
- [x] `test_gimli_and_object_are_cranelifts` fails with gimli 0.34.0 planted in `Cargo.lock` beside cranelift's 0.33.0, and passes on the real lock.
- [x] The dependency policy reads a renamed dependency as the crate it names, and is seen to fail on `object-features` without that reading.
- [x] `.github/dependabot.yml` groups `cranelift-*`, `gimli` and `object` ahead of the catch-all.
- [x] `Cargo.lock` gains no crate; `.config/approved-crates.txt` names cranelift-codegen as what brings gimli and its crates.
- [x] `node tools/quick.mjs` passes before the push.
