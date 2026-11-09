# CLAUDE.md — Project Switchboard

This file is the switchboard, not the rulebook.

1. **Live implementation status**: Read [`docs/STATUS.md`](docs/STATUS.md) first. It is the single source of truth for what is `Implemented` vs. `Specified` across all 6 compiler crates and names the immediate next step. Open design problems, and what is proposed for each, are in [`docs/design-review.md`](docs/design-review.md); a design change starts from its entry there.
2. **Language and compiler specification**: Read [`spec/README.md`](spec/README.md). Each file in `spec/*.md` defines exactly **one orthogonal mechanism and design decision** (Specification + Decision Rationale & Rejected Alternatives + Executable `<!-- @assert-* -->` assertions). Load only the specific `spec/<topic>.md` file governing the crate you are editing.
3. **Engineering roadmap**: Read [`ROADMAP.md`](ROADMAP.md) for the four stages of the work and stage 1's milestones, M0a, M0b and M1 to M9. [`docs/design-brief.md`](docs/design-brief.md) holds the design, the working rules and the long-term plan in full.
4. **Agent benchmark**: [`benchmark/README.md`](benchmark/README.md) defines the corpus. Its held-out set lives in the private repository `Menteion/mente-bench-heldout`, cloned beside this one as `..\mente-bench-heldout`. Never read it, or held-out results task by task, while working on the language card or on anything a benchmark agent is shown; only `npm run check:benchmark -- --held-out` opens it.

---

## Non-Negotiable Architectural Invariants

1. **Single-decision atomic specs (`spec/*.md`)**:
   - Never re-introduce numeric filename prefixes (`00_`..`08_`), nested subdirectories inside `spec/`, `DEC-XXX` numbers, or decorative emoji warning headers.
   - A claim is a list item that starts with its one maturity label written as code: `Implemented`, `Specified`, `Target` or `Hypothesis`. A label inside a sentence or in parentheses is not a claim, and `spec-guard claims` does not read it; write it as a claim. One claim carries one label.
   - `Implemented` means the compiler does it today. Its evidence is an `<!-- @assert-count -->` indented under the item, whose `symbol` starts with `fn test_` and names a unit test that fails when the behaviour breaks. A directive that names a function or a type shows only that the name exists, and a `.mn` example using a syntax never makes it `Implemented`.
   - `Specified` and `Hypothesis` need no evidence. A `Target` needs its measurement protocol under "Measurement protocols" in `ROADMAP.md`.
   - A claim another document cites carries `<!-- @spec-item id="..." -->` on the line above it.
   - `<!-- @assert-present -->`, `<!-- @assert-count -->` and `<!-- @assert-absence -->` directives still pin the structural facts a spec states, such as files that exist and spellings the examples must or must not use.
   - Do not write counts of files, directives or tests into documents; `mn spec` and `cargo test` print them.
2. **Forbidden language anti-patterns in `.mn` code**:
   - Never write `let` or `let mut` (use `val` and `var`).
   - Never write `&T`, `&mut T`, or `<'a>` lifetimes (use MVS parameter modes: default read-only, `mut`, `own`, `in`).
   - Never write `async` or `await` (use `with IO, Net` effects and `parallel (...)`).
   - Never write `.unwrap()` or `::<>` turbofish.
   - Never link upstream C++ `LLVM/MLIR` into `compiler/mente-mir` (keep `MirRegionSoA` self-contained pure Rust, indexed by `NodeId(u32)`). `mente-mir` has no dependencies; native code goes through Cranelift, a pure-Rust crate that only `mente-codegen` may depend on, and so do `gimli` and `object`, which write its object files and debug information (Norman approved them on 2026-11-10). The reference interpreter `mente-interp` never depends on `mente-mir` or `mente-codegen`, so that it shares no code with the paths it checks. Every other crate depends only on the workspace's own crates: a new external dependency needs Norman's approval, and `test_crates_depend_only_on_allowed_crates` holds the line. An approval names the direct dependencies and covers the crates they bring in; the pull request that adds or upgrades one lists every crate `Cargo.lock` gains ([`dependency-approval`](docs/design-review.md#dependency-approval)).
3. **English-only repository text**:
   - Code, comments, documentation, examples, commit messages and pull request text are written in English, even when the conversation with Norman is in Chinese.
   - `test_repository_text_is_english` in `compiler/mente-cli` fails on any CJK character in a file of the working tree (`.git`, `target` and `node_modules` aside). Commit messages and pull request text are outside its reach and follow this rule by hand. A future fixture that must hold non-English text (a lexer test of Unicode string literals, say) goes in a directory the test is changed to skip, named in the test.
4. **The design register and the documentation tools**:
   - Cite an entry of [`docs/design-review.md`](docs/design-review.md) as a link whose anchor is its key, such as [`interface-from-signature`](docs/design-review.md#interface-from-signature); `spec-graph` reports a key that is misspelled or renamed. A `done` entry links the documents it changed and ends with `(commit "subject")`.
   - A task brief's front matter may name `raises: [spec/<file>.md#<claim id>]` and `awaits: [docs/design-review.md#<key>]`. `spec-brief lint` rejects a claim that does not exist or an awaited entry that is not yet `decided` or `done`, and `spec-brief archive` refuses a raised claim that is not yet `Implemented`.
   - Run the `@spec-family` tools only through the npm scripts, or with `npx --no-install` and the full `@spec-family/` name. Never patch `node_modules` or the tools' own repository (`C:\Repos\specs`); write down a false positive or a missing feature and report it to Norman.
5. **Four Mandatory Engineering Guardrails**:
   - **Guardrail #1 (Solver Predictability — [`ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md))**: Tier-1 stays strictly within polynomial-time Interval + Octagon/DBM ($\pm x \pm y \le c$) domains and, whenever an obligation fails, MUST emit a concrete valuation Tier 1 cannot rule out (never called an input that fails) plus a copy-pasteable 1-line `guard` prescription as a suggested fix.
   - **Guardrail #2 (Dual-Track Codegen — [`mir-and-codegen.md`](spec/mir-and-codegen.md))**: Complete all high-level semantic optimizations (bounds erasure, Ghost Contract `Assume`, `AoSoA` offsets, `@branchless` select) in `MirRegionSoA` before lowering to Cranelift or any later backend.
   - **Guardrail #3 (Provenance Escape Diagnostics — [`regions-and-pools.md`](spec/regions-and-pools.md))**: Never expose internal region IDs in errors; `E0310` MUST prescribe either `in arena: Arena` or `Pool<T>` / `Handle<T>`.
   - **Guardrail #4 (Ecosystem Absorption — [`comptime-and-ffi.md`](spec/comptime-and-ffi.md))**: `mn bind` auto-synthesizes quarantined `extern "C"` + `with FFI` + `@layout` bindings from C headers, and from Rust libraries through their `cbindgen` headers (stage 4).

---

## Verification Commands (Must Pass Before Every Commit)

```powershell
# 1. Run all unit & integration tests across the 7 compiler crates (including spec assertions)
cargo test

# 2. Check spec/*.md front matter and run every @assert-* directive (rejects directives that cannot fail)
cargo run --bin mn -- spec

# 3. Check that each example reports exactly what its .expected file lists, then run the demo MIR region and its mutation gate
cargo run --bin mn -- test --mutate

# 4. Inspect Holographic Spec Blueprint projection (strips bodies, shows spec_hash/body_hash)
cargo run --bin mn -- view --spec examples/02_mechanical_sympathy_simd.mn

# 5. Run the pinned @spec-family checks: directives, prove, claims, the spec graph and briefs (`npm ci` once first)
npm run verify:docs

# 7. Check the agent-benchmark corpus against its references and its lock, and test the checker
npm run check:benchmark
npm run test:benchmark

# 6. Check the format and the lints (rustfmt, then clippy with warnings as errors)
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
```

---

## Crate-to-Spec Routing Table

| Crate | Governing `spec/*.md` files |
| :--- | :--- |
| `compiler/mente-syntax` | [`holographic-module.md`](spec/holographic-module.md), [`surface-syntax.md`](spec/surface-syntax.md), [`grammar.md`](spec/grammar.md), [`core-calculus.md`](spec/core-calculus.md), [`diagnostics.md`](spec/diagnostics.md) |
| `compiler/mente-ast` | [`holographic-module.md`](spec/holographic-module.md) |
| `compiler/mente-sema` | [`core-calculus.md`](spec/core-calculus.md), [`refinement-and-fixed-point.md`](spec/refinement-and-fixed-point.md), [`dimensional-units.md`](spec/dimensional-units.md), [`algebraic-types-and-null.md`](spec/algebraic-types-and-null.md), [`traits-and-generics.md`](spec/traits-and-generics.md), [`mutable-value-semantics.md`](spec/mutable-value-semantics.md), [`regions-and-pools.md`](spec/regions-and-pools.md), [`morphic-layouts.md`](spec/morphic-layouts.md), [`ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md), [`capability-effects.md`](spec/capability-effects.md), [`diagnostics.md`](spec/diagnostics.md) |
| `compiler/mente-mir` | [`core-calculus.md`](spec/core-calculus.md), [`mir-and-codegen.md`](spec/mir-and-codegen.md), [`hardware-contracts.md`](spec/hardware-contracts.md), [`disjoint-parallelism.md`](spec/disjoint-parallelism.md) |
| `compiler/mente-codegen` | [`error-handling-abi.md`](spec/error-handling-abi.md), [`hardware-contracts.md`](spec/hardware-contracts.md), [`mir-and-codegen.md`](spec/mir-and-codegen.md) |
| `compiler/mente-interp` | [`core-calculus.md`](spec/core-calculus.md) |
| `compiler/mente-cli` | [`comptime-and-ffi.md`](spec/comptime-and-ffi.md), [`holographic-module.md`](spec/holographic-module.md), [`diagnostics.md`](spec/diagnostics.md) |
