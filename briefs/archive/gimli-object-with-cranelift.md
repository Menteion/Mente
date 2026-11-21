---
id: gimli-object-with-cranelift
status: archived
type: chore
awaits: [docs/design-review.md#dependency-approval]
affectedFiles: [compiler/mente-codegen/Cargo.toml, compiler/mente-codegen/src/**, Cargo.lock, .config/approved-crates.txt, compiler/mente-cli/tests/gates.rs, .github/dependabot.yml, briefs/gimli-object-with-cranelift.md, briefs/archive/gimli-object-with-cranelift.md]
protectedFiles: [CLAUDE.md, spec/**]
integrity: sha256-1ff4dbdca86a818dc0c96cb14cbf6d61b200a68ad4c118d8a89f59cea6d4c351
---

<!-- spec-brief:banner -->
> **Archived 2026-11-21.**
> gimli and object come only through cranelift, with no direct dependency on either; a test refuses a lock that holds either in a version not cranelift's, and the dependency policy reads the lock and the workspace's members.
> Merged in pull request [#180](https://github.com/Menteion/Mente/pull/180).
> Recorded at commit `055f030`: 10 files changed, +527 −110.
> The body below describes the tree before execution and is not maintained.
<!-- /spec-brief:banner -->

# Move gimli and object only with cranelift

## Intent

cranelift-codegen and cranelift-object use gimli and object in their own signatures (`gimli::write::Address`, `FrameDescriptionEntry`, `object::write::SymbolId`). Dependabot's pull request 164 bumped gimli and object alone, so two versions of each sat in one graph and every Rust job failed with E0308. When this round is done, they cannot diverge from cranelift's, by structure:
- mente-codegen reaches them only through `cranelift_codegen::gimli` and `cranelift_object::object`.
- mente-codegen depends on neither directly: cranelift turns on every feature the object and DWARF writers use, object's `write` bringing `coff` and `pe`.
- The dependency policy reads `Cargo.lock`, cargo's own reading of the manifests, so that no spelling of a manifest, a rename in either quote or a `[dependencies.<name>]` table, gets a crate past it, and a crate with no source counts as the workspace's only if the root manifest names it a member.
- A test refuses a lock that holds either crate in any version but cranelift's.
- Dependabot moves them in one group with cranelift.

## Negative Scope

- No crate is added to `Cargo.lock`, none is upgraded, and no feature is turned off that the object and DWARF writers use.
- No change to the code generated, the object files written or their tests.
- The gates' other readings of the crate graph stay as they are.

## Invariants

- [x] mente-codegen names gimli and object only through cranelift's re-exports, and its manifest has no `gimli` and no `object` key.
- [x] mente-codegen depends on neither gimli nor object, and `cargo tree -e features -i object` shows the same features of object on each target as with the direct dependency it had.
- [x] `test_gimli_and_object_are_cranelifts` fails with gimli 0.34.0 planted in `Cargo.lock` beside cranelift's 0.33.0, and passes on the real lock.
- [x] The dependency policy reads each crate's dependencies from `Cargo.lock`, by the crate's own name. Planted in a manifest, a crate outside the policy is refused in every form: renamed in a table or inline with single quotes, a `[dependencies.<name>]` table, under a target, a dev-dependency, and renamed under an allowed key. The text reader before let the table forms through.
- [x] A crate with no source is the workspace's only if the root manifest names it a member. A crate at a path outside the workspace, planted as a dependency of mente-codegen, is refused by both the policy and the approval check, by its name, where both let it through before.
- [x] `.github/dependabot.yml` groups `cranelift-*`, `gimli` and `object` ahead of the catch-all.
- [x] `Cargo.lock` gains no crate; `.config/approved-crates.txt` names cranelift-codegen as what brings gimli and its crates, and cranelift-object as what brings object and its.
- [x] `node tools/quick.mjs` passes before the push.
