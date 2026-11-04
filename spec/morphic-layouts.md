---
title: Morphic Data Layouts, Exact Byte Layouts, and Bit-Level Structs
version: 0.1.0
status: active
date: 2026-11-04
---

# Morphic Data Layouts, Exact Byte Layouts, and Bit-Level Structs

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`

## 1. Specification

1. **Morphic container decoupling (`AoS` / `SoA` / `Tiled`, `Specified`)**:
   - Logical record declarations (`struct Particle { pos_x: f32, pos_y: f32, ... }`) are decoupled from physical collection layout:
     - `List<Particle>` — Array of Structures (`AoS`) for single-item random access.
     - `SoA<Particle>` — Structure of Arrays (`[pos_x...], [pos_y...]`) for full-column SIMD streaming.
     - `Tiled<Particle, lanes: 16>` — Array of Structures of Arrays (`AoSoA`), grouping 16 elements per tile. A tile's slice of one field is `16 × sizeof(field)` bytes: one 64-byte cache line / AVX-512 `ZMM` register for a 4-byte field, two lines for an 8-byte field.
   - A `mut` element projection of a `SoA` or `Tiled` container (`step(mut particles[i])`) follows copy-in/copy-out (`mutable-value-semantics.md`, item 4): the element is gathered into a temporary and scattered back.
   - `Implemented`: `MorphicLayout::byte_offset` in `mente-mir` computes an element's field offset under each of the three layouts (tested). No container type exists yet.
2. **Two-tier struct byte layout**:
   - **Plain `struct` (default, `Specified`)**: The compiler MAY reorder fields by descending alignment to eliminate implicit padding bytes.
   - **Verified `@layout(size: S, align: A)` + `@bytes(start..end)` (`Implemented` in `verify_explicit_struct_layout`, `compiler/mente-sema/src/lib.rs`)**: Locks field order and statically verifies that:
     1. `A` is a power of two (`E0410`) and `S` is a multiple of `A` (`E0411`).
     2. Every field carries an explicit `@bytes(start..end)` attribute (`E0412`) whose width equals the field type's byte size (`E0415`). A field type of unknown size is rejected (`E0417`): sizes are known for fixed-width primitives, `uN` / `iN` whose `N` is a multiple of 8 (`u24` is exactly 3 bytes), `q16.16` / `q1.15` / `q0.8`, `sat<T>` / `wrap<T>`, fixed arrays `[N]T`, and the module's own `bitstruct`s and `@layout` structs. Any other `uN` width belongs in a `bitstruct`. `usize` and `isize` have no size here, because a `@layout` must mean the same bytes on every target.
     3. Adjacent fields have zero unannotated padding gaps and zero overlaps (`start == cursor`, `E0413`), and no range is empty (`E0414`).
     4. The final field ends at `S` (`cursor == S`, `E0416`).
     5. Unless the struct is marked `@packed`, each field starts at a multiple of its type's natural alignment (`E0418`), and no field needs more alignment than the struct's `A` (`E0419`). Natural alignment is the declared `align` of a `@layout` struct, a bitstruct's backing size, an array's element alignment, and otherwise the largest power of two dividing the type's size, at most 16 (`u64` is 8, `u24` is 1). `@packed` lifts both rules for wire formats; the compiler then reads and writes such fields with unaligned accesses.
   - **Byte order (`Specified`)**: the bytes of a `@layout` struct are little-endian, so the same struct has the same bytes on every target; `@endian(big)` makes them big-endian. On a target of the other byte order, loads and stores of the struct's fields swap bytes. Plain structs use the target's order.
3. **Bit-exact hardware and protocol structures (`bitstruct`, `Implemented` in `verify_bitstruct_layout`, `compiler/mente-sema/src/lib.rs`)**:
   - `bitstruct FrameFlags : u16` packs bitfields into an explicit backing integer (`u8`..`u128`, else `E0420`) and verifies that the half-open `@bits(start..end)` ranges partition `0..N` contiguously without gap or overlap (`E0421`), empty range (`E0422`) or shortfall (`E0423`).
   - Each field's type must be exactly as wide as its range: `bool` is 1 bit, `uN` / `iN` are `N` bits, and a bitstruct of the same module is its backing width (`E0424` on a mismatch, `E0425` for any other type).
4. **Arbitrary-width integers (`Specified`; their widths are `Implemented` in the two provers above)**:
   - `uN` and `iN`, for `1 <= N <= 128`, are unsigned and two's-complement integers of exactly `N` bits' range: `u4` holds `0..=15`, `i4` holds `-8..=7`. `u8`..`u128` and `i8`..`i128` are the cases where `N` is a power of two.
   - Inside a `bitstruct` a `uN` field occupies exactly `N` bits, and inside a `@layout` struct exactly `N / 8` bytes when `N` is a multiple of 8. Anywhere else the compiler stores it in the smallest of 1, 2, 4, 8 or 16 bytes that holds it.
   - Arithmetic on `uN` follows `refinement-and-fixed-point.md`: a result outside `N` bits traps unless proven in range.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Manual SoA struct rewriting (C++ / Rust)** | Converting `Vec<Particle>` to `ParticleSoA { x: Vec<f32>, y: Vec<f32>, ... }` in C++ or Rust requires rewriting every data structure definition and every call site (`p[i].x` -> `p.x[i]`). Morphic containers preserve `p[i].x` syntax across `List<T>`, `SoA<T>`, and `Tiled<T, lanes>`. |
| **Silent compiler struct padding (`#[repr(C)]` in Rust / C structs)** | In C and Rust `#[repr(C)]`, misordering a `u8` and `u64` silently inserts 7 invisible padding bytes, wasting cache bandwidth and causing uninitialized memory leaks across FFI/network boundaries. `@layout` + `@bytes` makes every byte explicit and machine-checked. |
| **Byte offsets without alignment rules, or the target's byte order** | A `u64` at byte 4 passes a pure offset check and then faults or splits a cache line on targets that need aligned loads, and a struct written on a little-endian host reads back differently on a big-endian one, so "the same bytes on every target" would hold for neither. `@packed` and `@endian(big)` make each exception explicit. |
| **C bitfields (`unsigned int flag : 3;`)** | C bitfield bit ordering, endianness, and straddling behavior are implementation-defined by the C standard and non-portable across compilers. `bitstruct` with explicit `@bits(start..end)` is deterministic. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="pub fn verify_explicit_struct_layout" min="1" reason="Sema verifies exact @layout and @bytes contiguity and size invariants" -->
<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="pub fn verify_bitstruct_layout" min="1" reason="Sema verifies exact bitstruct @bits partitioning" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@layout(size: 64, align: 64)" min="1" reason="Example 02 enforces a 64-byte L1 cache-line struct layout" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="Tiled<Particle, lanes: 16>" min="1" reason="Example 02 uses AoSoA Tiled container layout" -->
<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_rejects_layout_field_of_unknown_size" min="1" reason="A @layout field whose size is unknown, usize included, is rejected rather than skipped" -->
<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_layout_fields_sit_at_aligned_offsets" min="1" reason="A @layout field must start at its natural alignment unless the struct is @packed" -->
