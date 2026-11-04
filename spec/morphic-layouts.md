---
title: Morphic Data Layouts, Exact Byte Layouts, and Bit-Level Structs
version: 0.1.0
status: active
date: 2026-11-04
---

# Morphic Data Layouts, Exact Byte Layouts, and Bit-Level Structs

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`

## 1. Specification

1. **Morphic container decoupling (`AoS` / `SoA` / `Tiled`, `Implemented` in `mente-syntax` and `examples/02_mechanical_sympathy_simd.mn`)**:
   - Logical record declarations (`struct Particle { pos_x: f32, pos_y: f32, ... }`) are decoupled from physical collection layout:
     - `List<Particle>` — Array of Structures (`AoS`) for single-item random access.
     - `SoA<Particle>` — Structure of Arrays (`[pos_x...], [pos_y...]`) for full-column SIMD streaming.
     - `Tiled<Particle, lanes: 16>` — Array of Structures of Arrays (`AoSoA`), grouping 16 elements per tile so each field slice fits a 64-byte L1 cache line / AVX-512 `ZMM` register.
2. **Two-tier struct byte layout (`Implemented` in `compiler/mente-sema/src/lib.rs`)**:
   - **Plain `struct` (default)**: The compiler MAY reorder fields by descending alignment to eliminate implicit padding bytes.
   - **Verified `@layout(size: S, align: A)` + `@bytes(start..end)`**: Locks field order and statically verifies in `verify_struct_layout` that:
     1. Every field carries an explicit `@bytes(start..end)` attribute whose width equals `type_byte_size(field.ty)`.
     2. Adjacent fields have zero unannotated padding gaps (`start == cursor`) and zero overlaps.
     3. The final field ends at `S` (`cursor == spec.size`).
3. **Bit-exact hardware and protocol structures (`bitstruct`, `Implemented` in `compiler/mente-sema/src/lib.rs`)**:
   - `bitstruct FrameFlags : u16` packs bitfields into an explicit backing integer (`u8`..`u128`) and verifies in `verify_bitstruct` that `@bits(low..=high)` ranges partition `0..N` contiguously without overlap or bit overflow.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Manual SoA struct rewriting (C++ / Rust)** | Converting `Vec<Particle>` to `ParticleSoA { x: Vec<f32>, y: Vec<f32>, ... }` in C++ or Rust requires rewriting every data structure definition and every call site (`p[i].x` -> `p.x[i]`). Morphic containers preserve `p[i].x` syntax across `List<T>`, `SoA<T>`, and `Tiled<T, lanes>`. |
| **Silent compiler struct padding (`#[repr(C)]` in Rust / C structs)** | In C and Rust `#[repr(C)]`, misordering a `u8` and `u64` silently inserts 7 invisible padding bytes, wasting cache bandwidth and causing uninitialized memory leaks across FFI/network boundaries. `@layout` + `@bytes` makes every byte explicit and machine-checked. |
| **C bitfields (`unsigned int flag : 3;`)** | C bitfield bit ordering, endianness, and straddling behavior are implementation-defined by the C standard and non-portable across compilers. `bitstruct` with explicit `@bits(low..=high)` is 100% deterministic. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="pub fn verify_explicit_struct_layout" min="1" reason="Sema verifies exact @layout and @bytes contiguity and size invariants" -->
<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="pub fn verify_bitstruct_layout" min="1" reason="Sema verifies exact bitstruct @bits partitioning" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@layout(size: 64, align: 64)" min="1" reason="Example 02 enforces a 64-byte L1 cache-line struct layout" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="Tiled<Particle, lanes: 16>" min="1" reason="Example 02 uses AoSoA Tiled container layout" -->
