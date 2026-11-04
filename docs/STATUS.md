# Mente — Implementation Status (`docs/STATUS.md`)

**Single source of live implementation status across `compiler/`, `examples/`, and `spec/`.**

| Field | Value |
| :--- | :--- |
| Current milestone | Phase 1 Foundation Complete (`v0.1.0-alpha`); **Phase 0 (`@spec-family` Governance Setup) queued as immediate Step 0** |
| Specification files | 19 flat semantic files in [`spec/`](../spec/README.md) (`README.md`, `philosophy.md`, and 17 single-decision specs) |
| Executable assertions | 61 `<!-- @assert-* -->` directives across `spec/*.md` (100% passing & counterfactual-proven via `mn spec`) |
| Compiler crates | 6 Rust crates (`mente-syntax`, `mente-ast`, `mente-sema`, `mente-mir`, `mente-codegen`, `mente-cli`) |
| Showcase `.mn` modules | 4 verified programs in [`examples/`](../examples/README.md) |

---

## Immediate Next Step — Phase 0 (Step 0): Full `@spec-family` Governance Configuration

Before expanding Phase 1 AST-to-`MIR` lowering, execute **Step 0** to wire the external `@spec-family` toolchain alongside `mn spec`:

1. **`package.json` devDependencies**:
   - `@spec-family/spec-guard`: `^0.30.0` (`spec-guard`, `spec-guard prove`)
   - `@spec-family/spec-graph`: `^0.21.0` (`spec-graph`, `spec-graph verify`)
   - `@spec-family/spec-brief`: `^0.12.0` (`spec-brief generate`, `spec-brief verify --strict`)
2. **`.spec-graph.json`**:
   - Point `adrDir` / `specDir` at `spec/` and `docs/` with `requireFrontMatter: true` and `strictLinks: true`.
3. **`.spec-brief.json` & `briefs/*.md`**:
   - Map each crate in `compiler/` (`mente-syntax`, `mente-ast`, `mente-sema`, `mente-mir`, `mente-codegen`, `mente-cli`) to its governing atomic `spec/*.md` files so context briefs are auto-generated per crate.

---

## Crate-by-Crate Maturity Ledger

| Crate | Path | Implemented today | Specified (next milestone) |
| :--- | :--- | :--- | :--- |
| **`mente-syntax`** | [`compiler/mente-syntax/src/lib.rs`](../compiler/mente-syntax/src/lib.rs) | Full `.mn` lexer & structural parser: `module`, `unit`, `struct`, `bitstruct`, `enum`, `trait`, `fn`, `val`/`var`, `mut`/`own`/`in`, `requires`/`ensures`/`invariant`/`decreases`, `with`, `parallel`, `parallel_for`, `comptime`, `@example`. | Full Pratt expression AST nodes for nested binary/pipe expressions inside function bodies. |
| **`mente-ast`** | [`compiler/mente-ast/src/lib.rs`](../compiler/mente-ast/src/lib.rs) | 128-bit FNV-1a/SplitMix dual semantic hashing (`spec_hash` vs. `body_hash`) and `< 0.1 ms` Holographic Spec Blueprint projection (`project_spec_blueprint`). | Incremental module dependency DAG caching keyed on `spec_hash`. |
| **`mente-sema`** | [`compiler/mente-sema/src/lib.rs`](../compiler/mente-sema/src/lib.rs) | Exact byte-range `@layout(size, align)` + `@bytes` prover (`E0410`–`E0416`), `bitstruct` `@bits` prover (`E0420`–`E0423`), `@total`/`decreases` loop checker (`E0501`), and `@branchless` control-flow verifier (`E0401`–`E0402`). | Abelian dimensional `unit` solver, MVS interprocedural provenance checker, and Tier-1 difference-bound matrix (`x - y <= c`) solver. |
| **`mente-mir`** | [`compiler/mente-mir/src/lib.rs`](../compiler/mente-mir/src/lib.rs) | 64-byte aligned `MirRegionSoA` (`NodeId(u32)`), `MorphicLayout` (`AoS`/`SoA`/`Tiled`) byte-offset calculator, `ValueLattice` bounds-check elimination (`proves_in_bounds`), `@branchless` opcode verifier, in-memory evaluator (`eval_in_memory`), and microsecond mutation prover (`run_in_memory_mutation_gate`). | Full AST-to-RVSDG lowering pass and equality saturation (`E-Graph`) rewrite loop. |
| **`mente-codegen`** | [`compiler/mente-codegen/src/lib.rs`](../compiler/mente-codegen/src/lib.rs) | Carry-Flag (`CF`) zero-overhead error ABI descriptor (`CarryFlagAbiLowering` for x86-64 `RAX`/`RDX`/`CF` and AArch64 `X0`/`X1`/`C`) and direct binary SPIR-V 1.6 emitter (`emit_spirv_binary`). | Direct ELF/PE/Mach-O x86-64/AArch64 machine-code emission and `comptime` bytecode VM. |
| **`mente-cli`** | [`compiler/mente-cli/src/main.rs`](../compiler/mente-cli/src/main.rs) | `mn view --spec`, `mn check`, `mn test --mutate`, and native `mn spec` (`guard` + counterfactual `prove` + `graph`). | `mn build`, `mn fmt`, and `mn lsp`. |
