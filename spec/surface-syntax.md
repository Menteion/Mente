---
title: Surface Syntax, Symmetric Bindings, and Expression Control Flow
version: 0.1.0
status: active
date: 2026-11-04
---

# Surface Syntax, Symmetric Bindings, and Expression Control Flow

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`

## 1. Specification

Today `mente-syntax` parses declarations only: a function body is kept as a flat token list, so every rule below about statements and expressions is `Specified` until a body parser exists.

1. **Block delimiters and line termination**:
   - Blocks are delimited by explicit braces `{` and `}` (`Implemented`: the parser matches balanced braces).
   - Statements terminate at newlines without trailing semicolons `;`. Line continuation occurs automatically after binary operators (`+`, `-`, `*`, `/`, `|>`, `and`, `or`) or inside unclosed delimiters `(...)`, `[...]`, `{...}` (`Specified`).
2. **Symmetric 3-letter variable bindings (`Specified`; the lexer has `Val` and `Var` tokens)**:
   - `val name: Type = expr` declares an immutable binding.
   - `var name: Type = expr` declares a local mutable binding.
   - The keywords `let`, `let mut`, and `const` MUST NOT be used for local bindings. Nothing rejects them yet: `let` lexes as an identifier.
3. **Function headers and return syntax (`Implemented` in `parse_fn`)**:
   - Return types follow `->`: `pub fn clamp(x: i32, hi: i32) -> i32`.
   - Single-expression functions MAY use `=`: `pub fn square(x: i32) -> i32 = x * x`.
4. **Expression-oriented branching, `guard`, and pipe `|>` (`Specified`; the lexer has `If`, `Match`, `Guard` and `PipeGt` tokens)**:
   - `if` and `match` are expressions that return the value of their evaluated branch; Mente has no ternary `? :` operator.
   - `guard <cond> else { <diverging-expr> }` enforces preconditions and narrows types for the rest of the enclosing block.
   - Data transformation chains use the left-to-right pipe operator `x |> f(a) |> g(b)`.
5. **Logical word operators (`Specified`; the lexer has `And`, `Or` and `Not` tokens)**:
   - Boolean conjunction, disjunction, and negation use the keywords `and`, `or`, and `not`, reserving `&`, `|`, `^`, and `~` exclusively for bitwise operations and type unions (`A | B`). The lexer has a `Pipe` token for `|`; it skips `&`, `^` and `~` without an error.
6. **Numeric literals and suffixes (`Specified`)**:
   - Digits may be grouped with `_` between digits: `1_000_000`, `0xFF_FF`. In a hexadecimal literal `a`–`f` are digits, so there a suffix starts with `_` and a letter beyond `f` (`0xFF_u8`).
   - A suffix starts with `_` followed by a letter and runs to the end of the identifier. It names a unit in scope (`200.0_meter`, `9.5_sec`, `5_ms`; `dimensional-units.md`), a fixed-point format with the type name's `.` written as `_` so the literal stays one token (`0.5_q16_16` is a `q16.16`, `0.0_q1_15` a `q1.15`), or an integer type (`255_u8`, `5_u4`).
   - A suffix that names none of these is an error; `_q16` alone is not a format. A literal outside its type's range is an error (`300_u8`).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Significant whitespace indentation (Python / Mojo / Nim)** | Fragile under automated code synthesis, diff patching, and multi-line contract blocks (`requires` / `ensures`). Explicit `{ }` guarantees unambiguous structural boundaries. |
| **Asymmetric `let` / `let mut` (Rust)** | Visual alignment breaks between immutable (3 chars) and mutable (7 chars) bindings. `val` and `var` are symmetric 3-character keywords. |
| **Symbolic boolean operators `&&`, `||`, `!` (C / Rust / Go)** | Prefix `!` collides visually with postfix error propagation `expr!`, and `||` collides with zero-argument closure syntax. `and`, `or`, `not` eliminate both ambiguities. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Val," min="1" reason="Lexer defines Val token for immutable bindings" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Var," min="1" reason="Lexer defines Var token for mutable bindings" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Guard," min="1" reason="Lexer defines Guard token for early-exit precondition narrowing" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="PipeGt," min="1" reason="Lexer defines PipeGt (|>) for left-to-right data pipelines" -->
<!-- @assert-absence target="examples" symbol="let mut" glob="*.mn" reason="Mente uses symmetric val/var bindings and forbids let mut" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="0.75_q16_16" min="1" reason="Example 02 writes q16.16 literals with the full format, dot as underscore" -->
