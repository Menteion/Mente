---
id: reference-type-idiom
status: deferred
trigger: when the language pause ends
type: feature
affectedFiles: [compiler/mente-syntax/src/**, compiler/mente-sema/src/lib.rs, compiler/mente-cli/src/fuzz.rs, spec/surface-syntax.md, spec/diagnostics.md, CLAUDE.md, briefs/reference-type-idiom.md, briefs/archive/reference-type-idiom.md]
protectedFiles: [.github/**]
---

# Name the Mente spelling where a type is written as a reference

## Intent

A reference passed in a call, `f(&mut v)`, is refused under its own code, `E0119`, with a message that names Mente's spelling, `f(mut v)` (`surface-syntax.md`, the item on other languages' idioms). A reference written where a type stands, `x: &u8`, `x: &mut u8`, `-> &u8`, a field `a: &u8`, `val y: &u8` or an element `[2]&u8`, is refused only with the generic `E0116`, "expected a type, found `&`", which names no Mente spelling; `test_references_in_types_are_refused` in `mente-sema` pins that error, and CLAUDE.md's invariant 2 cites it. When this round is done, each of those is refused as an idiom: a code of its own, or `E0119` widened, and a message that names the parameter mode Mente writes instead, read-only by default, `mut`, `own` or `in`, as `fn step(mut p: Particle)` does. Where one rewrite is plain, as `x: &u8` to `x: u8` in a parameter, the parser reads the Mente spelling and checks the rest. The work is compiler work, so it waits for the pause on new language work to end, when Norman's batch of ideas has been weighed.

## Negative Scope

- No reference type enters the language: `&T` stays refused wherever it stands.
- No change to the call-site idiom `E0119` beyond widening it, if that is the choice.
- No rewrite is applied where the mode is not plain, such as a reference in a field or a return type.

## Invariants

- [ ] The idiom item of `surface-syntax.md` names the new case and its code before the parser changes.
- [ ] The program generator's idiom rules (`fuzz.rs`) reach a reference in each type position, as `quality-per-change` asks of a new rule.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] Each type position `test_references_in_types_are_refused` lists gives the idiom diagnostic, by a test seen to fail without the change.
- [ ] CLAUDE.md's invariant 2 cites that test for references in types.
