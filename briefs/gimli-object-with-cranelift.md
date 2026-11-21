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
- mente-codegen depends on neither directly: cranelift turns on every feature the object and DWARF writers use, object's `write` bringing `coff` and `pe`.
- The dependency policy reads `Cargo.lock`, cargo's own reading of the manifests, so that no spelling of a manifest, a rename in either quote or a `[dependencies.<name>]` table, gets a crate past it.
- A test refuses a lock that holds either crate in any version but cranelift's.
- Dependabot moves them in one group with cranelift.

## Negative Scope

- No crate is added to `Cargo.lock`, none is upgraded, and no feature is turned off that the object and DWARF writers use.
- No change to the code generated, the object files written or their tests.
- The gates' other readings of the crate graph stay as they are.

## Invariants

- [x] mente-codegen names gimli and object only through cranelift's re-exports, and its manifest has no `gimli` and no `object` key.
- [x] mente-codegen depends on neither gimli nor object, and `cargo tree -e features -i object` shows the same features of object on each of the three targets as with the direct dependency it had.
- [x] `test_gimli_and_object_are_cranelifts` fails with gimli 0.34.0 planted in `Cargo.lock` beside cranelift's 0.33.0, and passes on the real lock.
- [x] The dependency policy reads each crate's dependencies from `Cargo.lock`, by the crate's own name. Planted in a manifest, a crate outside the policy is refused in each of six forms: renamed in a table or inline with single quotes, a `[dependencies.<name>]` table, under a target, a dev-dependency, and renamed under an allowed key. The text reader before let two of them through.
- [x] `.github/dependabot.yml` groups `cranelift-*`, `gimli` and `object` ahead of the catch-all.
- [x] `Cargo.lock` gains no crate; `.config/approved-crates.txt` names cranelift-codegen as what brings gimli and its crates, and cranelift-object as what brings object and its.
- [x] `node tools/quick.mjs` passes before the push.
